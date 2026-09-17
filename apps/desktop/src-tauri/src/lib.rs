//! VPet Core（Rust 侧）。
//!
//! Phase 0 只做三件事：把 pet 窗口放到屏幕右下角、托盘菜单（显示/隐藏、退出）、一个 `app_version` 命令
//! 用来验证 IPC。状态机 / 调度 / 工具 / 权限 / 存储从 Phase 2 起在 `core/` `tools/` `memory/` 下展开
//! （见 docs/03-architecture.md §3）。

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow,
};
use tauri_plugin_global_shortcut::Shortcut;

const PET_WINDOW: &str = "pet";

/// 呼出输入框的全局快捷键（docs/05 §4 的默认值，Q 待确认）
const PROMPT_SHORTCUT: &str = "Alt+V";

#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Phase 1 的假状态源：把调用方给的载荷原样当 `pet:state` 发给 Body，用来调
/// 「活动/心情 → 动画」的映射。Phase 2 起改由 `core/state_machine.rs` 的真状态机发，
/// 这个命令随之删掉。载荷形状由前端的 zod `PetState` 把关（docs/03 §5 事件面）。
#[tauri::command]
fn debug_set_pet_state(app: AppHandle, state: serde_json::Value) -> Result<(), String> {
    app.emit("pet:state", state).map_err(|e| e.to_string())
}

pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            if let Some(win) = app.get_webview_window(PET_WINDOW) {
                place_bottom_right(&win);
            }
            build_tray(app.handle())?;
            if let Err(e) = register_prompt_shortcut(app.handle()) {
                // 快捷键被别的程序占了不该拖垮启动，双击宠物一样能呼出输入框
                log::warn!("注册全局快捷键 {PROMPT_SHORTCUT} 失败: {e}");
            }
            log::info!("VPet {} 启动", env!("CARGO_PKG_VERSION"));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![app_version, debug_set_pet_state])
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
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("VPet")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => toggle_pet(app),
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

fn toggle_pet(app: &AppHandle) {
    let Some(w) = app.get_webview_window(PET_WINDOW) else { return };
    if w.is_visible().unwrap_or(true) {
        let _ = w.hide();
    } else {
        let _ = w.show();
        let _ = w.set_focus();
    }
}
