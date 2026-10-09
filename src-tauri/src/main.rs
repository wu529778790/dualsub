// 只在 release 中隐藏 macOS/Windows 的控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    livesub_player_lib::run()
}
