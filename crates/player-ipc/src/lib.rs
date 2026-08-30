//! # player-ipc —— 播放器控制通道客户端（开发文档 §7.3）
//!
//! Tier 2（桌面播放器）的控制策略是「IPC/控制接口优先，模拟按键兜底」（开发文档 §2、§7.3）：
//! IPC 无需焦点、可一步设精确倍速、部分可回读；按键只能按档位步进且要求前台焦点。
//! 本 crate 承载「IPC 优先」这一半，按播放器分四个通道：
//!
//! | 播放器 | 通道 | 本 crate 提供 | 对应 `appRules[].ipc.kind`（开发文档 §8） |
//! | --- | --- | --- | --- |
//! | mpv | JSON-IPC（Windows 命名管道） | [`MpvClient`]（设速/读速/播放暂停） | `"mpv-ipc"`（`pipe` 参数） |
//! | VLC | HTTP 接口 status.xml | [`VlcHttpClient`]（设速/读速/播放暂停） | `"vlc-http"`（`port` 参数） |
//! | Chromium 套壳客户端（B 站桌面端等） | CDP 调试口（HTTP + WebSocket） | [`CdpClient`]（设速/读速/播放暂停） | `"cdp"`（`port` 参数） |
//! | PotPlayer | 窗口消息（WM_USER SDK / WM_COMMAND） | [`potplayer`] 消息码常量表 + 换算函数 | 由主程序结合 platform-win 使用 |
//! | MPC-HC | 窗口消息（WM_COMMAND） | [`mpc_hc`] 消息码常量表 | 同上 |
//!
//! ## 架构约束：纯 IO，不引入 Win32 依赖
//!
//! PotPlayer / MPC-HC 的控制需要 `FindWindow` + `SendMessage`，属于 unsafe Win32 范畴。
//! 按开发文档 §5.1 的分层原则（unsafe 集中封装在 platform/win），窗口消息的**发送**由主程序
//! 结合 platform-win crate 完成；本 crate 只交付消息码常量与纯计算的参数换算，
//! 因此可在任意平台编译、全部逻辑可离线单元测试。

use thiserror::Error;

mod cdp;
mod mpv;
mod vlc;
pub mod mpc_hc;
pub mod potplayer;

pub use cdp::{CdpClient, GuardSession};
pub use mpv::MpvClient;
pub use vlc::VlcHttpClient;

/// 统一错误：区分「未运行/不可达」「认证失败」「协议错误」。
///
/// 上层（core 的 player adapter）依赖这个区分做决策：
/// - [`IpcError::Unavailable`] → `method = "auto"` 时自动回退到模拟按键（开发文档 §8）；
/// - [`IpcError::AuthFailed`] → 应用页显示「需要设置」并引导用户检查 VLC 密码（§7.3）；
/// - [`IpcError::Protocol`] → 记日志，通道本身可达但本次操作失败。
#[derive(Error, Debug)]
pub enum IpcError {
    #[error("播放器未运行或控制接口未开启")]
    Unavailable,
    #[error("认证失败（检查 VLC HTTP 密码）")]
    AuthFailed,
    #[error("协议错误：{0}")]
    Protocol(String),
}

/// Win32 `WM_COMMAND` 消息号（0x0111）。
///
/// 本 crate 不发送消息，只提供数值：platform-win 侧以
/// `SendMessage(hwnd, WM_COMMAND, 命令码 as WPARAM, 0)` 触发等价于菜单/热键的命令。
pub const WM_COMMAND: u32 = 0x0111;

/// Win32 `WM_USER` 消息号（0x0400），PotPlayer 官方 SDK 通道的消息号。
///
/// platform-win 侧以 `SendMessage(hwnd, WM_USER, POT_* as WPARAM, 参数 as LPARAM)` 调用。
pub const WM_USER: u32 = 0x0400;
