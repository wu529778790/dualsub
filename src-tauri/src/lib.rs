mod spike_mpv;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(spike_mpv::SpikeState::default())
        .invoke_handler(tauri::generate_handler![
            spike_mpv::cmd_mpv_load,
            spike_mpv::cmd_mpv_toggle_pause,
            spike_mpv::cmd_gen_test_video,
        ])
        .setup(|app| {
            // spike 自动化验证钩子：DUALSPIKE_AUTO_PLAY=/path/to.mp4 启动即加载
            if let Ok(path) = std::env::var("DUALSPIKE_AUTO_PLAY") {
                if !path.is_empty() {
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(800));
                        if let Err(e) = spike_mpv::spike_mpv_load(&handle, path) {
                            eprintln!("[spike-1] 自动加载失败: {e}");
                        }
                    });
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("DualSub 启动失败");
}
