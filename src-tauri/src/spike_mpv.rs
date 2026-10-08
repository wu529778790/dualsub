//! spike-1（macOS）：libmpv 原生子窗口嵌入 + 透明 WebView 分层验证
//!
//! 验证目标（对应技术方案 §7 最高风险项）：
//! 1. WebView 透明背景，能看到垫在下面的 mpv 视频画面
//! 2. mpv NSView 垫在 WKWebView 之下（addSubview positioned:Below）
//! 3. WebView 盖满全窗口时点击事件落在 WebView（前端接管输入的拍板验证）
//! 4. mpv loadfile / pause 控制正常
//!
//! spike 从简：raw FFI 直连 libmpv，不引 mpv crate；验证通过后重构进 player/ 模块。

use std::ffi::{c_char, c_void, CString};
use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use tauri::{AppHandle, Manager};

// ---------- libmpv raw FFI（仅 spike 所需最小集） ----------
unsafe extern "C" {
    fn mpv_create() -> *mut c_void;
    fn mpv_initialize(ctx: *mut c_void) -> i32;
    fn mpv_set_option(
        ctx: *mut c_void,
        name: *const c_char,
        format: i64,
        data: *const c_void,
    ) -> i32;
    fn mpv_command(ctx: *mut c_void, args: *mut *const c_char) -> i32;
    fn mpv_set_property_string(
        ctx: *mut c_void,
        name: *const c_char,
        value: *const c_char,
    ) -> i32;
    #[allow(dead_code)]
    fn mpv_terminate_destroy(ctx: *mut c_void);
}

const MPV_FORMAT_INT64: i64 = 4;

// ---------- 全局 spike 状态 ----------
struct MpvPtr(*mut c_void);
// spike：mpv 指针只会在主线程（run_on_main_thread）访问，Send 仅为 Tauri 托管所需
unsafe impl Send for MpvPtr {}

#[derive(Default)]
pub struct SpikeState {
    mpv: Mutex<Option<MpvPtr>>,
    paused: Mutex<bool>,
}

fn err_str(code: i32, what: &str) -> String {
    format!("{what} 失败: mpv_error {code}")
}

// ---------- spike-1 主流程：建视图 → 挂 mpv → 播文件 ----------
pub fn spike_mpv_load(app: &AppHandle, path: String) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("找不到主窗口")?;

    app.run_on_main_thread(move || {
        if let Err(e) = do_load(&window, &path) {
            eprintln!("[spike-1] {e}");
        }
    })
    .map_err(|e| e.to_string())
}

fn do_load(window: &tauri::WebviewWindow, path: &str) -> Result<(), String> {
    // NSView 等一切 AppKit 操作必须在主线程
    let mtm = MainThreadMarker::new().ok_or("不在主线程")?;
    let _ = mtm;

    // 1. 拿 NSWindow 与 WKWebView（Tauri 提供）
    let ns_window_ptr = window.ns_window().map_err(|e| e.to_string())?;
    let webview_ptr = window.ns_view().map_err(|e| e.to_string())?;
    assert!(!ns_window_ptr.is_null() && !webview_ptr.is_null());

    // 2. 全局状态
    let state = window.app_handle().state::<SpikeState>();
    let mut mpv_slot = state.mpv.lock().unwrap();

    // 3. 首次调用：创建 mpv NSView 垫底 + 初始化 mpv
    if mpv_slot.is_none() {
        let mpv_view: Retained<NSView> = unsafe {
            let ns_window = &*(ns_window_ptr as *const objc2_app_kit::NSWindow);
            let webview = &*(webview_ptr as *const NSView);
            let content = ns_window.contentView().ok_or("拿不到 contentView")?;

            let view = NSView::initWithFrame(mtm.alloc(), content.bounds());
            // 跟随窗口缩放
            view.setAutoresizingMask(
                objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                    | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            // 关键一步：垫到 WKWebView 之下
            content.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::Below,
                Some(webview),
            );
            view
        };

        // mpv 实例：wid = NSView 指针（macOS 语义）
        let ctx = unsafe {
            let ctx = mpv_create();
            if ctx.is_null() {
                return Err("mpv_create 失败".into());
            }
            let wid_name = c"wid".to_bytes_with_nul();
            let wid: i64 = Retained::as_ptr(&mpv_view) as *const NSView as i64;
            let r = mpv_set_option(ctx, wid_name.as_ptr().cast(), MPV_FORMAT_INT64, (&wid) as *const i64 as *const c_void);
            if r < 0 {
                mpv_terminate_destroy(ctx);
                return Err(err_str(r, "set_option wid"));
            }
            let r = mpv_initialize(ctx);
            if r < 0 {
                mpv_terminate_destroy(ctx);
                return Err(err_str(r, "mpv_initialize"));
            }
            ctx
        };
        *mpv_slot = Some(MpvPtr(ctx));
        eprintln!("[spike-1] mpv 已初始化并绑定 NSView");
    }

    // 4. loadfile
    let ctx = mpv_slot.as_ref().unwrap().0;
    unsafe {
        let cpath = CString::new(path).map_err(|_| "路径含 \\0")?;
        let a_load = c"loadfile".to_bytes_with_nul();
        let a_replace = c"replace".to_bytes_with_nul();
        let mut args = [
            a_load.as_ptr().cast(),
            cpath.as_ptr(),
            a_replace.as_ptr().cast(),
            std::ptr::null(),
        ];
        let r = mpv_command(ctx, args.as_mut_ptr());
        if r < 0 {
            return Err(err_str(r, "loadfile"));
        }
    }
    eprintln!("[spike-1] loadfile: {path}");
    Ok(())
}

// ---------- 控制：播放/暂停 ----------
pub fn spike_mpv_toggle_pause(app: &AppHandle) -> Result<bool, String> {
    let window = app.get_webview_window("main").ok_or("找不到主窗口")?;
    let state = window.app_handle().state::<SpikeState>();
    let mpv_slot = state.mpv.lock().unwrap();
    let ctx = mpv_slot.as_ref().ok_or("mpv 尚未初始化")?.0;
    let mut paused = state.paused.lock().unwrap();
    *paused = !*paused;
    unsafe {
        let name = c"pause".to_bytes_with_nul();
        let value = if *paused {
            c"yes".to_bytes_with_nul()
        } else {
            c"no".to_bytes_with_nul()
        };
        let r = mpv_set_property_string(ctx, name.as_ptr().cast(), value.as_ptr().cast());
        if r < 0 {
            return Err(err_str(r, "set pause"));
        }
    }
    Ok(*paused)
}

// ---------- Tauri 命令 ----------
#[tauri::command]
pub fn cmd_mpv_load(app: AppHandle, path: String) -> Result<(), String> {
    spike_mpv_load(&app, path)
}

#[tauri::command]
pub fn cmd_mpv_toggle_pause(app: AppHandle) -> Result<bool, String> {
    spike_mpv_toggle_pause(&app)
}

/// 生成 30s 测试视频（彩条+正弦音），供 spike 验证
#[tauri::command]
pub fn cmd_gen_test_video() -> Result<String, String> {
    let dir = std::env::temp_dir().join("dualsub-spike");
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
