mod asr;
mod models;
mod mt;
mod player;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(player::PlayerState::default())
        .manage(asr::AsrState::default())
        .manage(mt::MtState::default())
        .manage(models::ModelsState::default())
        .invoke_handler(tauri::generate_handler![
            player::cmd_player_load,
            player::cmd_player_toggle_pause,
            player::cmd_player_seek,
            player::cmd_player_set_volume,
            player::cmd_player_set_speed,
            player::cmd_player_toggle_fullscreen,
            player::cmd_player_get_state,
            player::cmd_gen_test_video,
            mt::cmd_mt_set_engine,
            mt::cmd_mt_get_engine,
            models::cmd_model_download,
        ])
        .setup(|app| {
            // 自动化验证钩子：DUALSUB_AUTO_PLAY=/path/to.mp4 启动即加载
            if let Ok(path) = std::env::var("DUALSUB_AUTO_PLAY") {
                if !path.is_empty() {
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(800));
                        if let Err(e) = player::player_load(&handle, path) {
                            eprintln!("[player] 自动加载失败: {e}");
                        }
                    });
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("DualSub 构建失败")
        .run(|app_handle, event| {
            // 方案 §4.4：退出即干净——收割 llama-server sidecar
            if let tauri::RunEvent::Exit = event {
                mt::shutdown(app_handle);
            }
        });
}
