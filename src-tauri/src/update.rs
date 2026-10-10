//! M5：应用内自动更新（tauri-plugin-updater）
//!
//! 之前误判「未签名 app 对 updater 支持差」：Tauri 的更新签名是自带的
//! minisign 体系（公钥内置 app、私钥只在 CI），与 Apple 签名/公证无关，
//! ad-hoc 签名的 .app 一样能被替换重启；浏览器之外的下载也不带隔离标记。
//!
//! 端点指向 GitHub Releases 最新版的 latest.json（由 release.yml 在打包后
//! 手工生成——macOS 更新包必须在内嵌 libmpv 重签之后再打 tar 并签名，
//! bundler 自带的更新产物是错误的）。注意 Releases 必须以正式版
//! （非 prerelease）发布，否则 releases/latest 解析不到。

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::UpdaterExt;

/// 在 check 与 install 两个命令之间暂存已检查到的更新对象
#[derive(Default)]
pub struct UpdateState {
    pending: std::sync::Mutex<Option<tauri_plugin_updater::Update>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub has_update: bool,
    pub notes: Option<String>,
}

#[tauri::command]
pub async fn cmd_check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());

    let update = app
        .updater()
        .map_err(|e| format!("更新器不可用: {e}"))?
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;

    let Some(update) = update else {
        return Ok(UpdateInfo {
            current: current.clone(),
            latest: current,
            has_update: false,
            notes: None,
        });
    };

    let info = UpdateInfo {
        current: current.clone(),
        latest: update.version.clone(),
        has_update: true,
        notes: update.body.clone(),
    };
    let state: State<UpdateState> = app.state();
    *state.pending.lock().unwrap() = Some(update);
    Ok(info)
}

/// 下载并安装更新：进度经 update://progress 事件推送（百分比），
/// 成功路径下 install 内部会替换应用并重启进程，前端无需收尾
#[tauri::command]
pub async fn cmd_install_update(app: AppHandle) -> Result<(), String> {
    let update = {
        let state: State<UpdateState> = app.state();
        let mut pending = state.pending.lock().unwrap();
        pending.take()
    };
    let update = update.ok_or("尚未检查到更新，请重新打开应用")?;

    let handle = app.clone();
    let mut received: u64 = 0;
    update
        .download_and_install(
            |chunk_len, total| {
                received += chunk_len as u64;
                let pct = total.map(|t| (received * 100 / t) as u32).unwrap_or(0);
                let _ = handle.emit("update://progress", pct.min(100));
            },
            || {},
        )
        .await
        .map_err(|e| format!("下载安装更新失败: {e}"))?;

    let _ = app.emit("update://progress", 100);
    // macOS/Linux：安装只替换文件，需重启进新版本；Windows：安装器退出前会自行结束进程。
    // restart() 不返回（进程退出重启），前端 promise 永远 pending 属预期。
    app.restart();
}
