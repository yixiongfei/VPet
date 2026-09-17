//! VPet Core（Rust 侧）。
//!
//! 目前这里负责：宠物窗口的摆位与托盘、鼠标穿透的 alpha 命中判定、全局快捷键，
//! 以及 `core/state_machine.rs` 的状态机和驱动它的心跳——数值随时间走、饿了自己去吃，
//! 状态落在 `core/db.rs` 的 SQLite 里，重启能接着上次继续。
//! 调度 / 番茄钟 / 工具 / 权限还没做，见 docs/03-architecture.md §3 与 docs/07 Phase 2。

mod core;

use std::sync::{Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use core::actions::Catalog;
use core::db::Db;
use core::food::{FoodItem, FoodShelf};
use core::pomodoro::{Phase, Pomodoro, Tick as PomoTick};
use core::scheduler::{parse_duration, Scheduler, Timer};
use core::tools::{
    builtin_tools, find as find_tool, summarize_input, AuditEntry, AuditLog, Decision, Origin,
    PermissionGate, ToolCall, ToolDef, ToolResult,
};
use core::state_machine::{catch_up, reduce, Event, Pet, PetState, Touch};

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow,
};
use tauri_plugin_global_shortcut::Shortcut;

const PET_WINDOW: &str = "pet";
const PANEL_WINDOW: &str = "panel";

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
/// 活动和心情都没变时，最多隔这么久也要把数值同步给 Body 一次
const SYNC_EVERY: Duration = Duration::from_secs(30);
/// 状态落库的间隔。一分钟一条，掉电最多丢一分钟的数值
const PERSIST_EVERY: Duration = Duration::from_secs(60);

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
fn get_pet_state(pet: State<'_, Mutex<Pet>>) -> PetState {
    pet.lock().map(|p| p.state.clone()).unwrap_or_default()
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
    money: Option<f32>,
) {
    apply(
        &app,
        &Event::Patch {
            strength,
            feeling,
            hunger,
            thirst,
            money,
        },
    );
}

/// 把事件喂给状态机，落到共享状态，然后广播给 Body
fn apply(app: &AppHandle, event: &Event) {
    let cat = app.state::<Catalog>();
    let shelf = app.state::<RwLock<FoodShelf>>();
    let Ok(shelf) = shelf.read() else { return };
    let pet = app.state::<Mutex<Pet>>();
    let Ok(mut st) = pet.lock() else { return };
    let mut next = reduce(&cat, &shelf, &st, event);
    next.state.updated_at = now_ms();
    *st = next.clone();
    drop(st);
    let _ = app.emit("pet:state", next.state);
}

/// 当前钟点（本地时区，0–24 的小数）。作息判断要的就是「现在几点」，
/// 状态机不读时钟，由这里喂进去
fn hour_now() -> f32 {
    use chrono::Timelike;
    let t = chrono::Local::now();
    t.hour() as f32 + t.minute() as f32 / 60.0
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
        let mut last_persist = Instant::now();

        loop {
            std::thread::sleep(TICK);
            let cat = app.state::<Catalog>();
            let shelf = app.state::<RwLock<FoodShelf>>();
            let Ok(shelf) = shelf.read() else { continue };
            let pet = app.state::<Mutex<Pet>>();
            let Ok(mut st) = pet.lock() else { continue };
            let before = st.state.clone();

            let mut next = reduce(
                &cat,
                &shelf,
                &st,
                &Event::Tick {
                    minutes: TICK.as_secs_f32() / 60.0,
                    hour: hour_now(),
                },
            );
            next.state.updated_at = now_ms();
            *st = next.clone();
            drop(st);

            let acted = next.state.action.as_ref().map(|a| a.id.as_str());
            let acted_before = before.action.as_ref().map(|a| a.id.as_str());
            let changed = acted != acted_before || next.state.mood != before.mood;
            if changed || last_sync.elapsed() >= SYNC_EVERY {
                last_sync = Instant::now();
                if changed {
                    if let Some(a) = next.state.action.as_ref() {
                        let what = match a.food.as_ref() {
                            Some(f) => format!("{}（{}）", a.name, f.name),
                            None => a.name.clone(),
                        };
                        log::info!(
                            "{} —— {}（体力{:.0} 心情{:.0} 饱腹{:.0} 水{:.0} 钱{:.0} Lv{}）",
                            what, a.reason,
                            next.state.strength, next.state.feeling,
                            next.state.hunger, next.state.thirst,
                            next.state.money, next.state.level
                        );
                    }
                }
                let _ = app.emit("pet:state", next.state.clone());
            }
            fire_due_timers(&app);
            advance_pomodoro(&app, TICK.as_secs_f32());

            if last_persist.elapsed() >= PERSIST_EVERY {
                last_persist = Instant::now();
                persist(&app, &next);
            }
        }
    });
}

/// 到点的计时器响一下。跟着心跳走，不另起一套时钟
fn fire_due_timers(app: &AppHandle) {
    let fired = {
        let sched = app.state::<Mutex<Scheduler>>();
        let Ok(mut sc) = sched.lock() else { return };
        let due = sc.take_due(now_ms());
        if due.is_empty() {
            return;
        }
        save_timers(app, &sc);
        due
    };
    for t in fired {
        log::info!("计时器响了：{}", t.label);
        let _ = app.emit("timer:fired", &t);
    }
}

/// 落一条状态流水。存不进去不该影响宠物继续跑——大不了这次的数值丢了
fn persist(app: &AppHandle, p: &Pet) {
    let Some(db) = app.try_state::<Mutex<Db>>() else { return };
    let Ok(db) = db.lock() else { return };
    if let Err(e) = db.record_pet_state(p) {
        log::warn!("状态落库失败: {e}");
    }
}

/// 开库、读回上次的状态、把关掉期间的时间补上。
///
/// 数据库打不开就在内存里跑（数值不留存），不该让宠物起不来。
fn restore_state(app: &AppHandle, cat: &Catalog, shelf: &FoodShelf) -> Pet {
    let path = match app.path().app_data_dir() {
        Ok(dir) => dir.join("vpet.db"),
        Err(e) => {
            log::warn!("拿不到数据目录，这次不落库: {e}");
            return Pet::default();
        }
    };
    let db = match Db::open(&path) {
        Ok(db) => db,
        Err(e) => {
            log::warn!("数据库打不开，这次不落库: {e}");
            return Pet::default();
        }
    };
    let restored = match db.latest_pet_state() {
        Ok(Some(p)) => {
            let after = catch_up(cat, shelf, &p, now_ms(), hour_now());
            log::info!(
                "读回上次状态（距今 {:.0} 分钟）：饱腹 {:.0} → {:.0}",
                (now_ms() - p.state.updated_at) as f32 / 60_000.0,
                p.state.hunger,
                after.state.hunger
            );
            after
        }
        Ok(None) => Pet::default(),
        Err(e) => {
            log::warn!("读上次状态失败，按新宠物开始: {e}");
            Pet::default()
        }
    };
    app.manage(Mutex::new(db));
    restored
}

/// Brain 能用的全部工具。`executor` 不在这张表里——LLM 不需要知道怎么执行
#[tauri::command]
fn list_tools() -> Vec<ToolDef> {
    builtin_tools()
}

/// 最近的审计。Panel 用来看她都做了什么
#[tauri::command]
fn recent_audit(audit: State<'_, Mutex<AuditLog>>, limit: Option<usize>) -> Vec<AuditEntry> {
    audit
        .lock()
        .map(|a| a.recent(limit.unwrap_or(50)))
        .unwrap_or_default()
}

/// 唯一的工具入口（docs/03 §5）。
///
/// 顺序是固定的：查工具 → 过权限门 → 执行 → 落审计。**审计一定要落**，
/// 哪怕被拒绝了——「她想做但没让做」和「她做了」一样重要。
#[tauri::command]
fn run_tool(app: AppHandle, call: ToolCall) -> ToolResult {
    let tools = builtin_tools();
    let Some(def) = find_tool(&tools, &call.name) else {
        return ToolResult::err(&call.call_id, "unknown_tool", format!("没有这个工具：{}", call.name));
    };

    let decision = {
        let gate = app.state::<Mutex<PermissionGate>>();
        let Ok(g) = gate.lock() else {
            return ToolResult::err(&call.call_id, "denied", "权限表被污染");
        };
        g.check(def, call.origin)
    };

    let result = match decision {
        Decision::Deny => ToolResult::err(&call.call_id, "denied", "这件事没有授权"),
        // Ask 的确认气泡是 Phase 3 的 UX（docs/03 §6）。在那之前一律当拒绝处理，
        // 宁可少做也不要在用户没点头的情况下动手
        Decision::Ask => {
            let _ = app.emit("tool:confirm", &call.name);
            ToolResult::err(&call.call_id, "denied", "需要你点头（确认气泡在 Phase 3）")
        }
        Decision::Allow => execute(&app, &call),
    };

    let audit = app.state::<Mutex<AuditLog>>();
    if let Ok(mut a) = audit.lock() {
        let entry = AuditEntry {
            at: now_ms(),
            tool: call.name.clone(),
            origin: call.origin,
            decision,
            ok: result.ok,
            input: summarize_input(&call.input),
            summary: match (&result.data, &result.error) {
                (Some(d), _) => summarize_input(d),
                (_, Some(e)) => e.message.clone(),
                _ => String::new(),
            },
        };
        let _ = app.emit("audit:appended", &entry);
        a.record(entry);
    }
    result
}

/// 工具的真正执行。放这里是因为只有这儿够得着 app state
fn execute(app: &AppHandle, call: &ToolCall) -> ToolResult {
    let id = &call.call_id;
    let arg = |k: &str| call.input.get(k).and_then(|v| v.as_str()).map(String::from);
    match call.name.as_str() {
        "create_timer" => {
            let Some(duration) = arg("duration") else {
                return ToolResult::err(id, "invalid_input", "缺 duration");
            };
            let repeat = call.input.get("repeat").and_then(|v| v.as_bool());
            match create_timer(app.clone(), duration, arg("label"), repeat) {
                Ok(t) => ToolResult::ok(id, serde_json::to_value(t).unwrap_or_default()),
                Err(e) => ToolResult::err(id, "invalid_input", e),
            }
        }
        "cancel_timer" => match arg("id") {
            Some(tid) => ToolResult::ok(id, serde_json::json!({ "cancelled": cancel_timer(app.clone(), tid) })),
            None => ToolResult::err(id, "invalid_input", "缺 id"),
        },
        "list_timers" => {
            let sched = app.state::<Mutex<Scheduler>>();
            let list = sched.lock().map(|s| s.list().to_vec()).unwrap_or_default();
            ToolResult::ok(id, serde_json::to_value(list).unwrap_or_default())
        }
        "start_pomodoro" => ToolResult::ok(
            id,
            serde_json::to_value(start_pomodoro(app.clone())).unwrap_or_default(),
        ),
        "stop_pomodoro" => ToolResult::ok(
            id,
            serde_json::json!({ "completed": stop_pomodoro(app.clone()) }),
        ),
        "get_pet_state" => {
            let pet = app.state::<Mutex<Pet>>();
            let st = pet.lock().map(|p| p.state.clone()).unwrap_or_default();
            ToolResult::ok(id, serde_json::to_value(st).unwrap_or_default())
        }
        "set_permission" => {
            let (Some(scope), Some(d)) = (arg("scope"), arg("decision")) else {
                return ToolResult::err(id, "invalid_input", "缺 scope 或 decision");
            };
            let decision = match d.as_str() {
                "allow" => Decision::Allow,
                "ask" => Decision::Ask,
                "deny" => Decision::Deny,
                other => return ToolResult::err(id, "invalid_input", format!("看不懂的决定：{other}")),
            };
            let gate = app.state::<Mutex<PermissionGate>>();
            if let Ok(mut g) = gate.lock() {
                g.set(&scope, decision);
                log::info!("授权变更：{scope} → {d}");
            }
            ToolResult::ok(id, serde_json::json!({ "scope": scope, "decision": d }))
        }
        other => ToolResult::err(id, "unknown_tool", format!("没有这个工具：{other}")),
    }
}

/// 开始番茄钟。已经在跑就返回当前进度，不会把它清零
#[tauri::command]
fn start_pomodoro(app: AppHandle) -> Option<PomoTick> {
    let pomo = app.state::<Mutex<Pomodoro>>();
    let started = {
        let Ok(mut p) = pomo.lock() else { return None };
        let fresh = p.start().is_some();
        if fresh {
            log::info!("番茄钟开始，{:.0} 分钟专注", p.rhythm.focus_min);
        }
        (fresh, p.snapshot())
    };
    if started.0 {
        apply(&app, &Event::Pin(Some("work".into())));
        let _ = app.emit("pomodoro:phase", PhaseEvent { from: Phase::Focus, to: Phase::Focus });
    }
    started.1
}

/// 停掉番茄钟，返回这一轮完成了几个专注
#[tauri::command]
fn stop_pomodoro(app: AppHandle) -> Option<u32> {
    let pomo = app.state::<Mutex<Pomodoro>>();
    let done = {
        let Ok(mut p) = pomo.lock() else { return None };
        p.stop()
    }?;
    log::info!("番茄钟结束，完成 {done} 个专注");
    apply(&app, &Event::Pin(None));
    Some(done)
}

#[tauri::command]
fn get_pomodoro(pomo: State<'_, Mutex<Pomodoro>>) -> Option<PomoTick> {
    pomo.lock().ok().and_then(|p| p.snapshot())
}

#[derive(Clone, serde::Serialize)]
struct PhaseEvent {
    from: Phase,
    to: Phase,
}

/// 番茄钟跟着心跳走。相位变了就换她该做的事，并告诉 Body
fn advance_pomodoro(app: &AppHandle, sec: f32) {
    let (change, snap) = {
        let pomo = app.state::<Mutex<Pomodoro>>();
        let Ok(mut p) = pomo.lock() else { return };
        if !p.is_running() {
            return;
        }
        (p.advance(sec), p.snapshot())
    };
    if let Some(c) = change {
        log::info!("番茄钟：{} → {}（已完成 {}）", c.from.label(), c.to.label(), c.completed);
        let tag = if c.to == Phase::Focus { "work" } else { "rest" };
        apply(app, &Event::Pin(Some(tag.into())));
        let _ = app.emit("pomodoro:phase", PhaseEvent { from: c.from, to: c.to });
    }
    if let Some(t) = snap {
        let _ = app.emit("pomodoro:tick", t);
    }
}

/// 排一个计时器。`duration` 按人写的方式给：`"10s"` `"25m"` `"1h"`
#[tauri::command]
fn create_timer(
    app: AppHandle,
    duration: String,
    label: Option<String>,
    repeat: Option<bool>,
) -> Result<Timer, String> {
    let ms = parse_duration(&duration).ok_or_else(|| format!("看不懂的时长：{duration}"))?;
    let sched = app.state::<Mutex<Scheduler>>();
    let Ok(mut sc) = sched.lock() else {
        return Err("调度器被污染".into());
    };
    let t = sc.add(
        label.as_deref().unwrap_or("计时器"),
        ms,
        repeat.unwrap_or(false),
        now_ms(),
    );
    log::info!("排了计时器「{}」，{duration} 后响", t.label);
    save_timers(&app, &sc);
    Ok(t)
}

#[tauri::command]
fn cancel_timer(app: AppHandle, id: String) -> bool {
    let sched = app.state::<Mutex<Scheduler>>();
    let Ok(mut sc) = sched.lock() else { return false };
    let ok = sc.cancel(&id);
    if ok {
        save_timers(&app, &sc);
    }
    ok
}

#[tauri::command]
fn list_timers(sched: State<'_, Mutex<Scheduler>>) -> Vec<Timer> {
    sched.lock().map(|s| s.list().to_vec()).unwrap_or_default()
}

fn save_timers(app: &AppHandle, sc: &Scheduler) {
    let Some(db) = app.try_state::<Mutex<Db>>() else { return };
    let Ok(db) = db.lock() else { return };
    if let Err(e) = db.save_scheduler(sc) {
        log::warn!("计时器落库失败: {e}");
    }
}

/// 打开面板窗口。已经开着就叫到前台，不重复开
fn open_panel(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(PANEL_WINDOW) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let url = tauri::WebviewUrl::App("panel.html".into());
    match tauri::WebviewWindowBuilder::new(app, PANEL_WINDOW, url)
        .title("VPet 面板")
        .inner_size(760.0, 640.0)
        .min_inner_size(420.0, 420.0)
        .build()
    {
        Ok(_) => log::info!("面板已打开"),
        Err(e) => log::warn!("打开面板失败: {e}"),
    }
}

/// 用户送她一样礼物。随机挑一件——拆盲盒比让人从二十项里选更有意思。
/// 礼物不花她的钱，钱是用户出的。
#[tauri::command]
fn give_gift(app: AppHandle) -> Option<String> {
    let shelf = app.state::<RwLock<FoodShelf>>();
    let picked = {
        let Ok(s) = shelf.read() else { return None };
        s.random("gift", now_ms() as u64)
            .map(|f| (f.id.clone(), f.name.clone()))
    };
    let (id, name) = picked?;
    log::info!("收到礼物：{name}");
    apply(&app, &Event::Gifted { id, name: name.clone() });
    Some(name)
}

/// Body 启动时把食物目录交给 Core。
///
/// 123 项食物是 build-assets 从原版 `food/*.lps` 转出来的，躺在 Body 的 manifest 里；
/// Core 要按需求和钱包挑一样买，就得先拿到这张表。和推命中掩码一个路子。
#[tauri::command]
fn set_food_catalog(shelf: State<'_, RwLock<FoodShelf>>, items: Vec<FoodItem>) {
    let n = items.len();
    if let Ok(mut s) = shelf.write() {
        s.set(items);
        log::info!("收到食物目录：{n} 项");
    }
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
        .setup(|app| {
            // 动作表读在最前面：状态机的每一步都要查它
            let cat = Catalog::load();
            // 再把上次的状态读回来（含关掉期间的补算），然后让心跳接手
            // 食物目录要等 Body 推过来，这之前货架是空的（买不起就退回动作表自带的量）
            let shelf = FoodShelf::default();
            let restored = restore_state(app.handle(), &cat, &shelf);
            app.manage(cat);
            app.manage(RwLock::new(shelf));
            // 计时器：过期的下一拍就会补发，没到期的接着等
            let timers = app
                .try_state::<Mutex<Db>>()
                .and_then(|db| db.lock().ok().map(|d| d.load_scheduler()))
                .unwrap_or_default();
            if !timers.is_empty() {
                log::info!("读回 {} 个计时器", timers.len());
            }
            app.manage(Mutex::new(timers));
            app.manage(Mutex::new(Pomodoro::default()));
            app.manage(Mutex::new(PermissionGate::default()));
            app.manage(Mutex::new(AuditLog::default()));
            app.manage(Mutex::new(restored));
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
            set_hit_test_pinned,
            set_food_catalog,
            give_gift,
            create_timer,
            cancel_timer,
            list_timers,
            start_pomodoro,
            stop_pomodoro,
            get_pomodoro,
            list_tools,
            run_tool,
            recent_audit
        ])
        .build(tauri::generate_context!())
        .expect("VPet 启动失败")
        .run(|app, event| {
            // 退出前再存一次，免得白丢最后这一分钟的数值
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(pet) = app.try_state::<Mutex<Pet>>() {
                    let snapshot = pet.lock().map(|p| p.clone()).ok();
                    if let Some(mut p) = snapshot {
                        p.state.updated_at = now_ms();
                        persist(app, &p);
                    }
                }
            }
        });
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
    let panel = MenuItem::with_id(app, "panel", "面板", true, None::<&str>)?;
    let gift = MenuItem::with_id(app, "gift", "送她礼物", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &panel, &gift, &passthrough, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("VPet")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_pet(app),
            "panel" => open_panel(app),
            "gift" => {
                give_gift(app.clone());
            }
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
