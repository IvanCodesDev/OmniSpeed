//! PotPlayer 窗口消息控制码（开发文档 §7.3：「PotPlayer：WM_COMMAND 控制消息设速，
//! SendMessage 无需前台，具体消息码 Spike 标定」——本模块即该 Spike 的标定结论）。
//!
//! 本 crate 只交付常量与纯计算的参数换算；`FindWindow` / `SendMessage` 由主程序
//! 结合 platform-win 完成（架构约束见 lib.rs 顶部说明）。
//!
//! # 调研结论（2026-08 网络调研，来源与置信度逐条标注）
//!
//! PotPlayer 实际有**两条**消息通道，能力不同：
//!
//! ## 通道一：WM_USER 官方 SDK（推荐，可一步精确设速、可回读）
//!
//! 调用形态：`SendMessage(hwnd, WM_USER, POT_* as WPARAM, 参数 as LPARAM)`，
//! 返回值（LRESULT）即查询结果。消息号用 [`crate::WM_USER`]（0x0400）。
//!
//! 出处：PotPlayer 官方论坛「팟플레이어 실험실（PotPlayer 实验室）」置顶的
//! PotPlayer SDK（2023-08-29 更新版，<https://m.cafe.daum.net/pot-tool/N88T/6>），
//! 常量名与数值同时被 AutoHotkey 社区库「PotPlayer x64 Function Library」、
//! ld3l/PotPlayerControl（JNA）等多个开源项目长期使用。**置信度：高。**
//!
//! 关键发现：[`POT_SET_SPEED`]（0x5016）可**一步设精确倍速**（lParam = 倍速×1000），
//! [`POT_GET_SPEED`]（0x5015）可回读——这正是 §7.3「IPC 一步设 5×、部分可回读」
//! 在 PotPlayer 上的落点，优先级应高于下面的步进命令。
//! 注意 SDK 区间是 200–12000（0.2×–12×）：PotPlayer 的 IPC 上限 12×，低于产品
//! 全局上限 16×（§7.8），adapter 的 capabilities 须按 12× 申报。
//!
//! ## 通道二：WM_COMMAND 热键命令码（兜底，只能按档位步进）
//!
//! 调用形态：`SendMessage(hwnd, WM_COMMAND, CMD_* as WPARAM, 0)`，等价于按下对应
//! 菜单/热键。消息号用 [`crate::WM_COMMAND`]（0x0111）。
//!
//! 出处：社区从 PotPlayer 提取的命令码表——ld3l/PotPlayerControl 的 raw.md
//! （<https://github.com/ld3l/PotPlayerControl/blob/main/raw.md>）与 AutoHotkey
//! 「PotPlayer x64 Function Library」（<https://www.autohotkey.com/boards/viewtopic.php?t=45385>）。
//! 非官方文档，但该表中与官方 SDK 重叠的条目数值完全吻合（如 20487 = 0x5007
//! SET_PLAY_STATUS、24624 = 0x6030 GET_VIDEO_WIDTH），交叉验证通过。**置信度：中。**
//! 验证方法：对运行中的 PotPlayer 逐条 `SendMessage` 后观察 OSD 提示（M2 集成回归矩阵，§12）。

/// 主窗口类名（64 位版，当前主流发行版）。来源：AutoHotkey 社区库以
/// `ahk_class PotPlayer64` 定位窗口，多项目一致。置信度：高。
pub const WINDOW_CLASS_64: &str = "PotPlayer64";

/// 主窗口类名（32 位版）。来源同上（`ahk_class PotPlayer`）。置信度：高。
pub const WINDOW_CLASS_32: &str = "PotPlayer";

// ---------------------------------------------------------------------------
// 通道一：WM_USER 官方 SDK（wParam = POT_*，lParam = 参数，返回 LRESULT）
// 以下数值全部出自官方 SDK 头文件（见模块头「通道一」出处）。
// ---------------------------------------------------------------------------

/// 查询播放状态。返回：0=停止（旧版资料记为 -1，见下方「存疑」）、1=暂停、2=播放中。
///
/// 存疑：2023-08 版官方 SDK 注释为 `0:Stopped`，而更早流传的版本写 `-1:Stopped`；
/// 主程序判断「是否停止」时建议同时接受 0 与 -1，实机标定后收敛（验证方法见模块头）。
pub const POT_GET_PLAY_STATUS: usize = 0x5006;

/// 设置播放状态。lParam：0=切换（推荐用于播放/暂停）、1=暂停、2=播放。
pub const POT_SET_PLAY_STATUS: usize = 0x5007;

/// 播放/暂停切换对应的 [`POT_SET_PLAY_STATUS`] lParam 值。
pub const POT_PLAY_STATUS_TOGGLE: isize = 0;
/// 暂停对应的 [`POT_SET_PLAY_STATUS`] lParam 值。
pub const POT_PLAY_STATUS_PAUSE: isize = 1;
/// 播放对应的 [`POT_SET_PLAY_STATUS`] lParam 值。
pub const POT_PLAY_STATUS_PLAY: isize = 2;

/// 查询当前倍速。返回 200–12000，即倍速×1000（0.2×–12×）。
pub const POT_GET_SPEED: usize = 0x5015;

/// **一步设置精确倍速**。lParam = 倍速×1000，区间 200–12000（0.2×–12×）。
/// 例：5× → `SendMessage(hwnd, WM_USER, POT_SET_SPEED, 5000)`。
pub const POT_SET_SPEED: usize = 0x5016;

/// SDK 速度参数的缩放因子（lParam = 倍速 × 1000）。
pub const POT_SPEED_SCALE: f64 = 1000.0;
/// SDK 速度参数下限（0.2×）。
pub const POT_SPEED_MIN: isize = 200;
/// SDK 速度参数上限（12×）——PotPlayer IPC 通道的能力上限，低于产品全局 16×。
pub const POT_SPEED_MAX: isize = 12000;

/// 目标倍速 → [`POT_SET_SPEED`] 的 lParam（×1000 取整，并收敛进 SDK 区间 200–12000）。
///
/// 这里只保护**协议边界**（越界值会被 PotPlayer 拒绝或产生未定义行为）；
/// 产品级 [0.25, 16] 的 clamp 由 core 统一执行（§7.8），不在本 crate 重复。
/// 非有限值（NaN/∞）不可能来自正常调用链，兜底回 1×。
pub fn speed_to_lparam(speed: f64) -> isize {
    if !speed.is_finite() {
        return POT_SPEED_SCALE as isize;
    }
    let scaled = (speed * POT_SPEED_SCALE).round();
    scaled.clamp(POT_SPEED_MIN as f64, POT_SPEED_MAX as f64) as isize
}

/// [`POT_GET_SPEED`] 的返回值（LRESULT）→ 倍速。
pub fn speed_from_lresult(value: isize) -> f64 {
    value as f64 / POT_SPEED_SCALE
}

// ---------------------------------------------------------------------------
// 通道二：WM_COMMAND 热键命令码（wParam = CMD_*，lParam = 0）
// 以下数值出自社区命令码表（见模块头「通道二」出处），置信度：中。
// ---------------------------------------------------------------------------

/// 播放/暂停切换。来源：AHK 库（CMD_PLAY_PAUSE=10014）与 raw.md
/// （10014 = "pause playback on / off"）两处一致。
pub const CMD_PLAY_PAUSE: usize = 10014;

/// 恢复正常速度（1×）。来源：raw.md（10246 = "playback speed 1x"）。
pub const CMD_SPEED_NORMAL: usize = 10246;

/// 减速一档（约 −0.1×）。来源：raw.md（10247 = "playback speed -0.1x"）。
pub const CMD_SPEED_DOWN: usize = 10247;

/// 加速一档（约 +0.1×）。来源：raw.md（10248 = "playback speed +0.1x"）。
/// 兜底逼近目标倍速时按 0.1×/次换算连发次数（§7.3「步长换算 + 节流连发」）。
pub const CMD_SPEED_UP: usize = 10248;

#[cfg(test)]
mod tests {
    use super::*;

    /// 常规换算：×1000 取整
    #[test]
    fn speed_to_lparam_scales_by_1000() {
        assert_eq!(speed_to_lparam(5.0), 5000);
        assert_eq!(speed_to_lparam(1.0), 1000);
        assert_eq!(speed_to_lparam(0.25), 250);
        // 浮点尾差取整（2.9999999 → 3000）
        assert_eq!(speed_to_lparam(2.999_999_9), 3000);
    }

    /// 越界值收敛进 SDK 区间（0.2×–12×），防止协议层拒绝
    #[test]
    fn speed_to_lparam_clamps_to_sdk_range() {
        assert_eq!(speed_to_lparam(0.1), POT_SPEED_MIN);
        assert_eq!(speed_to_lparam(16.0), POT_SPEED_MAX);
        assert_eq!(speed_to_lparam(0.0), POT_SPEED_MIN);
    }

    /// 非有限值兜底回 1×（不会 panic、不会产生区间外参数）
    #[test]
    fn speed_to_lparam_non_finite_falls_back_to_normal() {
        assert_eq!(speed_to_lparam(f64::NAN), 1000);
        assert_eq!(speed_to_lparam(f64::INFINITY), 1000);
    }

    #[test]
    fn speed_from_lresult_inverse() {
        assert_eq!(speed_from_lresult(5000), 5.0);
        assert_eq!(speed_from_lresult(250), 0.25);
        assert_eq!(speed_from_lresult(speed_to_lparam(2.5)), 2.5);
    }
}
