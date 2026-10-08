//! spike-2：whisper-rs 实时识别基准（技术方案 §9 量化验收）
//!
//! 用法：spike_asr <ggml模型路径> <16k单声道wav路径>
//! 输出：模型加载耗时 / 推理墙钟 / 音频时长 / RTF（<1 即能追上实时）/ 识别样例
//! CPU 与内存用 /usr/bin/time -l 外部测量。
//!
//! 结论产出后：本文件的精神（wav 解析、参数档位）迁入 src-tauri/src/asr/。

use std::io::Read;
use std::time::Instant;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("用法: spike_asr <ggml模型> <16k mono wav>");
        std::process::exit(2);
    }
    let model_path = &args[1];
    let wav_path = &args[2];

    // 1. 读音频
    let samples = read_wav_16k_mono(wav_path)?;
    let dur_sec = samples.len() as f64 / 16000.0;
    eprintln!("[spike-2] 音频时长 {dur_sec:.1}s（{} 采样）", samples.len());

    // 2. 加载模型
    let t_load = Instant::now();
    let ctx = WhisperContext::new_with_params(model_path, WhisperContextParameters::default())?;
    eprintln!("[spike-2] 模型加载 {:.2}s", t_load.elapsed().as_secs_f64());

    // 3. 推理参数（对应方案 §4.1 幻觉治理初值）
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    // SPIKE_THREADS 可覆盖线程数（基准权衡用），默认 whisper 自动（=物理核数）
    if let Ok(n) = std::env::var("SPIKE_THREADS") {
        if let Ok(n) = n.parse::<i32>() {
            if n > 0 {
                params.set_n_threads(n);
                eprintln!("[spike-2] 线程数覆盖: {n}");
            }
        }
    }
    params.set_language(Some("zh"));
    params.set_print_progress(false);
    params.set_print_special(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_no_speech_thold(0.6); // 幻觉治理：no_speech 阈值
    params.set_suppress_blank(true);

    // 4. 整段转写（衡量「追得上实时」：RTF < 1）
    let mut state = ctx.create_state()?;
    let t_infer = Instant::now();
    state.full(params, &samples)?;
    let infer = t_infer.elapsed().as_secs_f64();

    // 5. 输出
    let n_seg = state.full_n_segments()?;
    let rtf = infer / dur_sec;
    let lag = dur_sec - infer;
    println!("=== 结果 ===");
    println!("音频时长:   {dur_sec:.1}s");
    println!("推理墙钟:   {infer:.2}s");
    println!("RTF:        {rtf:.3}  (<1 即可追实时; 全程识别落后 {lag:.1}s)");
    println!("分段数:     {n_seg}");
    println!("--- 前 8 段 ---");
    for i in 0..n_seg.min(8) {
        let t0 = state.full_get_segment_t0(i)? as f64 / 100.0;
        let t1 = state.full_get_segment_t1(i)? as f64 / 100.0;
        let text = state.full_get_segment_text(i)?;
        println!("[{t0:8.1} → {t1:8.1}] {text}");
    }
    Ok(())
}

/// 解析 16-bit PCM 单声道 WAV → f32[-1,1]（spike 从简，只支持 ffmpeg 产出的标准 PCM wav）
fn read_wav_16k_mono(path: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
    let mut buf = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut buf)?;

    if &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        return Err("不是 RIFF/WAVE".into());
    }
    let mut pos = 12usize;
    let (mut channels, mut rate, mut bits) = (0u16, 0u32, 0u16);
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size = u32::from_le_bytes(buf[pos + 4..pos + 8].try_into()?) as usize;
        let body = &buf[pos + 8..(pos + 8 + size).min(buf.len())];
        match id {
            b"fmt " => {
                channels = u16::from_le_bytes(body[2..4].try_into()?);
                rate = u32::from_le_bytes(body[4..8].try_into()?);
                bits = u16::from_le_bytes(body[14..16].try_into()?);
            }
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + size + (size & 1);
    }
    let data = data.ok_or("缺 data 块")?;
    if (channels, rate, bits) != (1, 16000, 16) {
        return Err(format!("期望 16k/mono/16bit，实际 {rate}Hz/{channels}ch/{bits}bit").into());
    }
    Ok(data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect())
}
