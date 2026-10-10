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

/// macOS App Translocation 检测：从「下载」等位置直接运行带隔离标记的 app 时，
/// Gatekeeper 会把 app 挂载到只读临时目录（路径含 /AppTranslocation/），
/// 更新器无法在该位置替换文件（os error 30 Read-only file system）。
/// 提前拦截并给出可操作指引，而不是让用户面对裸系统错误。
fn translocation_error() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        Some(
            "应用正运行在系统临时位置（还没移入「应用程序」文件夹），无法自动更新。\
             请退出应用，把 LiveSub-Player 拖入「应用程序」文件夹，再从那里打开后重试更新。"
                .to_string(),
        )
    } else {
        None
    }
}

/// 从当前 exe 向上定位 .app 包路径
fn app_bundle_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(|p| p.to_path_buf())
}

/// 启动时检查应用是否已安装到「应用程序」文件夹。
/// 没安装（从下载目录/dmg 直接运行）会触发 Gatekeeper App Translocation，
/// app 被挂到只读目录导致在线更新无法替换文件——通知前端引导一键安装。
pub fn check_install_location(app: &AppHandle) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(bundle) = app_bundle_path() else {
        return;
    };
    // 接受系统级 /Applications 与用户级 ~/Applications
    let installed = bundle
        .parent()
        .and_then(|p| p.file_name())
        .is_some_and(|n| n == "Applications");
    if !installed {
        let _ = app.emit("app://not-installed", ());
    }
}

/// 一键安装到「应用程序」文件夹：ditto 自复制 → 清隔离标记 → 从新位置重启。
/// 成功路径下当前进程退出，前端 promise 不返回属预期。
#[tauri::command]
pub async fn cmd_install_to_applications(app: AppHandle) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("仅 macOS 支持此操作".into());
    }
    let bundle = app_bundle_path().ok_or("无法定位应用包")?;
    let name = bundle.file_name().ok_or("无法定位应用包")?;
    let dst = std::path::Path::new("/Applications").join(name);
    if dst.exists() {
        return Err("「应用程序」文件夹中已有 LiveSub-Player，请退出后直接从那里打开".into());
    }
    // ditto 保留元数据与权限
    let st = std::process::Command::new("ditto")
        .args([
            bundle.to_string_lossy().as_ref(),
            dst.to_string_lossy().as_ref(),
        ])
        .status()
        .map_err(|e| format!("复制应用失败: {e}"))?;
    if !st.success() {
        return Err("复制应用到「应用程序」文件夹失败（可能没有写入权限）".into());
    }
    // 清掉隔离标记，避免下次启动再被 Gatekeeper 转移到只读目录
    let _ = std::process::Command::new("xattr")
        .args(["-rd", "com.apple.quarantine", dst.to_string_lossy().as_ref()])
        .status();
    // 从新位置重启
    let _ = std::process::Command::new("open").arg(&dst).spawn();
    app.exit(0);
    Ok(())
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
    // 下载前先拦：Translocation 环境下载完也装不进去，省一次全量下载
    if let Some(msg) = translocation_error() {
        return Err(msg);
    }

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
