use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, WebviewWindow};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DesktopSettings {
    pub size: f64,
    pub always_on_top: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self { size: 500.0, always_on_top: true }
    }
}

impl DesktopSettings {
    fn validated(mut self) -> Result<Self, String> {
        if !self.size.is_finite() {
            return Err("显示大小必须是有效数字".into());
        }
        self.size = self.size.round().clamp(200.0, 800.0);
        Ok(self)
    }

    pub fn apply(&self, window: &WebviewWindow) -> Result<(), String> {
        window.set_size(LogicalSize::new(self.size, self.size)).map_err(|e| e.to_string())?;
        window.set_always_on_top(self.always_on_top).map_err(|e| e.to_string())
    }
}

pub fn init(app: &AppHandle) -> DesktopSettings {
    let settings = app.path().app_data_dir().ok()
        .and_then(|dir| std::fs::read(dir.join("desktop-settings.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<DesktopSettings>(&bytes).ok())
        .and_then(|settings| settings.validated().ok())
        .unwrap_or_default();
    app.manage(Mutex::new(settings.clone()));
    settings
}

#[tauri::command]
pub fn get_desktop_settings(app: AppHandle) -> Result<DesktopSettings, String> {
    app.state::<Mutex<DesktopSettings>>().lock().map(|s| s.clone())
        .map_err(|_| "无法读取显示设置".into())
}

#[tauri::command]
pub fn set_desktop_settings(app: AppHandle, settings: DesktopSettings) -> Result<DesktopSettings, String> {
    let settings = settings.validated()?;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let state = app.state::<Mutex<DesktopSettings>>();
    let mut saved = state.lock().map_err(|_| "无法保存显示设置")?;
    let window = app.get_webview_window(crate::PET_WINDOW).ok_or("桌宠窗口尚未就绪")?;
    settings.apply(&window)?;
    let bytes = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
    if let Err(e) = std::fs::write(dir.join("desktop-settings.json"), bytes) {
        let _ = saved.apply(&window);
        return Err(format!("显示设置写入失败：{e}"));
    }
    *saved = settings.clone();
    drop(saved);
    let _ = app.emit("desktop:settings", &settings);
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_window_size_and_roundtrips_topmost() {
        for (size, expected) in [(1.0, 200.0), (900.0, 800.0), (420.4, 420.0)] {
            let settings = DesktopSettings { size, always_on_top: false }.validated().unwrap();
            assert_eq!(settings.size, expected);
            let loaded: DesktopSettings = serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(loaded, settings);
        }
        assert!(DesktopSettings { size: f64::NAN, always_on_top: true }.validated().is_err());
    }
}
