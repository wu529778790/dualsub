//! M2 实时识别管线（技术方案 §4.1，项目灵魂模块）
//!
//! 架构：
//! - 单 worker 线程持有 whisper 上下文（模型只加载一次，0.14s）
//! - ffmpeg 常驻进程管道输出 16k mono f32（方案拍板：常驻管道为主，不逐段 spawn）
//!   `-ss` 置于 `-i` 前 + accurate seek：时间戳以 ffmpeg 输出为基准，不用播放时钟累加
//! - 能量 VAD（M2.0 简化）：整窗静音直接跳过不进 whisper；silero-vad(ort) 留 M2.1 替换
//! - 调度：识别是软实时——领先播放 >30s 就等；seek/换片杀管道重启会话
//! - 幂等：会话内窗口顺序唯一；seek 即换会话重置，不重复识别

use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const SAMPLE_RATE: usize = 16000;
/// 识别窗口 12s（方案 §4.1：单段上限 ~15s 内）
const WINDOW_SAMPLES: usize = SAMPLE_RATE * 12;
/// 能量 VAD：100ms 帧的 RMS 门限（TTS/人声典型 0.02+，静音 <0.003）
const VAD_RMS_GATE: f32 = 0.008;
/// 静音窗判定：有声帧占比低于 2% 直接跳过 whisper
const VAD_MIN_SPEECH_RATIO: f32 = 0.02;
/// 追帧不追播：识别位置领先播放超过该值就等待
const AHEAD_LIMIT_SEC: f64 = 30.0;

// ---------- 全局状态 ----------
pub struct AsrState {
    tx: Mutex<Option<Sender<Msg>>>,
    current_path: Mutex<String>,
    /// 播放位置（毫秒），player 事件线程持续更新
    playback_pos_ms: AtomicU64,
}

impl Default for AsrState {
    fn default() -> Self {
        Self {
            tx: Mutex::new(None),
            current_path: Mutex::new(String::new()),
            playback_pos_ms: AtomicU64::new(0),
        }
    }
}

enum Msg {
    Start { path: String, pos: f64 },
    Seek { pos: f64 },
    /// 预留：优雅退出 worker（当前进程退出即回收）
    #[allow(dead_code)]
    Shutdown,
}

/// 会话内轮询消息的结果
enum Poll {
    None,
    /// 重启会话（新位置，可选换文件）
    Restart { pos: f64, path: Option<String> },
    Stop,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SubtitleEvent {
    start: f64,
    end: f64,
    text: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    start: f64,
    end: f64,
}

// ---------- 对 player 的接口 ----------
pub fn set_playback_pos(app: &AppHandle, t: f64) {
    let st = app.state::<AsrState>();
    st.playback_pos_ms
        .store((t * 1000.0).max(0.0) as u64, Ordering::Relaxed);
}

pub fn on_video_start(app: &AppHandle, path: String) {
    {
        let st = app.state::<AsrState>();
        *st.current_path.lock().unwrap() = path.clone();
        st.playback_pos_ms.store(0, Ordering::Relaxed);
    }
    send(app, Msg::Start { path, pos: 0.0 });
}

pub fn on_seek(app: &AppHandle, pos: f64) {
    send(app, Msg::Seek { pos });
}

fn send(app: &AppHandle, msg: Msg) {
    let st = app.state::<AsrState>();
    let mut tx_slot = st.tx.lock().unwrap();
    if tx_slot.is_none() {
        let (tx, rx) = mpsc::channel::<Msg>();
        let app2 = app.clone();
        std::thread::spawn(move || worker(app2, rx));
        *tx_slot = Some(tx);
    }
    let _ = tx_slot.as_ref().unwrap().send(msg);
}

// ---------- worker ----------
fn worker(app: AppHandle, rx: Receiver<Msg>) {
    let Some(model_path) = find_model() else {
        let _ = app.emit(
            "asr://error",
            "未找到 whisper 模型：请把 ggml-small.bin 放入 ~/.dualsub/models/（M4 将提供下载管理）",
        );
        eprintln!("[asr] 模型缺失，识别管线不启动");
        return;
    };
    let t0 = Instant::now();
    let ctx = match WhisperContext::new_with_params(&model_path.to_string_lossy(), WhisperContextParameters::default())
    {
        Ok(c) => c,
        Err(e) => {
            let _ = app.emit("asr://error", format!("whisper 模型加载失败: {e}"));
            return;
        }
    };
    eprintln!(
        "[asr] 模型加载完成 {:.2}s（{}）",
        t0.elapsed().as_secs_f64(),
        model_path.display()
    );

    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Shutdown => break,
            Msg::Start { path, pos } => run_session(&app, &ctx, &rx, &path, pos),
            Msg::Seek { pos } => {
                let path = app.state::<AsrState>().current_path.lock().unwrap().clone();
                if !path.is_empty() {
                    run_session(&app, &ctx, &rx, &path, pos);
                }
            }
        }
    }
}

fn find_model() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let tier = crate::settings::read("whisper_model").unwrap_or_else(|| "small".into());
    let file = match tier.as_str() {
        "base" => "ggml-base.bin",
        _ => "ggml-small.bin",
    };
    let p = PathBuf::from(home).join(format!(".dualsub/models/{file}"));
    p.is_file().then_some(p)
}

// ---------- 会话：从 pos 起读 ffmpeg 管道，遇 seek/换片重启 ----------
fn run_session(
    app: &AppHandle,
    ctx: &WhisperContext,
    rx: &Receiver<Msg>,
    path: &str,
    start_pos: f64,
) {
    let mut current_path = path.to_string();
    let mut pos = start_pos;

    'session: loop {
        eprintln!("[asr] 会话开始：{current_path} 自 {pos:.1}s");
        let mut child = match spawn_ffmpeg(&current_path, pos) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[asr] {e}");
                let _ = app.emit("asr://error", e);
                return;
            }
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            return;
        };

        let mut window_idx: u64 = 0;
        let mut byte_buf = vec![0u8; WINDOW_SAMPLES * 4];
        let mut samples = vec![0f32; WINDOW_SAMPLES];
        let mut last_text = String::new();
        let mut repeat_count = 0usize;
        // 语言：首窗自动检测，之后钉住（方案 §4.2 源语言自动猜）
        let mut detected_lang: Option<String> = None;

        loop {
            // 窗口间轮询：seek / 换片 / 关停
            match poll_rx(rx) {
                Poll::Stop => {
                    let _ = child.kill();
                    return;
                }
                Poll::Restart { pos: p, path: np } => {
                    let _ = child.kill();
                    eprintln!("[asr] 识别中收到 seek/换片 → 重启会话 @{p:.1}s");
                    pos = p;
                    if let Some(np) = np {
                        current_path = np;
                    }
                    continue 'session;
                }
                Poll::None => {}
            }

            // 整窗读满（EOF 提前退出）
            match read_exact_or_eof(&mut stdout, &mut byte_buf) {
                ReadOutcome::Full => {}
                ReadOutcome::Eof => {
                    eprintln!("[asr] 音频读到结尾，会话结束（共 {window_idx} 窗）");
                    let _ = child.kill();
                    return;
                }
                ReadOutcome::Error => {
                    let _ = child.kill();
                    return;
                }
            }
            for (i, chunk) in byte_buf.chunks_exact(4).enumerate() {
                samples[i] = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }

            let window_start =
                pos + (window_idx as f64) * (WINDOW_SAMPLES as f64 / SAMPLE_RATE as f64);
            let window_end = window_start + WINDOW_SAMPLES as f64 / SAMPLE_RATE as f64;

            // 追帧不追播：领先播放太多就等（等的同时持续轮询 seek）
            loop {
                let play = app.state::<AsrState>().playback_pos_ms.load(Ordering::Relaxed) as f64
                    / 1000.0;
                if window_start <= play + AHEAD_LIMIT_SEC {
                    break;
                }
                match poll_rx(rx) {
                    Poll::Stop => {
                        let _ = child.kill();
                        return;
                    }
                    Poll::Restart { pos: p, path: np } => {
                        let _ = child.kill();
                        eprintln!("[asr] 等待中收到 seek/换片 → 重启会话 @{p:.1}s");
                        pos = p;
                        if let Some(np) = np {
                            current_path = np;
                        }
                        continue 'session;
                    }
                    Poll::None => {}
                }
                std::thread::sleep(Duration::from_millis(200));
            }

            // 能量 VAD：静音窗直接跳过（省 30–50% 算力的第一道闸）
            let _ = app.emit(
                "asr://progress",
                ProgressEvent { start: window_start, end: window_end },
            );
            let speech_ratio = energy_vad(&samples);
            if speech_ratio < VAD_MIN_SPEECH_RATIO {
                window_idx += 1;
                continue;
            }

            // whisper 推理
            let t_infer = Instant::now();
            let mut state = match ctx.create_state() {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[asr] create_state 失败: {e}");
                    let _ = child.kill();
                    return;
                }
            };
            if let Err(e) = state.full(make_params(detected_lang.as_deref()), &samples) {
                eprintln!("[asr] 推理失败: {e}");
                window_idx += 1;
                continue;
            }
            // 首窗后钉住检测到的语言
            if detected_lang.is_none() {
                if let Ok(id) = state.full_lang_id_from_state() {
                    if let Some(code) = whisper_rs::get_lang_str(id) {
                        eprintln!("[asr] 检测语言: {code}");
                        detected_lang = Some(code.to_string());
                    }
                }
            }
            eprintln!(
                "[asr] 窗 {window_idx} [{window_start:.1}-{window_end:.1}] 推理 {:.2}s（语音占比 {:.0}%)",
                t_infer.elapsed().as_secs_f64(),
                speech_ratio * 100.0
            );

            // 段落事件 + 幻觉治理
            let n_seg = state.full_n_segments().unwrap_or(0);
            for i in 0..n_seg {
                let text = state
                    .full_get_segment_text(i)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if text.is_empty() {
                    continue;
                }
                // TODO(M2.1): whisper-rs 0.14 未暴露段落级 no_speech_prob；
                // 静音段已被 VAD 硬跳过，重复幻觉由下方去重兜底
                // 连续重复行去重（spike-2 发现的幻觉形态）
                if text == last_text {
                    repeat_count += 1;
                    if repeat_count >= 3 {
                        continue;
                    }
                } else {
                    last_text = text.clone();
                    repeat_count = 0;
                }
                let seg_start =
                    window_start + state.full_get_segment_t0(i).unwrap_or(0) as f64 / 100.0;
                let seg_end =
                    window_start + state.full_get_segment_t1(i).unwrap_or(0) as f64 / 100.0;
                eprintln!("[asr] 字幕 [{seg_start:.1}-{seg_end:.1}] {text}");
                let _ = app.emit(
                    "asr://subtitle",
                    SubtitleEvent { start: seg_start, end: seg_end, text: text.clone() },
                );
                // 进翻译队列（M3：攒批 + 缓存 + llama-server）；已是中文的跳过
                if !crate::mt::is_mostly_chinese(&text) {
                    crate::mt::enqueue(app, seg_start, seg_end, &text);
                }
            }
            window_idx += 1;
        }
    }
}

fn make_params<'a>(lang: Option<&'a str>) -> FullParams<'a, 'a> {
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    match lang {
        Some(l) => {
            params.set_language(Some(l));
            // 简体引导只在确认是中文时加（日/英内容加中文提示词会把输出带歪）
            if l == "zh" {
                params.set_initial_prompt("以下是普通话的句子。");
            }
        }
        None => {
            // 首窗：自动检测
            params.set_language(None);
            params.set_detect_language(true);
        }
    }
    params.set_no_speech_thold(0.6);
    params.set_suppress_blank(true);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_print_special(false);
    params
}

/// 排空消息队列，最后一条生效
fn poll_rx(rx: &Receiver<Msg>) -> Poll {
    let mut result = Poll::None;
    loop {
        match rx.try_recv() {
            Ok(Msg::Shutdown) => return Poll::Stop,
            Ok(Msg::Seek { pos }) => {
                result = Poll::Restart { pos, path: None };
            }
            Ok(Msg::Start { path, pos }) => {
                result = Poll::Restart { pos, path: Some(path) };
            }
            Err(TryRecvError::Empty) => return result,
            Err(TryRecvError::Disconnected) => return Poll::Stop,
        }
    }
}

fn spawn_ffmpeg(path: &str, pos: f64) -> Result<std::process::Child, String> {
    std::process::Command::new(ffmpeg_path())
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{pos:.3}"),
            "-i",
            path,
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("ffmpeg 启动失败: {e}"))
}

/// 能量 VAD：返回有声帧占比
fn energy_vad(samples: &[f32]) -> f32 {
    const FRAME: usize = SAMPLE_RATE / 10; // 100ms
    let mut speech_frames = 0usize;
    let mut total = 0usize;
    for frame in samples.chunks(FRAME) {
        let rms = (frame.iter().map(|x| x * x).sum::<f32>() / frame.len() as f32).sqrt();
        if rms > VAD_RMS_GATE {
            speech_frames += 1;
        }
        total += 1;
    }
    if total == 0 { 0.0 } else { speech_frames as f32 / total as f32 }
}

enum ReadOutcome {
    Full,
    Eof,
    Error,
}

/// 读满缓冲；EOF 返回 Eof（不足半窗按 EOF，够半窗按满窗处理）
fn read_exact_or_eof(r: &mut impl Read, byte_buf: &mut [u8]) -> ReadOutcome {
    let mut filled = 0usize;
    while filled < byte_buf.len() {
        match r.read(&mut byte_buf[filled..]) {
            Ok(0) => {
                return if filled >= byte_buf.len() / 2 {
                    ReadOutcome::Full
                } else {
                    ReadOutcome::Eof
                };
            }
            Ok(n) => filled += n,
            Err(_) => return ReadOutcome::Error,
        }
    }
    ReadOutcome::Full
}

fn ffmpeg_path() -> &'static str {
    if PathBuf::from("/opt/homebrew/bin/ffmpeg").is_file() {
        "/opt/homebrew/bin/ffmpeg"
    } else {
        "ffmpeg"
    }
}
