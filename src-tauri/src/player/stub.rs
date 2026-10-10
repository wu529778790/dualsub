//! 非 macOS 平台 stub：保证跨平台可编译可打包，具体播放内核随 M1-Windows 接入
//! （Windows: mpv 子 HWND 嵌入；Linux X11: subwindow；Wayland: 独立窗口降级）

use serde::Serialize;
use tauri::Manager;

#[derive(Default)]
pub struct PlayerState {}

/// 推给前端的播放状态（与 mac.rs 保持同构）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEvent {
    pub time_pos: Option<f64>,
    pub duration: Option<f64>,
    pub paused: Option<bool>,
}

const UNSUPPORTED: &str = "该平台播放内核尚未接入（Windows/Linux 里程碑进行中）";

#[tauri::command]
pub fn cmd_player_load(_path: String) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub fn cmd_player_toggle_pause() -> Result<bool, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub fn cmd_player_seek(_target: f64, _absolute: bool) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub fn cmd_player_set_volume(_volume: f64) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub fn cmd_player_set_speed(_speed: f64) -> Result<(), String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub fn cmd_player_toggle_fullscreen(app: tauri::AppHandle) -> Result<bool, String> {
    // 全屏是窗口操作，跨平台直接可用
    let window = app
        .get_webview_window("main")
        .ok_or("找不到主窗口")?;
    let next = !window.is_fullscreen().unwrap_or(false);
    window.set_fullscreen(next).map_err(|e| e.to_string())?;
    Ok(next)
}

#[tauri::command]
pub fn cmd_player_get_state() -> Result<PlayerEvent, String> {
    Err(UNSUPPORTED.into())
}

#[tauri::command]
pub fn cmd_player_screenshot() -> Result<String, String> {
    Err(UNSUPPORTED.into())
}

