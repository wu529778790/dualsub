//! 设置持久化（~/.livesub-player/settings.json）：简单 KV，各模块按 key 读取
//!
//! 数据目录统一走 `data_dir()`：首次调用时把旧版 `~/.dualsub` 整体迁移为
//! `~/.livesub-player`（模型/设置/翻译缓存一次搬完，避免重复下载）。

use std::path::PathBuf;

use serde_json::Value;

/// 应用数据目录：~/.livesub-player（首次调用时从旧 ~/.dualsub 迁移）
pub fn data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    let new_dir = PathBuf::from(&home).join(".livesub-player");
    let old_dir = PathBuf::from(&home).join(".dualsub");
    if !new_dir.exists() && old_dir.is_dir() {
        // 旧目录整体改名迁移；失败不影响后续（缺目录按未下载处理）
        let _ = std::fs::rename(&old_dir, &new_dir);
    }
    new_dir
}

pub fn settings_file() -> PathBuf {
    data_dir().join("settings.json")
}

fn read_all() -> Value {
    std::fs::read(settings_file())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

pub fn read(key: &str) -> Option<String> {
    read_all()[key].as_str().map(|s| s.to_string())
}

pub fn write(key: &str, value: &str) -> Result<(), String> {
    let mut v = read_all();
    v[key] = Value::String(value.to_string());
    let path = settings_file();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_string(&v).unwrap()).map_err(|e| e.to_string())
}

// ---------- Tauri 命令 ----------
#[tauri::command]
pub fn cmd_get_setting(key: String) -> Option<String> {
    read(&key)
}

#[tauri::command]
pub fn cmd_set_setting(key: String, value: String) -> Result<(), String> {
    write(&key, &value)
}
