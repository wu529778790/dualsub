//! 设置持久化（~/.dualsub/settings.json）：简单 KV，各模块按 key 读取

use std::path::PathBuf;

use serde_json::Value;

pub fn settings_file() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".dualsub/settings.json")
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
