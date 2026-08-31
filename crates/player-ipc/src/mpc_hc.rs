//! MPC-HC 窗口消息控制码（开发文档 §7.3 IPC/控制接口，§11 M2 内置 MPC-HC 规则）。
//!
//! 本 crate 只交付常量表；`FindWindow` / `SendMessage` 由主程序结合 platform-win
//! 完成（架构约束见 lib.rs 顶部说明）。
//!
//! # 调研结论（2026-08 网络调研）
//!
//! 调用形态：`SendMessage(hwnd, WM_COMMAND, ID_* as WPARAM, 0)`（消息号用
//! [`crate::WM_COMMAND`]），等价于触发对应菜单项/快捷键，**无需前台焦点**。
//!
//! 出处：MPC-HC 官方源码 `src/mpc-hc/resource.h`（现行维护版 clsid2/mpc-hc，
//! <https://github.com/clsid2/mpc-hc/blob/develop/src/mpc-hc/resource.h>；
//! 原版 mpc-hc 数值一致）。这些 ID 同时公开于程序内「选项 → 播放器 → 键」的 ID 列
//! 与 Web 界面（默认端口 13579，`/command.html?wm_command=889`），三处一致。
//! **置信度：高。**
//!
//! 能力边界（对 §7.3 双通道决策的影响）：clsid2 维护版**有**「一步设精确倍速」的
//! 命令——[`PLAYBACK_RATES`] 那 14 条绝对倍速码，见下节。步进码
//! [`ID_PLAY_INCRATE`] / [`ID_PLAY_DECRATE`] 的步长随设置而变（`nSpeedStep` 默认 0
//! 即倍增/减半，用户设成 1–75 才是加性的 0.01–0.75×），且**没有任何回读**，
//! 因此步进只能当兜底，能走绝对码就别走它。
//!
//! # 绝对倍速命令（2026-08-31 源码取证，纠正此前「没有 set-rate」的结论）
//!
//! `clsid2/mpc-hc` `src/mpc-hc/MainFrm.cpp` 的 `filePlaybackRates` 把
//! `ID_PLAY_PLAYBACKRATE_025..800`（5001–5014）逐条映射到确定倍速，
//! `OnPlayChangeRate` 收到即 `SetPlayingRate(rate)` → `IMediaSeeking::SetRate(rate)`，
//! **一步落到该值**，与播放器原状态无关。命令已注册进消息映射表
//! （`ON_COMMAND_RANGE(ID_PLAY_PLAYBACKRATE_START, ID_PLAY_PLAYBACKRATE_END, OnPlayChangeRate)`），
//! MFC 不区分 WM_COMMAND 来自菜单还是外部 `SendMessage`，因此外部进程发送等价于点菜单。
//! 于是 MPC-HC 的 adapter 能申报**确定值**而非估计值，OSD 不必再报开环估算。
//!
//! **仅限 clsid2 维护版**：原版 `mpc-hc/mpc-hc`（2017 停更于 1.7.13）的 resource.h
//! 里只有 894–896，没有 5001–5014；发过去是静默 no-op。现今分发渠道（官网、
//! K-Lite Codec Pack）给的都是 clsid2 版，故内置规则按有绝对码处理。
//!
//! # MPC-BE（2026-08-31 已由源码结案，原「存疑」作废）
//!
//! MPC-BE 是独立演进的分支，主窗口类名为 [`WINDOW_CLASS_MPC_BE`]（`"MPC-BE"`）。
//! 查 `Aleksoid1978/MPC-BE` `src/apps/mplayerc/resource.h`：
//! `ID_PLAY_PLAY` 887 / `ID_PLAY_PLAYPAUSE` 889 / `ID_PLAY_DECRATE` 894 /
//! `ID_PLAY_INCRATE` 895 / `ID_PLAY_RESETRATE` 896，**与 MPC-HC 逐条一致**，
//! 且 BE 的消息映射表同样注册了这几条，故播放暂停与步进在 BE 上可用。
//! 但 BE 全文搜不到 `PLAYBACKRATE`——**绝对倍速码 BE 没有**，其步进走自己的
//! `GetNextRate(rate, nSpeedStep)` 档位表。因此 BE 必须单列规则，只发 894–896。

/// MPC-HC 主窗口类名（原版与 clsid2 维护版一致）。
/// 来源：AutoHotkey 社区库以 `ahk_class MediaPlayerClassicW` 定位窗口，多来源一致。
/// 置信度：高。
pub const WINDOW_CLASS: &str = "MediaPlayerClassicW";

/// MPC-BE 主窗口类名（信息性常量，命令码兼容性存疑，见模块头「MPC-BE 注意」）。
pub const WINDOW_CLASS_MPC_BE: &str = "MPC-BE";

// 以下 ID 逐条对应 clsid2/mpc-hc `src/mpc-hc/resource.h` 中的同名 #define。

/// 播放（`#define ID_PLAY_PLAY 887`）。
pub const ID_PLAY_PLAY: usize = 887;
/// 暂停（`#define ID_PLAY_PAUSE 888`）。
pub const ID_PLAY_PAUSE: usize = 888;
/// 播放/暂停切换（`#define ID_PLAY_PLAYPAUSE 889`）——播放暂停用这个。
pub const ID_PLAY_PLAYPAUSE: usize = 889;
/// 停止（`#define ID_PLAY_STOP 890`）。
pub const ID_PLAY_STOP: usize = 890;
/// 减速一档（`#define ID_PLAY_DECRATE 894`）。
pub const ID_PLAY_DECRATE: usize = 894;
/// 加速一档（`#define ID_PLAY_INCRATE 895`）。
pub const ID_PLAY_INCRATE: usize = 895;
/// 恢复正常速度 1×（`#define ID_PLAY_RESETRATE 896`）——两个分支都支持，
/// 是唯一一条「任何 MPC 变体上都必然把倍速钉到 1.0」的命令。
pub const ID_PLAY_RESETRATE: usize = 896;

/// 绝对倍速命令码 → 该命令设定的确切倍速（clsid2 `MainFrm.cpp` 的 `filePlaybackRates`，
/// 命令码取自同仓 `resource.h` 的 `ID_PLAY_PLAYBACKRATE_*`）。
///
/// 这是 MPC-HC 上唯一的**绝对**倍速控制面：发一条即到位，不依赖也不需要回读当前值。
/// 表按倍速升序排列，规则层的就近取档 / 挪一档依赖这一点。
///
/// 注意档距不均匀：1× 附近密（0.9/1.0/1.1/1.25），两端疏（2→3→4→6→8），
/// 所以它是**档位表**而不是网格，中间值只能就近取档。
///
/// 真机验证（MPC-HC 2.8.1，2026-08-31）：14 档逐条从外部进程 `SendMessage` 下发，
/// 播放器全程不在前台，13 档的实测播放速率与命令值吻合。**唯独 8× 只是自报**——
/// 播放器回报 8.000 而播放位置只以 3.0× 推进（6× 实测 6.03 完全跟得上），
/// 是渲染链的速率上限，不是命令没生效。这一档 MPC-HC 自己的 OSD 同样显示 8×。
pub const PLAYBACK_RATES: [(usize, f64); 14] = [
    (5001, 0.25),
    (5002, 0.50),
    (5003, 0.75),
    (5004, 0.90),
    (5005, 1.00),
    (5006, 1.10),
    (5007, 1.25),
    (5008, 1.50),
    (5009, 1.75),
    (5010, 2.00),
    (5011, 3.00),
    (5012, 4.00),
    (5013, 6.00),
    (5014, 8.00),
];

/// 全部可达倍速（[`PLAYBACK_RATES`] 的倍速列，升序）。
/// 交给规则层当作该应用的档位表，「就近取档 / 挪一档」的算法在那边统一实现，
/// 本模块只负责这张表本身以及档位 → 命令码的映射。
pub fn rate_ladder() -> Vec<f64> {
    PLAYBACK_RATES.iter().map(|(_, rate)| *rate).collect()
}

/// 档位倍速 → 对应的绝对倍速命令码。入参须是 [`rate_ladder`] 里的某一档
/// （由规则层就近取档得到）；不在表内返回 `None`，由调用方降级为步进。
pub fn command_for(rate: f64) -> Option<usize> {
    // 表里都是两位小数的确切值，取档时原样传递，这里的容差只为兜住浮点往返
    PLAYBACK_RATES
        .iter()
        .find(|(_, r)| (r - rate).abs() < 1e-6)
        .map(|(cmd, _)| *cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 命令码与倍速逐条对齐 clsid2 源码（resource.h 的 #define + MainFrm.cpp 的
    /// filePlaybackRates）。这张表是「MPC-HC 能精确设速」的全部依据，写错一条就是谎报。
    #[test]
    fn table_matches_the_source() {
        assert_eq!(PLAYBACK_RATES.first(), Some(&(5001, 0.25)));
        assert_eq!(PLAYBACK_RATES.last(), Some(&(5014, 8.0)));
        // 命令码连号 5001..=5014，倍速严格递增——两个不变量都被查表逻辑依赖
        for (i, (cmd, rate)) in PLAYBACK_RATES.iter().enumerate() {
            assert_eq!(*cmd, 5001 + i, "第 {i} 条命令码应连号");
            if i > 0 {
                assert!(*rate > PLAYBACK_RATES[i - 1].1, "档位表必须升序");
            }
        }
        // 全部落在 ID_PLAY_PLAYBACKRATE_START(5000)..END(5029) 的开区间内，
        // 否则 MPC-HC 的 ON_COMMAND_RANGE 根本不会分发给 OnPlayChangeRate
        assert!(PLAYBACK_RATES.iter().all(|(cmd, _)| (5001..=5028).contains(cmd)));
    }

    /// 档位 → 命令码：表内的每一档都要能查到，且查到的是它自己那条
    #[test]
    fn command_for_maps_every_rung() {
        for (cmd, rate) in PLAYBACK_RATES {
            assert_eq!(command_for(rate), Some(cmd), "{rate}× 应映射到 {cmd}");
        }
        // 档位之间的值不属于任何命令：规则层没就近取档就传进来是 bug，
        // 宁可返回 None 让上层降级为步进，也不要挑一条"差不多"的命令发出去
        assert_eq!(command_for(2.4), None);
        assert_eq!(command_for(0.1), None);
        assert_eq!(command_for(f64::NAN), None);
    }

    /// 1× 必须在表内：`reset` 走 ID_PLAY_RESETRATE，回读值申报的就是这一档
    #[test]
    fn one_times_is_a_rung() {
        assert!(rate_ladder().contains(&1.0));
        assert_eq!(rate_ladder().len(), PLAYBACK_RATES.len());
    }
}
