//! M4 模型仓库（第一片）：按需下载器
//!
//! 产品决策（用户拍板）：安装包不内置模型（~30MB），模型按需下载——
//! 在线翻译默认可用；被过滤/断网时引导用户点「下载本地模型」落回离线兜底。
//! 支持：hf-mirror 镜像源、流式写盘、断点续传（Range）、进度事件。

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

pub struct ModelsState {
    downloading: AtomicBool,
}

impl Default for ModelsState {
    fn default() -> Self {
        Self {
            downloading: AtomicBool::new(false),
        }
    }
}

struct ModelSpec {
    /// hf-mirror 直链（国内可达优先）
    url: &'static str,
    /// 落盘文件名（mt.rs find_model 按此查找）
    dest: &'static str,
}

/// 内置模型清单（方案 §4.3：JSON 配置内置，可随版本更新）
fn spec(name: &str) -> Option<ModelSpec> {
    match name {
        "translation" => Some(ModelSpec {
            url: "https://hf-mirror.com/imikeliu/Hunyuan-MT-7B-Q4_K_M-GGUF/resolve/main/hunyuan-mt-7b-q4_k_m.gguf",
            dest: "translation.gguf",
        }),
        _ => None,
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    name: String,
    downloaded_mb: f64,
    total_mb: f64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoneEvent {
    name: String,
}

fn models_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "无 HOME")?;
    let dir = PathBuf::from(home).join(".dualsub/models");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

#[tauri::command]
pub fn cmd_model_download(app: AppHandle, name: String) -> Result<(), String> {
    let state = app.state::<ModelsState>();
    if state.downloading.swap(true, Ordering::SeqCst) {
        return Err("已有下载任务进行中".into());
    }

    let Some(spec) = spec(&name) else {
        state.downloading.store(false, Ordering::SeqCst);
        return Err(format!("未知模型: {name}"));
    };
    let dir = models_dir()?;
    let dest = dir.join(spec.dest);
    if dest.exists() {
        eprintln!("[models] {} 已存在，跳过下载", dest.display());
        state.downloading.store(false, Ordering::SeqCst);
        let _ = app.emit("models://done", DoneEvent { name: name.clone() });
        return Ok(());
    }

    let app2 = app.clone();
    let name2 = name.clone();
    std::thread::spawn(move || {
        let r = download(&app2, &name2, spec.url, &dest);
        app2.state::<ModelsState>().downloading.store(false, Ordering::SeqCst);
        match r {
            Ok(()) => {
                eprintln!("[models] {} 下载完成", dest.display());
                let _ = app2.emit("models://done", DoneEvent { name: name2 });
            }
            Err(e) => {
                eprintln!("[models] 下载失败: {e}");
                let _ = app2.emit("models://error", e);
            }
        }
    });
    Ok(())
}

fn download(app: &AppHandle, name: &str, url: &str, dest: &PathBuf) -> Result<(), String> {
    let mut builder = ureq::AgentBuilder::new().timeout_read(Duration::from_secs(60));
    if let Some(p) = crate::mt::env_proxy() {
        builder = builder.proxy(p);
    }
    let agent = builder.build();

    // 断点续传：.part 已有内容则 Range 续传
    let part = dest.with_extension("part");
    let mut start: u64 = part.metadata().map(|m| m.len()).unwrap_or(0);
    if start > 0 {
        eprintln!("[models] 断点续传自 {:.1}MB", start as f64 / 1048576.0);
    }

    let mut req = agent.get(url);
    if start > 0 {
        req = req.set("Range", &format!("bytes={start}-"));
    }
    let resp = req.call().map_err(|e| format!("下载请求失败: {e}"))?;
    let total: u64 = resp
        .header("content-length")
        .and_then(|h| h.parse::<u64>().ok())
        .map(|len| len + start)
        .unwrap_or(0);
    let mut reader = resp.into_reader();

    let mut file = if start > 0 {
        std::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .map_err(|e| e.to_string())?
    } else {
        std::fs::File::create(&part).map_err(|e| e.to_string())?
    };

    let mut buf = [0u8; 256 * 1024];
    let mut last_emit = 0u64;
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                start += n as u64;
                // 每 ~16MB 或 1% 进度发一次事件
                if total > 0 && start - last_emit > 16 * 1048576 {
                    last_emit = start;
                    let _ = app.emit(
                        "models://progress",
                        ProgressEvent {
                            name: name.to_string(),
                            downloaded_mb: start as f64 / 1048576.0,
                            total_mb: total as f64 / 1048576.0,
                        },
                    );
                }
            }
            Err(e) => return Err(format!("下载中断: {e}")),
        }
    }

    // sha256 校验留 M4 完整版（清单带 hash）；此处以文件大小兜底
    if total > 0 && start < total {
        return Err(format!("下载不完整（{}/{}）", start, total));
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())?;
    Ok(())
}
