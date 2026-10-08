//! M3 翻译管线（技术方案 §4.2）
//!
//! - llama-server sidecar 常驻（OpenAI 兼容 /v1/chat/completions），懒启动、健康检查、退出收割
//! - 攒批翻译：4 句一批（方案：3–5 句带上下文，译文连贯且省 token），2s 超时触发
//! - 翻译缓存：(源文本, 源语言, 目标语言) 键——M3 固定 zh 目标，键取源文本；内存 + 磁盘 JSON
//! - 降级：模型/binary 缺失时标记不可用，播放照常（「看剧永不停」原则），前端隐藏译文行

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 每批最大句数（方案 §4.2：3–5 句）
const BATCH_LINES: usize = 4;
/// 攒批超时：首句入队后等这么久没凑满就翻译
const BATCH_TIMEOUT_MS: u64 = 2000;
const BASE_PORT: u16 = 18434;

pub struct MtState {
    tx: Mutex<Option<Sender<TransJob>>>,
    child: Mutex<Option<std::process::Child>>,
    cache: Mutex<HashMap<String, String>>,
    /// 翻译服务可用性（false 后前端隐藏译文行）
    pub unavailable: AtomicBool,
}

impl Default for MtState {
    fn default() -> Self {
        Self {
            tx: Mutex::new(None),
            child: Mutex::new(None),
            cache: Mutex::new(HashMap::new()),
            unavailable: AtomicBool::new(false),
        }
    }
}

#[derive(Debug)]
struct TransJob {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TranslationEvent {
    start: f64,
    end: f64,
    src: String,
    dst: String,
}

// ---------- 对 asr 的接口 ----------
pub fn enqueue(app: &AppHandle, start: f64, end: f64, text: &str) {
    if app.state::<MtState>().unavailable.load(Ordering::Relaxed) {
        return;
    }
    let st = app.state::<MtState>();
    let mut tx_slot = st.tx.lock().unwrap();
    if tx_slot.is_none() {
        let (tx, rx) = mpsc::channel::<TransJob>();
        let app2 = app.clone();
        std::thread::spawn(move || worker(app2, rx));
        *tx_slot = Some(tx);
    }
    let _ = tx_slot.as_ref().unwrap().send(TransJob {
        start,
        end,
        text: text.to_string(),
    });
}

/// 主进程退出时收割 llama-server（方案 §4.4：退出即干净）
pub fn shutdown(app: &AppHandle) {
    let st = app.state::<MtState>();
    let mut guard = st.child.lock().unwrap();
    if let Some(mut child) = guard.take() {
        eprintln!("[mt] 收割 llama-server (pid {:?})", child.id());
        let _ = child.kill();
        let _ = child.wait();
    }
}

// ---------- worker：攒批翻译 ----------
fn worker(app: AppHandle, rx: Receiver<TransJob>) {
    // 启动时加载磁盘缓存
    let cache_path = cache_file();
    {
        let st = app.state::<MtState>();
        if let Ok(bytes) = std::fs::read(&cache_path) {
            if let Ok(map) = serde_json::from_slice::<HashMap<String, String>>(&bytes) {
                eprintln!("[mt] 磁盘缓存载入 {} 条", map.len());
                *st.cache.lock().unwrap() = map;
            }
        }
    }

    let mut batch: Vec<TransJob> = Vec::new();
    loop {
        // 攒批：BATCH_LINES 句或 BATCH_TIMEOUT_MS 触发
        if batch.len() >= BATCH_LINES {
            flush(&app, &mut batch, &cache_path);
            continue;
        }
        match rx.recv_timeout(Duration::from_millis(if batch.is_empty() {
            u64::MAX / 2 // 队列空：长等
        } else {
            BATCH_TIMEOUT_MS
        })) {
            Ok(job) => batch.push(job),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    flush(&app, &mut batch, &cache_path);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn flush(app: &AppHandle, batch: &mut Vec<TransJob>, cache_path: &PathBuf) {
    if batch.is_empty() {
        return;
    }
    let st = app.state::<MtState>();

    // 1. 缓存命中直接出
    let mut uncached: Vec<(usize, &TransJob)> = Vec::new();
    {
        let cache = st.cache.lock().unwrap();
        for (i, job) in batch.iter().enumerate() {
            if let Some(dst) = cache.get(&job.text) {
                emit_translation(app, job, dst);
            } else {
                uncached.push((i, job));
            }
        }
    }
    if uncached.is_empty() {
        batch.clear();
        return;
    }

    // 2. sidecar
    let port = match ensure_server(app) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[mt] {e}");
            app.state::<MtState>()
                .unavailable
                .store(true, Ordering::Relaxed);
            let _ = app.emit("mt://unavailable", e);
            batch.clear();
            return;
        }
    };

    // 3. 批量请求
    let lines: Vec<&str> = uncached.iter().map(|(_, j)| j.text.as_str()).collect();
    match translate_batch(port, &lines) {
        Ok(outs) if outs.len() == lines.len() => {
            let mut cache = st.cache.lock().unwrap();
            for ((i, job), dst) in uncached.into_iter().zip(outs.into_iter()) {
                let dst = dst.trim().to_string();
                cache.insert(job.text.clone(), dst.clone());
                emit_translation(app, job, &dst);
                let _ = i; // 顺序已对齐
            }
        }
        Ok(outs) => {
            // 行数不齐：退化为逐句翻译（MVP 简化，保证不出错行）
            eprintln!(
                "[mt] 批量输出行数不齐（{}/{}），退化逐句",
                outs.len(),
                lines.len()
            );
            for (_, job) in &uncached {
                if let Ok(dst) = translate_batch(port, &[&job.text]) {
                    if let Some(first) = dst.into_iter().next() {
                        let dst = first.trim().to_string();
                        st.cache.lock().unwrap().insert(job.text.clone(), dst.clone());
                        emit_translation(app, job, &dst);
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("[mt] 翻译请求失败: {e}");
            // 请求失败不清 batch：丢批次，后续句子重试
        }
    }

    batch.clear();
    save_cache(&st.cache.lock().unwrap(), cache_path);
}

fn emit_translation(app: &AppHandle, job: &TransJob, dst: &str) {
    if dst.is_empty() {
        return;
    }
    let _ = app.emit(
        "asr://translation",
        TranslationEvent {
            start: job.start,
            end: job.end,
            src: job.text.clone(),
            dst: dst.to_string(),
        },
    );
    eprintln!("[mt] 译文: {}", dst);
}

// ---------- llama-server sidecar ----------
fn find_model() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join(".dualsub/models");
    for name in ["translation.gguf", "translation-tiny.gguf"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// sidecar 端口（进程内单实例，拉起时定档）
static SERVER_PORT: Mutex<Option<u16>> = Mutex::new(None);

fn ensure_server(app: &AppHandle) -> Result<u16, String> {
    let st = app.state::<MtState>();
    if st.child.lock().unwrap().is_some() {
        return Ok(SERVER_PORT.lock().unwrap().unwrap_or(BASE_PORT));
    }

    let Some(model) = find_model() else {
        return Err(
            "未找到翻译模型：请把翻译 GGUF 放入 ~/.dualsub/models/translation.gguf（M4 将提供下载管理）"
                .into(),
        );
    };
    if !std::path::Path::new(LLAMA_SERVER).is_file() {
        return Err("未找到 llama-server（开发期请 brew install llama.cpp）".into());
    }

    let port = pick_port().ok_or("找不到可用端口")?;
    eprintln!("[mt] 拉起 llama-server :{port}（{}）", model.display());
    let child = std::process::Command::new(LLAMA_SERVER)
        .args([
            "-m",
            model.to_str().ok_or("模型路径非法")?,
            "--host",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "-c",
            "4096",
            "-t",
            "4",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("llama-server 启动失败: {e}"))?;
    *st.child.lock().unwrap() = Some(child);
    *SERVER_PORT.lock().unwrap() = Some(port);

    // 健康检查：最多 60s（模型首次加载可能较久）
    let health = format!("http://127.0.0.1:{port}/health");
    for _ in 0..200 {
        if let Ok(resp) = ureq::get(&health).timeout(Duration::from_millis(1500)).call() {
            if resp.status() == 200 {
                eprintln!("[mt] llama-server 就绪 :{port}");
                return Ok(port);
            }
        }
        // 若进程已死，快速失败
        if let Some(c) = st.child.lock().unwrap().as_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                *st.child.lock().unwrap() = None;
                return Err("llama-server 启动后退出（模型或参数问题）".into());
            }
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    Err("llama-server 健康检查超时".into())
}

const LLAMA_SERVER: &str = "/opt/homebrew/bin/llama-server";

fn pick_port() -> Option<u16> {
    for p in BASE_PORT..BASE_PORT + 10 {
        if TcpListener::bind(("127.0.0.1", p)).is_ok() {
            return Some(p);
        }
    }
    None
}

// ---------- 翻译请求 ----------
fn translate_batch(port: u16, lines: &[&str]) -> Result<Vec<String>, String> {
    let url = format!("http://127.0.0.1:{port}/v1/chat/completions");
    // 编号映射：小模型守不住「行数一致」，用编号对齐更稳
    let numbered: Vec<String> =
        lines.iter().enumerate().map(|(i, l)| format!("{}. {}", i + 1, l)).collect();
    let body = serde_json::json!({
        "messages": [
            {
                "role": "system",
                "content": "你是专业的字幕翻译引擎。用户会给出若干条带编号的字幕行。把每行字幕翻译成简体中文，输出时保留每行开头的编号数字并替换 N 为实际数字（例：输入「1. hello」输出「1. 你好」）。一行一条，不要解释、不要引号。"
            },
            { "role": "user", "content": numbered.join("\n") }
        ],
        "temperature": 0.2,
        "max_tokens": 1024,
        "stream": false
    });
    let resp = ureq::post(&url)
        .timeout(Duration::from_secs(180))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("HTTP 请求失败: {e}"))?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .ok_or("响应缺少 content")?
        .to_string();
    // 编号解析回原顺序；未编号行按顺序填空位（小模型兼容）
    let mut out: Vec<Option<String>> = vec![None; lines.len()];
    let mut unnumbered: Vec<String> = Vec::new();
    for l in content.lines() {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        // 识别 "12. " / "12、" / "12)" 前缀
        let mut split = l.splitn(2, |c: char| c == '.' || c == '、' || c == ')');
        let head = split.next().unwrap_or("").trim();
        let rest = split.next().unwrap_or("").trim();
        match head.parse::<usize>() {
            Ok(idx) if idx >= 1 && idx <= lines.len() && !rest.is_empty() => {
                out[idx - 1] = Some(rest.to_string());
            }
            _ => unnumbered.push(l.to_string()),
        }
    }
    for slot in out.iter_mut() {
        if slot.is_none() {
            *slot = unnumbered.pop();
        }
    }
    // 缺失的编号用原文兜底（宁出原文不出错行）
    let outs: Vec<String> = out
        .into_iter()
        .zip(lines.iter())
        .map(|(o, l)| o.unwrap_or_else(|| l.to_string()))
        .collect();
    Ok(outs)
}

/// 源文本已是目标语言（简体中文）时跳过翻译，省 sidecar 算力
pub fn is_mostly_chinese(text: &str) -> bool {
    let cjk = text.chars().filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c)).count();
    let total = text.chars().count().max(1);
    cjk * 100 / total > 30
}

// ---------- 缓存持久化 ----------
#[derive(Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: HashMap<String, String>,
}

fn cache_file() -> PathBuf {
    let mut p = std::env::temp_dir();
    // 方案：磁盘缓存跨会话有效——放 ~/.dualsub/，不随系统临时目录清理
    if let Ok(home) = std::env::var("HOME") {
        let dir = PathBuf::from(home).join(".dualsub");
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir.join("translation-cache.json");
        }
    }
    p.push("dualsub-translation-cache.json");
    p
}

fn save_cache(map: &HashMap<String, String>, path: &PathBuf) {
    let data = CacheFile {
        version: 1,
        entries: map.clone(),
    };
    if let Ok(s) = serde_json::to_string(&data) {
        let _ = std::fs::write(path, s);
    }
}
