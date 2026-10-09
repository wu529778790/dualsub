//! M5：应用内新版本提示（替代 Tauri updater——未签名 app 对 updater 支持差）
//!
//! 启动时查 GitHub Releases 最新 tag，与应用版本比较；提示用户跳转下载页。

use serde::Serialize;
use tauri::AppHandle;

const RELEASES_API: &str = "https://api.github.com/repos/wu529778790/dualsub/releases/latest";
const RELEASES_PAGE: &str = "https://github.com/wu529778790/dualsub/releases/latest";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
}

#[tauri::command]
pub fn cmd_check_update(_app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let latest = fetch_latest_tag()?;
    // 去掉 v 前缀比较
    let latest_clean = latest.trim_start_matches('v').to_string();
    let has_update = version_gt(&latest_clean, &current);
    Ok(Some(UpdateInfo {
        current,
        latest: latest_clean,
        has_update,
    }))
}

fn fetch_latest_tag() -> Result<String, String> {
    let resp = ureq::get(RELEASES_API)
        .set("User-Agent", "dualsub-app")
        .timeout(std::time::Duration::from_secs(8))
        .call()
        .map_err(|e| format!("检查更新失败: {e}"))?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    v["tag_name"]
        .as_str()
        .map(|s| s.to_string())
        .ok_or("响应缺少 tag_name".into())
}

/// 简化 semver 比较：逐段数值比较（v0.2.1 > v0.1.9）
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .map(|p| p.chars().filter(|c| c.is_ascii_digit()).collect::<String>())
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    let (pa, pb) = (parse(a), parse(b));
    for i in 0..pa.len().max(pb.len()) {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

/// 打开 Release 下载页
#[tauri::command]
pub fn cmd_open_releases() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "windows")]
    let program = "explorer";

    std::process::Command::new(program)
        .arg(RELEASES_PAGE)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("打开浏览器失败: {e}"))
}
