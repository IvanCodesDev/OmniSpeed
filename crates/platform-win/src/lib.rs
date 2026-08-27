//! platform-win —— OmniSpeed 的 Win32 平台封装层（开发文档 §5.1 platform/win）。
//!
//! 项目里所有 unsafe Win32 调用集中在本 crate 内部，对外只暴露安全 API：
//! - 前台窗口监听/查询（§7.1：SetWinEventHook 事件驱动，避免轮询）；
//! - 模拟按键（§7.3 播放器控制通道的"模拟按键兜底"：SendInput 发送播放器自身快捷键）；
//! - 窗口查找与 WM_COMMAND / WM_USER 控制消息（§7.3 PotPlayer/MPC-HC 的 IPC 通道）。
//!
//! v1 仅实现 Windows；跨平台抽象（PlatformInput trait）按 §4.3 预留给上层。

mod error;
mod input;
mod watcher;
mod window;

pub use error::Error;
pub use input::{parse_key, send_key_combo, KeyCombo};
pub use watcher::ForegroundWatcher;
pub use window::{
    bring_to_foreground, find_window, find_window_by_process, foreground_info, is_foreground,
    post_message, process_name_of, send_message, window_pid, ForegroundInfo,
};
