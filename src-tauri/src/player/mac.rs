//! M1 播放内核：libmpv 进程内集成（macOS：NSView 垫底 + wid 绑定）
//!
//! 架构（技术方案 §4）：
//! - 主 handle（mpv_create）：命令/属性写入
//! - 事件 handle（mpv_create_client）：专用线程 drain 事件，观察 time-pos/duration/pause
//!   推给前端（tauri emit），FILE_LOADED 时补挂外挂字幕
//! - whisper 进程内、llama-server 由 mt 模块管理，此处不管子进程

use std::ffi::{c_char, c_double, c_int, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

// ---------- libmpv FFI ----------
unsafe extern "C" {
    fn mpv_create() -> *mut c_void;
    fn mpv_initialize(ctx: *mut c_void) -> i32;
    fn mpv_set_option(
        ctx: *mut c_void,
        name: *const c_char,
        format: c_int,
        data: *const c_void,
    ) -> i32;
    fn mpv_set_property(
        ctx: *mut c_void,
        name: *const c_char,
        format: c_int,
        data: *const c_void,
    ) -> i32;
    fn mpv_get_property(
        ctx: *mut c_void,
        name: *const c_char,
        format: c_int,
        data: *mut c_void,
    ) -> i32;
    fn mpv_command(ctx: *mut c_void, args: *mut *const c_char) -> i32;
    fn mpv_observe_property(
        ctx: *mut c_void,
        reply_userdata: u64,
        name: *const c_char,
        format: c_int,
    ) -> i32;
    fn mpv_wait_event(ctx: *mut c_void, timeout: c_double) -> *mut mpv_event;
    fn mpv_create_client(ctx: *mut c_void, name: *const c_char) -> *mut c_void;
    #[allow(dead_code)]
    fn mpv_terminate_destroy(ctx: *mut c_void);
}

#[repr(C)]
struct mpv_event_property {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct mpv_event {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

const MPV_FORMAT_INT64: c_int = 4;
const MPV_FORMAT_FLAG: c_int = 3;
const MPV_FORMAT_DOUBLE: c_int = 5;

const MPV_EVENT_SHUTDOWN: c_int = 1;
const MPV_EVENT_FILE_LOADED: c_int = 8;
const MPV_EVENT_SEEK: c_int = 20;
const MPV_EVENT_PROPERTY_CHANGE: c_int = 22;

// ---------- 全局状态 ----------
struct MpvPtr(*mut c_void);
// mpv 句柄线程安全（命令/属性）；事件只由事件线程 drain
unsafe impl Send for MpvPtr {}
unsafe impl Sync for MpvPtr {}

#[derive(Default)]
pub struct PlayerState {
    mpv: Mutex<Option<MpvPtr>>,
    /// FILE_LOADED 时由事件线程补挂的外挂字幕（已转 UTF-8）
    pending_subs: Mutex<Vec<PathBuf>>,
}

/// 推给前端的播放状态（属性变化即推）
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEvent {
    time_pos: Option<f64>,
    duration: Option<f64>,
    paused: Option<bool>,
}

fn mpv_err(code: i32, what: &str) -> String {
    format!("{what} 失败: mpv_error {code}")
}

unsafe fn cmd(ctx: *mut c_void, args: &[&CStr]) -> Result<(), String> {
    let mut ptrs: Vec<*const c_char> = args.iter().map(|a| a.as_ptr()).collect();
    ptrs.push(std::ptr::null());
    let r = mpv_command(ctx, ptrs.as_mut_ptr());
    if r < 0 {
        return Err(mpv_err(r, &format!("mpv_command {}", args[0].to_string_lossy())));
    }
    Ok(())
}

unsafe fn set_f64(ctx: *mut c_void, name: &CStr, v: f64) -> Result<(), String> {
    let r = mpv_set_property(ctx, name.as_ptr(), MPV_FORMAT_DOUBLE, (&v as *const f64).cast());
    if r < 0 {
        return Err(mpv_err(r, &format!("set {}", name.to_string_lossy())));
    }
    Ok(())
}

unsafe fn get_f64(ctx: *mut c_void, name: &CStr) -> Option<f64> {
    let mut out: f64 = 0.0;
    if mpv_get_property(ctx, name.as_ptr(), MPV_FORMAT_DOUBLE, (&mut out as *mut f64).cast()) >= 0 {
        Some(out)
    } else {
        None
    }
}

unsafe fn get_flag(ctx: *mut c_void, name: &CStr) -> Option<bool> {
    let mut out: c_int = 0;
    if mpv_get_property(ctx, name.as_ptr(), MPV_FORMAT_FLAG, (&mut out as *mut c_int).cast()) >= 0 {
        Some(out != 0)
    } else {
        None
    }
}

// ---------- 子窗口 + mpv 初始化（主线程） ----------
fn ensure_mpv(window: &tauri::WebviewWindow) -> Result<*mut c_void, String> {
    let state = window.app_handle().state::<PlayerState>();
    let mut slot = state.mpv.lock().unwrap();
    if let Some(p) = slot.as_ref() {
        return Ok(p.0);
    }

    let mpv_view: Retained<NSView> = unsafe {
        let mtm = MainThreadMarker::new().ok_or("不在主线程")?;
        let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())?;
        let webview_ptr = window.ns_view().map_err(|e| e.to_string())?;
        let ns_window = &*(ns_window_ptr as *const objc2_app_kit::NSWindow);
        let webview = &*(webview_ptr as *const NSView);
        let content = ns_window.contentView().ok_or("拿不到 contentView")?;

        let view = NSView::initWithFrame(mtm.alloc(), content.bounds());
        view.setAutoresizingMask(
            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // 垫到 WKWebView 之下（spike-1 已验证）
        content.addSubview_positioned_relativeTo(
            &view,
            NSWindowOrderingMode::Below,
            Some(webview),
        );
        view
    };

    let ctx = unsafe {
        let ctx = mpv_create();
        if ctx.is_null() {
            return Err("mpv_create 失败".into());
        }
        let wid: i64 = Retained::as_ptr(&mpv_view) as *const NSView as i64;
        let r = mpv_set_option(
            ctx,
            c"wid".as_ptr(),
            MPV_FORMAT_INT64,
            (&wid as *const i64).cast(),
        );
        if r < 0 {
            mpv_terminate_destroy(ctx);
            return Err(mpv_err(r, "set_option wid"));
        }
        let r = mpv_initialize(ctx);
        if r < 0 {
            mpv_terminate_destroy(ctx);
            return Err(mpv_err(r, "mpv_initialize"));
        }
        ctx
    };

    *slot = Some(MpvPtr(ctx));
    Ok(ctx)
}

// ---------- 事件线程 ----------
fn spawn_event_thread(app: AppHandle, client: MpvPtr) {
    // 注意：闭包体内不得出现 client.0（edition 2021 字段级捕获会绕过 Send 包装）
    std::thread::spawn(move || unsafe { event_loop(app, client) });
}

unsafe fn event_loop(app: AppHandle, client: MpvPtr) {
    let client = client.0;
    unsafe {
        loop {
            let ev = mpv_wait_event(client, 10.0);
            if ev.is_null() {
                continue;
            }
            match (*ev).event_id {
                MPV_EVENT_SHUTDOWN => break,
                MPV_EVENT_FILE_LOADED => {
                    // 补挂嗅探到的外挂字幕（mpv sub 轨渲染，保真且省事）
                    let subs: Vec<PathBuf> = {
                        let st = app.state::<PlayerState>();
                        let mut guard = st.pending_subs.lock().unwrap();
                        std::mem::take(&mut *guard)
                    };
                    for sub in subs {
                        let c_sub = CString::new(sub.to_string_lossy().as_bytes()).unwrap();
                        if cmd(client, &[c"sub-add", c_sub.as_c_str(), c"auto"]).is_err() {
                            eprintln!("[player] sub-add 失败: {}", sub.display());
                        } else {
                            eprintln!("[player] 外挂字幕已挂: {}", sub.display());
                        }
                    }
                }
                MPV_EVENT_PROPERTY_CHANGE => {
                    let prop = (*ev).data as *mut mpv_event_property;
                    if prop.is_null() || (*prop).name.is_null() {
                        continue;
                    }
                    let name = CStr::from_ptr((*prop).name).to_string_lossy().to_string();
                    let payload = match name.as_str() {
                        "time-pos" => {
                            let t = read_prop_f64(prop);
                            if let Some(t) = t {
                                crate::asr::set_playback_pos(&app, t);
                            }
                            PlayerEvent {
                                time_pos: t,
                                duration: None,
                                paused: None,
                            }
                        }
                        "duration" => PlayerEvent {
                            time_pos: None,
                            duration: read_prop_f64(prop),
                            paused: None,
                        },
                        "pause" => PlayerEvent {
                            time_pos: None,
                            duration: None,
                            paused: read_prop_flag(prop),
                        },
                        _ => continue,
                    };
                    let _ = app.emit("player://state", payload);
                }
                MPV_EVENT_SEEK => {
                    // seek → 识别管线从新位置重启（时间戳以 ffmpeg 输出为基准）
                    let t = get_f64(client, c"time-pos").unwrap_or(0.0);
                    crate::asr::set_playback_pos(&app, t);
                    crate::asr::on_seek(&app, t);
                }
                _ => {}
            }
        }
    }
}

unsafe fn read_prop_f64(prop: *mut mpv_event_property) -> Option<f64> {
    if (*prop).format == MPV_FORMAT_DOUBLE && !(*prop).data.is_null() {
        Some(*((*prop).data as *const f64))
    } else {
        None
    }
}

unsafe fn read_prop_flag(prop: *mut mpv_event_property) -> Option<bool> {
    if (*prop).format == MPV_FORMAT_FLAG && !(*prop).data.is_null() {
        Some(*((*prop).data as *const c_int) != 0)
    } else {
        None
    }
}

// ---------- 外挂字幕嗅探 + GBK→UTF-8 ----------
fn sniff_subs(video: &Path) -> Vec<PathBuf> {
    const SUB_EXTS: [&str; 4] = ["srt", "ass", "ssa", "vtt"];
    let mut out = Vec::new();
    let Some(dir) = video.parent() else { return out };
    let Some(stem) = video.file_stem().and_then(|s| s.to_str()) else {
        return out;
    };
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        let p = e.path();
        let Some(ext) = p.extension().and_then(|x| x.to_str()) else { continue };
        if !SUB_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
            continue;
        }
        let Some(sub_stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        // 同名（ep01.srt）或带语言后缀（ep01.chs.ass）
        if sub_stem == stem || sub_stem.starts_with(&format!("{stem}.")) {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// 非合法 UTF-8 的字幕按 GB18030（GBK 超集）转 UTF-8 临时文件
fn ensure_utf8(p: &Path) -> PathBuf {
    let Ok(bytes) = std::fs::read(p) else { return p.to_path_buf() };
    if std::str::from_utf8(&bytes).is_ok() {
        return p.to_path_buf();
    }
    let (decoded, _, had_errors) = encoding_rs::GB18030.decode(&bytes);
    if !had_errors {
        let dir = std::env::temp_dir().join("livesub-subs");
        let _ = std::fs::create_dir_all(&dir);
        let name = format!(
            "{}.utf8.{}",
            p.file_stem().and_then(|s| s.to_str()).unwrap_or("sub"),
            p.extension().and_then(|s| s.to_str()).unwrap_or("srt")
        );
        let out = dir.join(name);
        if std::fs::write(&out, decoded.as_bytes()).is_ok() {
            eprintln!("[player] 字幕 GBK→UTF-8: {}", p.display());
            return out;
        }
    }
    p.to_path_buf()
}

// ---------- 加载 ----------
pub fn player_load(app: &AppHandle, path: String) -> Result<(), String> {
    let video = PathBuf::from(&path);
    if !video.is_file() {
        return Err(format!("文件不存在: {path}"));
    }

    // 字幕先嗅探转码
    let subs: Vec<PathBuf> = sniff_subs(&video).iter().map(|p| ensure_utf8(p)).collect();

    let app_h = app.clone();
    app.run_on_main_thread(move || {
        let app = &app_h;
        let window = match app.get_webview_window("main") {
            Some(w) => w,
            None => return,
        };
        let ctx = match ensure_mpv(&window) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[player] {e}");
                return;
            }
        };

        // 事件线程 + 属性观察只挂一次（进程生命周期内只有一个 mpv 实例）
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| unsafe {
            let client_name = c"livesub-events";
            let client = mpv_create_client(ctx, client_name.as_ptr());
            if client.is_null() {
                eprintln!("[player] 事件 client 创建失败");
            } else {
                spawn_event_thread(app.clone(), MpvPtr(client));
            }
            for (name, fmt) in [
                (c"time-pos", MPV_FORMAT_DOUBLE),
                (c"duration", MPV_FORMAT_DOUBLE),
                (c"pause", MPV_FORMAT_FLAG),
            ] {
                mpv_observe_property(ctx, 0, name.as_ptr(), fmt);
            }
        });

        {
            let st = app.state::<PlayerState>();
            *st.pending_subs.lock().unwrap() = subs;
        }

        unsafe {
            let cpath = CString::new(path.as_str()).unwrap();
            if cmd(ctx, &[c"loadfile", cpath.as_c_str(), c"replace"]).is_err() {
                return;
            }
        }
        eprintln!("[player] loadfile: {path}");

        // 实时识别管线启动（模型缺失时只发事件，不影响播放）
        crate::asr::on_video_start(app, path.clone());
    })
    .map_err(|e| e.to_string())
}

// ---------- 控制命令 ----------
pub fn player_toggle_pause(app: &AppHandle) -> Result<bool, String> {
    let ctx = current_mpv(app)?;
    unsafe {
        let next = !get_flag(ctx, c"pause").unwrap_or(false);
        let flag: c_int = next as c_int;
        let r = mpv_set_property(
            ctx,
            c"pause".as_ptr(),
            MPV_FORMAT_FLAG,
            (&flag as *const c_int).cast(),
        );
        if r < 0 {
            return Err(mpv_err(r, "set pause"));
        }
        Ok(next)
    }
}

pub fn player_seek(app: &AppHandle, target: f64, absolute: bool) -> Result<(), String> {
    let ctx = current_mpv(app)?;
    unsafe {
        let secs = CString::new(format!("{target:.3}")).unwrap();
        let mode: &CStr = if absolute { c"absolute" } else { c"relative" };
        cmd(ctx, &[c"seek", secs.as_c_str(), mode])
    }
}

pub fn player_set_volume(app: &AppHandle, volume: f64) -> Result<(), String> {
    let ctx = current_mpv(app)?;
    unsafe { set_f64(ctx, c"volume", volume.clamp(0.0, 100.0)) }
}

pub fn player_set_speed(app: &AppHandle, speed: f64) -> Result<(), String> {
    let ctx = current_mpv(app)?;
    unsafe { set_f64(ctx, c"speed", speed.clamp(0.25, 4.0)) }
}

pub fn player_toggle_fullscreen(app: &AppHandle) -> Result<bool, String> {
    let window = app.get_webview_window("main").ok_or("找不到主窗口")?;
    let next = !window.is_fullscreen().unwrap_or(false);
    window.set_fullscreen(next).map_err(|e| e.to_string())?;
    Ok(next)
}

pub fn player_get_state(app: &AppHandle) -> Result<PlayerEvent, String> {
    let ctx = current_mpv(app)?;
    unsafe {
        Ok(PlayerEvent {
            time_pos: get_f64(ctx, c"time-pos"),
            duration: get_f64(ctx, c"duration"),
            paused: get_flag(ctx, c"pause"),
        })
    }
}

fn current_mpv(app: &AppHandle) -> Result<*mut c_void, String> {
    let st = app.state::<PlayerState>();
    let guard = st.mpv.lock().unwrap();
    match guard.as_ref() {
        Some(p) => Ok(p.0),
        None => Err("播放器未初始化".into()),
    }
}

// ---------- Tauri 命令 ----------
#[tauri::command]
pub fn cmd_player_load(app: AppHandle, path: String) -> Result<(), String> {
    player_load(&app, path)
}

#[tauri::command]
pub fn cmd_player_toggle_pause(app: AppHandle) -> Result<bool, String> {
    player_toggle_pause(&app)
}

#[tauri::command]
pub fn cmd_player_seek(app: AppHandle, target: f64, absolute: bool) -> Result<(), String> {
    player_seek(&app, target, absolute)
}

#[tauri::command]
pub fn cmd_player_set_volume(app: AppHandle, volume: f64) -> Result<(), String> {
    player_set_volume(&app, volume)
}

#[tauri::command]
pub fn cmd_player_set_speed(app: AppHandle, speed: f64) -> Result<(), String> {
    player_set_speed(&app, speed)
}

#[tauri::command]
pub fn cmd_player_toggle_fullscreen(app: AppHandle) -> Result<bool, String> {
    player_toggle_fullscreen(&app)
}

#[tauri::command]
pub fn cmd_player_get_state(app: AppHandle) -> Result<PlayerEvent, String> {
    player_get_state(&app)
}

/// 生成 30s 测试视频（彩条+正弦音），开发用
#[tauri::command]
pub fn cmd_gen_test_video() -> Result<String, String> {
    let dir = std::env::temp_dir().join("livesub-spike");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let out = dir.join("test.mp4");
    if !out.exists() {
        let st = std::process::Command::new("/opt/homebrew/bin/ffmpeg")
            .args([
                "-y",
                "-f", "lavfi", "-i", "testsrc2=duration=30:size=1280x720:rate=30",
                "-f", "lavfi", "-i", "sine=frequency=440:duration=30",
                "-c:v", "libx264",
                "-pix_fmt", "yuv420p",
                "-c:a", "aac",
                out.to_str().ok_or("路径非法")?,
            ])
            .output()
            .map_err(|e| format!("ffmpeg 启动失败: {e}"))?;
        if !st.status.success() {
            return Err(format!(
                "ffmpeg 失败: {}",
                String::from_utf8_lossy(&st.stderr)
            ));
        }
    }
    Ok(out.to_string_lossy().to_string())
}
