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
//! 能力边界（对 §7.3 双通道决策的影响）：MPC-HC **没有**「一步设精确倍速」的公开
//! 消息（resource.h 与 Web 接口命令表中均未发现 set-rate 类命令），只能
//! [`ID_PLAY_INCRATE`] / [`ID_PLAY_DECRATE`] 步进 + [`ID_PLAY_RESETRATE`] 归一逼近。
//! 步长随版本/设置而变（旧版为倍增/减半；clsid2 维护版可在选项中配置固定步长），
//! 因此 MPC-HC 的 adapter 只能以「目标倍速估计值」申报状态（§3 非对称控制、§14）。
//!
//! # MPC-BE 注意（存疑项）
//!
//! MPC-BE 是独立演进的分支：主窗口类名为 [`WINDOW_CLASS_MPC_BE`]（`"MPC-BE"`，
//! 来源：AutoHotkey 社区多篇脚本以 `ahk_class MPC-BE` 定位），其命令码与 MPC-HC
//! **不完全一致**（例如 MPC-HC 的 24044 回收站命令在 BE 不存在；BE 的 seek 类 ID
//! 与 HC 有出入）。播放/暂停 889 在两者间有社区用例佐证一致，但 894–896 调速码
//! 未在 MPC-BE 侧验证——M2 仅内置 MPC-HC 规则（§11），MPC-BE 支持需另行标定
//! （验证方法：查 aleksoid1978/MPC-BE 源码 resource.h，或实机 SendMessage 观察）。

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
/// 恢复正常速度 1×（`#define ID_PLAY_RESETRATE 896`）——逼近目标倍速前先归一，
/// 使「步长换算 + 节流连发」（§7.3）有确定的起点。
pub const ID_PLAY_RESETRATE: usize = 896;
