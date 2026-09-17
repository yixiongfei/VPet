//! VPet Core（Rust 侧）。
//!
//! 目前这里负责：宠物窗口的摆位与托盘、鼠标穿透的 alpha 命中判定、全局快捷键，
//! 以及 `core/state_machine.rs` 的状态机和驱动它的心跳——数值随时间走、饿了自己去吃。
//! 调度 / 工具 / 权限 / 存储还没做，见 docs/03-architecture.md §3 与 docs/07 Phase 2。

mod core;

use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use core::state_machine::{reduce, Activity, Event, PetState, Touch};

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow,
};
use tauri_plugin_global_shortcut::Shortcut;

const PET_WINDOW: &str = "pet";

/// 呼出输入框的全局快捷键（docs/05 §4 的默认值，Q 待确认）
const PROMPT_SHORTCUT: &str = "Alt+V";

/// 命中掩码的边长，必须与前端 body/hitMask.ts 的 N 一致
const MASK_N: usize = 48;
/// 掩码坐标系的边长（pet.json 的 500×500 逻辑参考系）
const PET_LOGICAL_SIZE: f64 = 500.0;
/// 光标轮询间隔（docs/05 §4）
const POLL: Duration = Duration::from_millis(50);

/// 状态推进的节拍。一秒一次，数值按 1/60 分钟走——比每分钟一跳平滑，
/// 也让吃喝这种几秒钟的过场能踩准点
const TICK: Duration = Duration::from_secs(1);
/// 吃 / 喝 演多久。和夹心动画的长度（约 2.6 s）对齐
const CONSUME: Duration = Duration::from_millis(2600);
/// 活动和心情都没变时，最多隔这么久也要把数值同步给 Body 一次
const SYNC_EVERY: Duration = Duration::from_secs(30);

/// 鼠标穿透的判定状态。
///
/// 一条硬规则：**任何拿不准的情况都退到「不穿透」**。穿透错了，宠物就再也点不着，
/// 只能从托盘退出；不穿透错了，无非是挡住了下面窗口的一次点击。两种代价不对称。
#[derive(Default)]
struct HitState {
    /// 48×48 按位打包的不透明格子。None = 还没收到第一帧，当作压在宠物身上
    mask: Option<Vec<u8>>,
    /// 交互进行中（按下 / 提起），钉住不穿透
    pinned: bool,
    /// 用户从托盘关掉了穿透
    disabled: bool,
    /// 当前是否已穿透，只在变化时才真去调 set_ignore_cursor_events
    ignoring: bool,
}

impl HitState {
    /// 逻辑坐标是否压在宠物身上
    fn opaque_at(&self, x: f64, y: f64) -> bool {
        let Some(mask) = &self.mask else {
            return true; // 还没有掩码：宁可不穿透
        };
        let n = MASK_N as f64;
        let (col, row) = ((x / PET_LOGICAL_SIZE * n) as isize, (y / PET_LOGICAL_SIZE * n) as isize);
        if !(0..MASK_N as isize).contains(&col) || !(0..MASK_N as isize).contains(&row) {
            return false; // 光标不在窗口范围里
        }
        let i = row as usize * MASK_N + col as usize;
        mask.get(i >> 3).is_some_and(|b| b & (1 << (i & 7)) != 0)
    }
}

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Body 启动时拉一次当前状态，不用干等下一次 `pet:state`
#[tauri::command]
fn get_pet_state(pet: State<'_, Mutex<PetState>>) -> PetState {
    pet.lock().map(|s| *s).unwrap_or_default()
}

/// Body 报告宠物被摸了。数值怎么变是状态机的事，Body 不算数
#[tauri::command]
fn pet_touched(app: AppHandle, zone: String) {
    let touch = match zone.as_str() {
        "head" => Touch::Head,
        "body" => Touch::Body,
        "raise" => Touch::Raise,
        other => {
            log::warn!("未知的触摸区域 {other}");
            return;
        }
    };
    apply(&app, &Event::Touched(touch));
}

/// 调试：直接改数值，用来验证阈值行为（把 hunger 调低看它去不去吃）
#[tauri::command]
fn debug_patch_pet_state(
    app: AppHandle,
    strength: Option<f32>,
    feeling: Option<f32>,
    hunger: Option<f32>,
    thirst: Option<f32>,
) {
    apply(
        &app,
        &Event::Patch {
            strength,
            feeling,
            hunger,
            thirst,
        },
    );
}

/// 把事件喂给状态机，落到共享状态，然后广播给 Body
fn apply(app: &AppHandle, event: &Event) {
    let pet = app.state::<Mutex<PetState>>();
    let Ok(mut st) = pet.lock() else { return };
    let mut next = reduce(&st, event);
    next.updated_at = now_ms();
    *st = next;
    drop(st);
    let _ = app.emit("pet:state", next);
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 宠物的心跳：推进数值、到点触发吃喝、变化时同步给 Body。
///
/// 这是「有身体」的关键——不需要用户说话，它自己就会饿、会去吃、会累。
fn spawn_pet_clock(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_sync = Instant::now();
        // 吃 / 喝 演了多久。放在这里而不是 PetState 里，是为了让 reduce 保持纯函数
        let mut consuming_since: Option<Instant> = None;

        loop {
            std::thread::sleep(TICK);
            let pet = app.state::<Mutex<PetState>>();
            let Ok(mut st) = pet.lock() else { continue };
            let before = *st;

            let mut next = reduce(
                &before,
                &Event::Tick {
                    minutes: TICK.as_secs_f32() / 60.0,
                },
            );

            // 吃 / 喝 是过场：演够 CONSUME 就把饱腹/水补上并回空闲
            if matches!(next.activity, Activity::Eating | Activity::Drinking) {
                let since = consuming_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= CONSUME {
                    next = reduce(&next, &Event::Consumed);
                    consuming_since = None;
                }
            } else {
                consuming_since = None;
            }

            next.updated_at = now_ms();
            *st = next;
            drop(st);

            let changed = next.activity != before.activity || next.mood != before.mood;
            if changed || last_sync.elapsed() >= SYNC_EVERY {
                last_sync = Instant::now();
                if changed {
                    log::info!(
                        "状态：{:?}/{:?} 体力{:.0} 心情{:.0} 饱腹{:.0} 水{:.0}",
                        next.activity, next.mood, next.strength, next.feeling, next.hunger, next.thirst
                    );
                }
                let _ = app.emit("pet:state", next);
            }
        }
    });
}

/// Body 每换一帧推一次当前帧的 alpha 掩码（按位打包的 48×48）
#[tauri::command]
fn set_hit_mask(hit: State<'_, Mutex<HitState>>, cells: Vec<u8>) {
    const WANT: usize = (MASK_N * MASK_N).div_ceil(8);
    if cells.len() != WANT {
        log::warn!("掩码长度 {} != {WANT}，忽略", cells.len());
        return;
    }
    if let Ok(mut st) = hit.lock() {
        st.mask = Some(cells);
    }
}

/// 交互期间钉住不穿透，否则把宠物拖到光标不再压着它的位置时拖拽会断掉
#[tauri::command]
fn set_hit_test_pinned(hit: State<'_, Mutex<HitState>>, pinned: bool) {
    if let Ok(mut st) = hit.lock() {
        st.pinned = pinned;
    }
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(Mutex::new(HitState::default()))
        .manage(Mutex::new(PetState::default()))
        .setup(|app| {
            if let Some(win) = app.get_webview_window(PET_WINDOW) {
                place_bottom_right(&win);
            }
            build_tray(app.handle())?;
            spawn_hit_test(app.handle().clone());
            spawn_pet_clock(app.handle().clone());
            if let Err(e) = register_prompt_shortcut(app.handle()) {
                // 快捷键被别的程序占了不该拖垮启动，双击宠物一样能呼出输入框
                log::warn!("注册全局快捷键 {PROMPT_SHORTCUT} 失败: {e}");
            }
            log::info!("VPet {} 启动", env!("CARGO_PKG_VERSION"));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_version,
            get_pet_state,
            pet_touched,
            debug_patch_pet_state,
            set_hit_mask,
            set_hit_test_pinned
        ])
        .run(tauri::generate_context!())
        .expect("VPet 运行失败");
}

/// 把窗口放到当前显示器右下角（留出任务栏）。Phase 1 会改为记住上次位置。
fn place_bottom_right(win: &WebviewWindow) {
    let (Ok(Some(monitor)), Ok(size)) = (win.current_monitor(), win.outer_size()) else {
        return;
    };
    let scale = monitor.scale_factor();
    let margin_right = (24.0 * scale) as i32;
    let margin_bottom = (72.0 * scale) as i32; // 任务栏高度的粗略估计
    let mon_pos = monitor.position();
    let mon_size = monitor.size();
    let x = mon_pos.x + mon_size.width as i32 - size.width as i32 - margin_right;
    let y = mon_pos.y + mon_size.height as i32 - size.height as i32 - margin_bottom;
    if let Err(e) = win.set_position(PhysicalPosition::new(x.max(mon_pos.x), y.max(mon_pos.y))) {
        log::warn!("设置窗口位置失败: {e}");
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let toggle = MenuItem::with_id(app, "toggle", "显示 / 隐藏", true, None::<&str>)?;
    // 穿透在 Windows 上是有坑的一项（docs/07 风险表），留个能当场关掉的开关
    let passthrough = CheckMenuItem::with_id(app, "passthrough", "鼠标穿透", true, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &passthrough, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("VPet")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_pet(app),
            "passthrough" => toggle_passthrough(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Some(w) = tray.app_handle().get_webview_window(PET_WINDOW) {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// 每 POLL 读一次光标，压在宠物不透明像素上就关掉穿透，否则打开
/// （docs/05 §4 的 alpha 命中）。跑在后台线程，只在判定结果变化时才去动窗口。
fn spawn_hit_test(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        let Some(win) = app.get_webview_window(PET_WINDOW) else { continue };

        let ignore = match should_ignore(&app, &win) {
            Ok(v) => v,
            // 读不到光标 / 窗口位置就退到不穿透，别把宠物锁死
            Err(e) => {
                log::debug!("穿透判定失败，退到不穿透: {e}");
                false
            }
        };

        let hit = app.state::<Mutex<HitState>>();
        let Ok(mut st) = hit.lock() else { continue };
        if st.ignoring == ignore {
            continue;
        }
        if let Err(e) = win.set_ignore_cursor_events(ignore) {
            log::warn!("切换穿透失败: {e}");
            continue;
        }
        st.ignoring = ignore;
    });
}

fn should_ignore(app: &AppHandle, win: &WebviewWindow) -> Result<bool, String> {
    {
        let hit = app.state::<Mutex<HitState>>();
        let st = hit.lock().map_err(|_| "命中状态被污染")?;
        if st.disabled || st.pinned {
            return Ok(false);
        }
    }
    let cursor = app.cursor_position().map_err(|e| e.to_string())?;
    let origin = win.outer_position().map_err(|e| e.to_string())?;
    let scale = win.scale_factor().map_err(|e| e.to_string())?;
    // 物理像素 → 窗口内的逻辑坐标（窗口就是 500×500 逻辑像素，与掩码同一套参考系）
    let x = (cursor.x - origin.x as f64) / scale;
    let y = (cursor.y - origin.y as f64) / scale;

    let hit = app.state::<Mutex<HitState>>();
    let st = hit.lock().map_err(|_| "命中状态被污染")?;
    Ok(!st.opaque_at(x, y))
}

/// 全局快捷键 → 把宠物窗口叫到前台并让 Body 弹出输入框。
/// 窗口得先拿到焦点，否则输入框收不到键盘。
fn register_prompt_shortcut(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

    let shortcut: Shortcut = PROMPT_SHORTCUT.parse()?;
    app.global_shortcut().on_shortcut(shortcut, |app, _, event| {
        if event.state() != ShortcutState::Pressed {
            return;
        }
        if let Some(w) = app.get_webview_window(PET_WINDOW) {
            let _ = w.show();
            let _ = w.set_focus();
        }
        let _ = app.emit("pet:prompt", ());
    })?;
    Ok(())
}

/// 关掉穿透时立刻恢复成可点，别等下一轮轮询
fn toggle_passthrough(app: &AppHandle) {
    let hit = app.state::<Mutex<HitState>>();
    let Ok(mut st) = hit.lock() else { return };
    st.disabled = !st.disabled;
    if st.disabled && st.ignoring {
        if let Some(w) = app.get_webview_window(PET_WINDOW) {
            let _ = w.set_ignore_cursor_events(false);
        }
        st.ignoring = false;
    }
    log::info!("鼠标穿透: {}", if st.disabled { "关" } else { "开" });
}

fn toggle_pet(app: &AppHandle) {
    let Some(w) = app.get_webview_window(PET_WINDOW) else { return };
    if w.is_visible().unwrap_or(true) {
        let _ = w.hide();
    } else {
        let _ = w.show();
        let _ = w.set_focus();
    }
}
