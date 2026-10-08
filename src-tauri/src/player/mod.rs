//! M1 播放内核（平台分发）
//!
//! - macOS：libmpv 进程内 + NSView 垫底（spike-1 已验证）
//! - Windows/Linux：M1-Windows 里程碑接入（子 HWND / X11 subwindow）；
//!   当前提供 stub 命令保证跨平台可编译

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(not(target_os = "macos"))]
mod stub;
#[cfg(not(target_os = "macos"))]
pub use stub::*;
