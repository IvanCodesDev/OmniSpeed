//! 应用规则：内置播放器规则表、用户覆盖与前端 DTO（开发文档 §8 appRules）。
//! 规则回答三个问题：这个进程是谁、用什么通道控制它（IPC 优先 / 按键兜底）、按键是什么。
//! M2 内置 mpv / VLC / PotPlayer / MPC-HC 四家（开发文档 §11 M2 行）；
//! 浏览器条目仅占位展示，真实接管等 M3 扩展接入。

use player_ipc::mpc_hc;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppKind {
    Browser,
    Player,
    /// 平台桌面客户端（B 站桌面端等 Chromium 套壳应用，走 CDP 接管）
    Client,
    Unknown,
}

/// 接管状态（PRD §7.2）：connected=扩展已连接（M3）/ adapted=规则可用 / needs-setup=缺少可用通道
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppStatus {
    Connected,
    Adapted,
    NeedsSetup,
}

/// 控制方式：auto = IPC 优先、失败回退按键（开发文档 §7.3）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleMethod {
    Auto,
    Ipc,
    Hotkey,
    Extension,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IpcKind {
    MpvIpc,
    VlcHttp,
    WmCommand,
    /// Chromium 套壳客户端的 CDP 调试口（`port` 参数；需先「接管」带参启动）
    Cdp,
    None,
}

/// IPC 通道参数（按 kind 取用对应字段）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcSettings {
    pub pipe: Option<String>,
    pub port: Option<u16>,
    pub password: Option<String>,
}

/// 模拟按键兜底的键位（目标播放器自身的快捷键，格式由 platform-win parse_key 解析）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyBindings {
    pub up: String,
    pub down: String,
    pub reset: String,
}

/// 键盘**绝对档位**：按下 `key` 之后倍速确定等于 `rate`，而不是在原值上加减。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateAnchor {
    pub key: String,
    pub rate: f64,
}

/// 播放器自身快捷键构成的倍速网格（[`AppRule::key_rate`]）。
///
/// 按键通道一向只能「按一下快一点」，是因为相对步进离不开当前值，而按键又回读不了。
/// 但有些播放器（百度网盘桌面端的数字键 1–5）给的是**绝对值**：按下即钉死到确定倍速。
/// 有了这种档位，精确设速就成立了——先按最近的档位键把倍速钉住，再用步进键补足差额，
/// 全程不需要知道也不需要回读播放器的原状态，因为每一步的结果都是已知的。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRateGrid {
    pub anchors: Vec<RateAnchor>,
    /// [`KeyBindings::up`] / [`KeyBindings::down`] 每按一次的增减量
    pub step: f64,
    /// 播放器自身的倍速上下限，超出即收敛到端点
    pub min: f64,
    pub max: f64,
    /// 连发步进键的最小间隔。步进键要先读回当前倍速再加减，播放器把这一步
    /// 摊在页面 JS → 原生播放器的异步 IPC 上；贴着发时后一次读到的还是旧值，
    /// 两次按键算出同一个结果，**表现为静默少走一格**。按键通道又没有回读，
    /// 少走的这一格永远补不回来，所以间隔按各播放器实测值来定（百度网盘 150ms）
    #[serde(default = "default_step_gap_ms")]
    pub step_gap_ms: u64,
}

/// 未标注间隔的自定义网格按此值下发：比实测出的百度网盘阈值再宽一档
fn default_step_gap_ms() -> u64 {
    150
}

/// 一次精确设速要按的键，以及按完之后播放器上的**确切**倍速
#[derive(Debug, Clone, PartialEq)]
pub struct KeyPlan {
    /// 先按的绝对档位键
    pub anchor: String,
    /// 补足差额的步进键（[`KeyBindings`] 的 up 或 down）
    pub step_key: String,
    pub steps: u32,
    pub rate: f64,
    /// 相邻两次按键之间至少要等的毫秒数（见 [`KeyRateGrid::step_gap_ms`]）
    pub gap_ms: u64,
}

impl KeyRateGrid {
    /// 单次设速允许的最多按键数。内置网格最多 6 键（1 档位 + 5 步进）；
    /// 自定义网格档位稀疏时按键数线性增长，超过此数说明这网格不适合精确设速，
    /// 与其糊上去连按几十下，不如让上层降级为步进。
    const MAX_KEYS: u32 = 12;

    /// 目标倍速 → 按键计划。目标先收敛到 `[min, max]`，再对齐到 `step` 网格
    /// （0.1 是播放器自身的精度，1.75 只能落到 1.8，不是实现偷懒）。
    /// 无可用档位、step 非正、或所需按键过多时返回 `None`，由调用方降级。
    pub fn plan(&self, keys: &KeyBindings, target: f64) -> Option<KeyPlan> {
        if self.step <= 0.0 || !self.step.is_finite() || !target.is_finite() {
            return None;
        }
        // 整数化到 step 为单位再运算：0.1 在二进制里本就不精确，
        // 直接累加浮点会让「按 5 次」和「差 0.5」对不上
        let want = self.units(target.clamp(self.min, self.max));
        let anchor = self.anchors.iter().min_by_key(|a| {
            let u = self.units(a.rate);
            ((u - want).abs(), u) // 距离相同时取倍速小的那个，保证结果稳定可测
        })?;

        let delta = want - self.units(anchor.rate);
        let steps = u32::try_from(delta.unsigned_abs()).ok()?;
        if steps.saturating_add(1) > Self::MAX_KEYS {
            return None;
        }
        Some(KeyPlan {
            anchor: anchor.key.clone(),
            step_key: if delta >= 0 { keys.up.clone() } else { keys.down.clone() },
            steps,
            // 回读值直接用算出来的确切倍速；抹掉浮点尾数，免得 OSD 上蹦出 2.5000000000000004
            rate: ((want as f64) * self.step * 1000.0).round() / 1000.0,
            gap_ms: self.step_gap_ms,
        })
    }

    /// 热键步进的目标值：把「当前值 ± 步长」折算成**整数格数**的移动，至少一格。
    ///
    /// 不这么做会有两个后果。其一，默认步长 0.25 在 0.1 网格上会让 OSD 报出 1.25×——
    /// 一个这台播放器根本给不出的值，随后又被回读悄悄改成 1.3×。其二更糟：步长不足
    /// 半格时（配置文件被手改到 0.05 即是），就近取整会把目标吸回原值，
    /// 于是热键按下去彻底没反应，而且没有任何报错。
    pub fn step_target(&self, from: f64, step: f64, dir: i32) -> Option<f64> {
        if self.step <= 0.0 || !self.step.is_finite() || !from.is_finite() || !step.is_finite() {
            return None;
        }
        let moves = (step.abs() / self.step).round().max(1.0) as i64;
        let moved = if dir < 0 { -moves } else { moves };
        let units = self.units(from.clamp(self.min, self.max)) + moved;
        let rate = ((units as f64) * self.step * 1000.0).round() / 1000.0;
        Some(rate.clamp(self.min, self.max))
    }

    fn units(&self, rate: f64) -> i64 {
        // 0.1 这类步长在二进制里不精确：1.75 / 0.1 实际算出 17.499999999999996，
        // 直接 round 会把正落在半格上的目标甩到下面一格（1.75 → 1.7）。
        // 先收到 1e-9 精度，半格才按"四舍五入到远离零"的直觉走
        let raw = rate / self.step;
        ((raw * 1e9).round() / 1e9).round() as i64
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRule {
    pub id: String,
    pub name: String,
    /// 小写主进程名，如 "vlc.exe"
    pub process: String,
    /// 同一软件的其他进程名（如 PotPlayer 的 32/64 位变体），匹配时一并考虑
    #[serde(default)]
    pub aliases: Vec<String>,
    pub kind: AppKind,
    pub method: RuleMethod,
    pub ipc: IpcKind,
    pub ipc_config: Option<IpcSettings>,
    pub keys: Option<KeyBindings>,
    /// 播放器自身快捷键含绝对档位时的倍速网格，有它按键通道才能精确设速。
    /// 属代码资产（同 process/kind），不随用户配置覆盖
    #[serde(default)]
    pub key_rate: Option<KeyRateGrid>,
    /// 控制通道自身的**确定倍速档位表**（升序）：设速只会落在这些值上。
    ///
    /// 与 [`key_rate`](Self::key_rate) 的区别在通道而非形式——网格是「连按快捷键拼出来的」，
    /// 因而有连发间隔与按键数上限；档位表是控制消息**一条到位**的（MPC-HC 的
    /// `ID_PLAY_PLAYBACKRATE_*`），没有中间态，但只能落在表里这些值上。
    /// 同属代码资产，不随用户配置覆盖。
    #[serde(default)]
    pub rate_ladder: Option<Vec<f64>>,
    pub builtin: bool,
}

/// 就近取档（[`AppRule::rate_ladder`]）：距离相同时取倍速小的那个。
/// 与 [`KeyRateGrid::plan`] 的取舍一致——结果必须稳定可测，
/// 不能随浮点误差在两档之间摇摆，否则同一个目标值两次下发会落到不同档上。
pub fn snap_to_ladder(ladder: &[f64], target: f64) -> Option<f64> {
    if !target.is_finite() {
        return None;
    }
    ladder
        .iter()
        .copied()
        .reduce(|best, cand| if (cand - target).abs() < (best - target).abs() { cand } else { best })
}

/// 档位表上的浮点容差：`Core.rate` 已被 `clamp_rate` 收到两位小数，
/// 1e-6 足以把它吸附回档上，又不至于把相邻档误判成同一档
const LADDER_EPS: f64 = 1e-6;

impl AppRule {
    pub fn matches(&self, process_name: &str) -> bool {
        self.process == process_name || self.aliases.iter().any(|a| a == process_name)
    }

    /// 沿档位表挪一档（dir: +1 / -1）。`from` 不在档上时取该方向上第一个越过它的档；
    /// 该方向已无档位则收到端点——热键连按到顶应停在最高档，既不绕回也不跳空。
    pub fn ladder_step(&self, from: f64, dir: i32) -> Option<f64> {
        let ladder = self.rate_ladder.as_deref()?;
        if !from.is_finite() || dir == 0 {
            return None;
        }
        let mut rungs = ladder.iter().copied();
        if dir > 0 {
            rungs.find(|r| *r > from + LADDER_EPS).or_else(|| ladder.last().copied())
        } else {
            rungs.rfind(|r| *r < from - LADDER_EPS).or_else(|| ladder.first().copied())
        }
    }

    /// 该规则是否有**能读回真实倍速**的通道。
    ///
    /// 热键步进据此决定要不要先把目标值量化到按键网格上：有回读时异步拍会拿真实值
    /// 校正 `Core.rate`，先量化只会白白把用户设的 0.25 步长撑成网格的 0.3；
    /// 没有回读时（百度网盘、MPC-HC）就必须自己量化，否则 OSD 报的是播放器给不出的值。
    pub fn can_read_back_rate(&self) -> bool {
        if !matches!(self.method, RuleMethod::Auto | RuleMethod::Ipc) {
            return false;
        }
        match self.ipc {
            IpcKind::MpvIpc | IpcKind::VlcHttp | IpcKind::Cdp => true,
            // PotPlayer 的 SDK 有 POT_GET_SPEED 可回读；MPC 系的 WM_COMMAND 一条回读也没有
            IpcKind::WmCommand => self.id == "potplayer",
            IpcKind::None => false,
        }
    }

    /// 接管状态推导：浏览器等 M3 扩展；播放器只要有 IPC 或按键任一通道即视为已适配
    pub fn status(&self) -> AppStatus {
        match self.kind {
            AppKind::Browser => AppStatus::NeedsSetup,
            _ if self.ipc != IpcKind::None || self.keys.is_some() => AppStatus::Adapted,
            _ => AppStatus::NeedsSetup,
        }
    }

    /// 展示文案（前端 methodLabel）
    pub fn method_label(&self) -> String {
        match self.method {
            RuleMethod::Extension => "浏览器扩展".into(),
            // 有绝对档位时按键通道也能一步到精确值，与"只能步进"要区分开
            RuleMethod::Hotkey if self.key_rate.is_some() => "播放器快捷键 · 精确设速".into(),
            RuleMethod::Hotkey => "快捷键".into(),
            RuleMethod::Ipc if self.ipc == IpcKind::Cdp => "CDP 接管".into(),
            RuleMethod::Ipc => "IPC 接口".into(),
            RuleMethod::Auto => {
                if self.ipc == IpcKind::Cdp {
                    "CDP 接管".into()
                } else if self.rate_ladder.is_some() {
                    // 控制消息能一条到位，但只落在固定档位上，与"任意精确值"要区分开
                    "控制消息 · 档位设速".into()
                } else if self.ipc != IpcKind::None {
                    "IPC 接口 · 按键兜底".into()
                } else {
                    "快捷键".into()
                }
            }
        }
    }
}

/// 内置规则表。键位为各播放器的出厂默认（用户可在应用页改）：
/// - mpv：[ / ] 步进调速，Backspace 恢复 1.0（IPC 管道为约定默认值，需用户以
///   --input-ipc-server 启动或写入 mpv.conf，应用页给出指引）
/// - VLC：[ / ] 步进，= 恢复；HTTP 接口默认 8080，密码必须由用户设置
/// - PotPlayer：C / X 步进，Z 恢复；优先 WM_COMMAND 控制消息
/// - MPC-HC：Ctrl+Up / Ctrl+Down 步进，R 恢复（存疑，见 player-ipc 调研）；优先 WM_COMMAND
pub fn built_in_rules() -> Vec<AppRule> {
    let keys = |up: &str, down: &str, reset: &str| {
        Some(KeyBindings { up: up.into(), down: down.into(), reset: reset.into() })
    };
    vec![
        AppRule {
            id: "mpv".into(),
            name: "mpv".into(),
            process: "mpv.exe".into(),
            aliases: vec!["mpvnet.exe".into()],
            kind: AppKind::Player,
            method: RuleMethod::Auto,
            ipc: IpcKind::MpvIpc,
            ipc_config: Some(IpcSettings {
                pipe: Some(r"\\.\pipe\mpvsocket".into()),
                ..Default::default()
            }),
            keys: keys("]", "[", "Backspace"),
            key_rate: None,
            rate_ladder: None,
            builtin: true,
        },
        AppRule {
            id: "vlc".into(),
            name: "VLC media player".into(),
            process: "vlc.exe".into(),
            aliases: vec![],
            kind: AppKind::Player,
            method: RuleMethod::Auto,
            ipc: IpcKind::VlcHttp,
            ipc_config: Some(IpcSettings { port: Some(8080), ..Default::default() }),
            keys: keys("]", "[", "="),
            // HTTP 接口出厂没密码、装完即用的场景下基本不可用，按键才是 VLC 的常态通道。
            // 好在 VLC 的按键语义正好是「锚点 + 均匀加性」，与本网格模型逐条对得上
            // （`modules/control/hotkeys.c` 的 AdjustRateFine：
            //  `floor(rate/0.1 + dir + 0.05) * 0.1`，即在 0.1 网格上整格挪一格且自带吸附；
            //  `=` 走 ACTIONID_RATE_NORMAL → `var_SetFloat(rate, 1.f)`，是唯一的绝对锚点）
            key_rate: Some(KeyRateGrid {
                anchors: vec![RateAnchor { key: "=".into(), rate: 1.0 }],
                step: 0.1,
                // VLC 自身能到 0.03125×–31.25×（INPUT_RATE_DEFAULT / INPUT_RATE_MIN·MAX），
                // 但只有 `=` 一个锚点，MAX_KEYS=12 就把按键通道的精确半径钉死在 ±1.1。
                // 这里写按键真够得着的区间而不是 VLC 的理论区间：写宽了 plan() 会直接放弃，
                // 退回开环单步——那才是 OSD 谎报的来源
                min: 0.25,
                max: 2.1,
                // 真机标定（VLC 3.0.23）：11 连发 × 5 档间隔 × 3 轮，间隔 0/5/10/20/40ms
                // 全部精确落到 2.1，一格没丢。结构上也应当如此——VLC 的步进是进程内
                // `floor()` 现算现写，不像百度网盘那样要先异步读回当前倍速，
                // 没有"后一次读到旧值"的窗口。取 20ms 纯属留余量，11 步共 220ms
                step_gap_ms: 20,
            }),
            rate_ladder: None,
            builtin: true,
        },
        AppRule {
            id: "potplayer".into(),
            name: "PotPlayer".into(),
            process: "potplayermini64.exe".into(),
            aliases: vec![
                "potplayermini.exe".into(),
                "potplayer64.exe".into(),
                "potplayer.exe".into(),
            ],
            kind: AppKind::Player,
            method: RuleMethod::Auto,
            ipc: IpcKind::WmCommand,
            ipc_config: None,
            // PotPlayer 闭源，C/X 的步长与上下限无从取证；SDK 又开箱即用，
            // 按键只是深度兜底。与其抄一份「最佳努力值」，不如留空等真机标定
            keys: keys("C", "X", "Z"),
            key_rate: None,
            rate_ladder: None,
            builtin: true,
        },
        AppRule {
            id: "mpc-hc".into(),
            name: "MPC-HC".into(),
            process: "mpc-hc64.exe".into(),
            aliases: vec!["mpc-hc.exe".into()],
            kind: AppKind::Player,
            method: RuleMethod::Auto,
            ipc: IpcKind::WmCommand,
            ipc_config: None,
            keys: keys("Ctrl+Up", "Ctrl+Down", "R"),
            key_rate: None,
            // clsid2 维护版有 14 条绝对倍速命令，一条即到位（见 player_ipc::mpc_hc）。
            // 这是 MPC-HC 唯一能申报确定值的路子：INC/DECRATE 出厂是倍增/减半且无回读，
            // 靠它推算倍速从第一下就会与播放器对不上
            rate_ladder: Some(mpc_hc::rate_ladder()),
            builtin: true,
        },
        // MPC-BE 是独立分支，不能当作 MPC-HC 的 alias：窗口类名不同（`MPC-BE`），
        // 且**没有**绝对倍速命令（源码全文无 PLAYBACKRATE），只有 894/895/896 与 HC 一致。
        // 混在一起会让我们把绝对码发给一个不认它的播放器——静默 no-op，还申报成功
        AppRule {
            id: "mpc-be".into(),
            name: "MPC-BE".into(),
            process: "mpc-be64.exe".into(),
            aliases: vec!["mpc-be.exe".into()],
            kind: AppKind::Player,
            method: RuleMethod::Auto,
            ipc: IpcKind::WmCommand,
            ipc_config: None,
            keys: keys("Ctrl+Up", "Ctrl+Down", "R"),
            key_rate: None,
            rate_ladder: None,
            builtin: true,
        },
        // 平台桌面客户端（Chromium 套壳，CDP 接管；进程名即客户端 exe 名）。
        // 端口各占一个，避免多客户端同时接管时相互串线
        AppRule {
            id: "bilibili-client".into(),
            name: "哔哩哔哩桌面端".into(),
            process: "哔哩哔哩.exe".into(),
            aliases: vec!["bilibili.exe".into()],
            kind: AppKind::Client,
            method: RuleMethod::Ipc,
            ipc: IpcKind::Cdp,
            ipc_config: Some(IpcSettings { port: Some(9333), ..Default::default() }),
            keys: None,
            key_rate: None,
            rate_ladder: None,
            builtin: true,
        },
        // 百度网盘桌面端：同样是 Electron 套壳，但**播放器不是 HTML5 的**——
        // 界面在 video_player.asar 里，解码与画面走原生 vastplayer.dll，经 videoipc 合成
        // 到窗口上。页面里既没有 <video> 也没有 playbackRate，CDP 那套（cdp.rs 的
        // MEDIA_PRELUDE / GUARD_SOURCE 全建立在 HTMLMediaElement 上）一行都用不上。
        // 可用的控制面是播放器自带的键盘快捷键，好在数字键给的是绝对倍速：
        // 1/2/3/4/5 → 精确 1×–5×，, / . → ∓0.1（源码内上下限 0.5–5），
        // 于是「档位 + 步进」反而能精确设速，见 KeyRateGrid。
        AppRule {
            id: "baidu-netdisk".into(),
            name: "百度网盘".into(),
            process: "baidunetdiskunite.exe".into(),
            aliases: vec!["baidunetdisk.exe".into()],
            kind: AppKind::Client,
            method: RuleMethod::Hotkey,
            ipc: IpcKind::None,
            ipc_config: None,
            keys: keys(".", ",", "1"),
            key_rate: Some(KeyRateGrid {
                anchors: [1.0, 2.0, 3.0, 4.0, 5.0]
                    .into_iter()
                    .map(|rate| RateAnchor { key: format!("{rate:.0}"), rate })
                    .collect(),
                step: 0.1,
                min: 0.5,
                max: 5.0,
                // 真机实测：40ms 连发五次 `.` 只走四格（2→2.4），100/150ms 连测 8 轮全中。
                // 取 150ms 留一档余量，满打满算 6 键约 750ms，OSD 由同步链路先行显示
                step_gap_ms: 150,
            }),
            rate_ladder: None,
            builtin: true,
        },
        AppRule {
            id: "chrome".into(),
            name: "Google Chrome".into(),
            process: "chrome.exe".into(),
            aliases: vec![],
            kind: AppKind::Browser,
            method: RuleMethod::Extension,
            ipc: IpcKind::None,
            ipc_config: None,
            keys: None,
            key_rate: None,
            rate_ladder: None,
            builtin: true,
        },
        AppRule {
            id: "edge".into(),
            name: "Microsoft Edge".into(),
            process: "msedge.exe".into(),
            aliases: vec![],
            kind: AppKind::Browser,
            method: RuleMethod::Extension,
            ipc: IpcKind::None,
            ipc_config: None,
            keys: None,
            key_rate: None,
            rate_ladder: None,
            builtin: true,
        },
        AppRule {
            id: "firefox".into(),
            name: "Firefox".into(),
            process: "firefox.exe".into(),
            aliases: vec![],
            kind: AppKind::Browser,
            method: RuleMethod::Extension,
            ipc: IpcKind::None,
            ipc_config: None,
            keys: None,
            key_rate: None,
            rate_ladder: None,
            builtin: true,
        },
    ]
}

/// 把持久化的规则合并到内置表：内置项按 id 覆盖可编辑字段，自定义项整条追加。
/// 内置规则的进程名/种类始终以代码为准，避免旧配置钉死已修正的内置数据。
pub fn merge_saved(rules: &mut Vec<AppRule>, saved: Vec<AppRule>) {
    for s in saved {
        if let Some(r) = rules.iter_mut().find(|r| r.id == s.id) {
            r.method = s.method;
            r.ipc = s.ipc;
            r.ipc_config = s.ipc_config;
            r.keys = s.keys;
        } else {
            rules.push(AppRule { builtin: false, ..s });
        }
    }
}

/// 前端「应用页」保存的规则补丁（契约见 lib/ipc.ts AppRulePatch）
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppRulePatch {
    pub id: String,
    pub process: String,
    pub name: String,
    pub method: RuleMethod,
    pub keys: Option<KeyBindings>,
    pub ipc_config: Option<IpcSettings>,
}

/// 前端应用页条目 DTO（契约见 lib/ipc.ts AppInfo）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub id: String,
    pub name: String,
    pub process: String,
    pub kind: AppKind,
    pub status: AppStatus,
    pub method: RuleMethod,
    pub method_label: String,
    pub ipc: IpcKind,
    pub running: bool,
    pub builtin: bool,
    pub keys: Option<KeyBindings>,
    pub ipc_config: Option<IpcSettings>,
    /// 有值 = 该应用的按键通道能精确设速，应用页据此说明可用区间与精度
    pub key_rate: Option<KeyRateGrid>,
    /// 有值 = 该应用只能落在这些确定倍速上（控制消息的档位表），应用页据此列出可选档位
    pub rate_ladder: Option<Vec<f64>>,
}

/// connected：扩展已连接的浏览器进程名集合（NM 桥维护），
/// 浏览器条目的状态由它决定（已连接 / 需要设置）
pub fn to_app_info(
    rule: &AppRule,
    running: &HashSet<String>,
    connected: &HashSet<String>,
) -> AppInfo {
    let mut status = rule.status();
    if rule.kind == AppKind::Browser
        && (connected.contains(&rule.process) || rule.aliases.iter().any(|a| connected.contains(a)))
    {
        status = AppStatus::Connected;
    }
    AppInfo {
        id: rule.id.clone(),
        name: rule.name.clone(),
        process: rule.process.clone(),
        kind: rule.kind,
        status,
        method: rule.method,
        method_label: rule.method_label(),
        ipc: rule.ipc,
        running: running.contains(&rule.process)
            || rule.aliases.iter().any(|a| running.contains(a)),
        builtin: rule.builtin,
        keys: rule.keys.clone(),
        ipc_config: rule.ipc_config.clone(),
        key_rate: rule.key_rate.clone(),
        rate_ladder: rule.rate_ladder.clone(),
    }
}

/// 当前正在运行的进程名集合（小写）。sysinfo 全量刷新有数十毫秒开销，
/// 只在应用页拉取/状态广播时调用，不进高频路径。
pub fn running_processes() -> HashSet<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing(),
    );
    sys.processes()
        .values()
        .map(|p| p.name().to_string_lossy().to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 百度网盘桌面端的真实网格（数字键 1–5 为绝对档位，`,`/`.` ∓0.1，播放器自身限 0.5–5×）
    fn baidu() -> (KeyRateGrid, KeyBindings) {
        let rule = built_in_rules()
            .into_iter()
            .find(|r| r.id == "baidu-netdisk")
            .expect("内置规则应含百度网盘");
        (
            rule.key_rate.expect("百度网盘应带倍速网格"),
            rule.keys.expect("百度网盘应带按键"),
        )
    }

    /// 档位本身一键到位，不多按任何步进
    #[test]
    fn anchors_need_no_stepping() {
        let (grid, keys) = baidu();
        for anchor in [1.0, 2.0, 3.0, 4.0, 5.0] {
            let plan = grid.plan(&keys, anchor).expect("档位应可达");
            assert_eq!(plan.steps, 0, "{anchor}× 应当一键到位");
            assert_eq!(plan.anchor, format!("{anchor:.0}"));
            assert_eq!(plan.rate, anchor);
        }
    }

    /// 档位之间的值：先按最近档位钉住，再用步进键补差额
    #[test]
    fn plans_anchor_then_steps() {
        let (grid, keys) = baidu();

        // 2.5 到 2 和 3 等距，取倍速小的那个（结果必须稳定，否则测试与真机都会飘）
        let up = grid.plan(&keys, 2.5).unwrap();
        assert_eq!((up.anchor.as_str(), up.step_key.as_str(), up.steps), ("2", ".", 5));
        assert_eq!(up.rate, 2.5);
        // 正是这一串在 40ms 下真机丢过一格，间隔必须随计划一起下发
        assert_eq!(up.gap_ms, grid.step_gap_ms);

        // 1.8 离 2 更近 → 从 2 往下退两格，比从 1 往上按八次少得多
        let down = grid.plan(&keys, 1.8).unwrap();
        assert_eq!((down.anchor.as_str(), down.step_key.as_str(), down.steps), ("2", ",", 2));
        assert_eq!(down.rate, 1.8);
    }

    /// 目标对齐到 0.1 网格（播放器的物理精度），正落在半格上时向上取
    #[test]
    fn snaps_target_to_grid() {
        let (grid, keys) = baidu();
        assert_eq!(grid.plan(&keys, 1.73).unwrap().rate, 1.7);
        assert_eq!(grid.plan(&keys, 1.77).unwrap().rate, 1.8);
        // 1.75 / 0.1 在浮点里是 17.4999…，不额外收敛就会掉到 1.7
        assert_eq!(grid.plan(&keys, 1.75).unwrap().rate, 1.8);
    }

    /// 超出播放器自身上下限时收敛到端点，而不是拒绝或按一长串键
    #[test]
    fn clamps_to_player_limits() {
        let (grid, keys) = baidu();
        let fast = grid.plan(&keys, 16.0).unwrap();
        assert_eq!((fast.anchor.as_str(), fast.steps, fast.rate), ("5", 0, 5.0));
        let slow = grid.plan(&keys, 0.05).unwrap();
        assert_eq!((slow.anchor.as_str(), slow.step_key.as_str(), slow.steps), ("1", ",", 5));
        assert_eq!(slow.rate, 0.5);
    }

    /// 全区间扫一遍：终值必须精确等于对齐后的目标，且不出现浮点尾数；
    /// 按键数最多 6（1 档位 + 5 步进）——这正是"锚点+步进"能替代回读的前提
    #[test]
    fn whole_range_is_exact_and_within_six_keys() {
        let (grid, keys) = baidu();
        for tenths in 5..=50 {
            let target = f64::from(tenths) / 10.0;
            let plan = grid.plan(&keys, target).unwrap();
            assert!(plan.steps <= 5, "{target}× 用了 {} 次步进", plan.steps);
            assert_eq!(
                (plan.rate * 10.0).round() as i64,
                i64::from(tenths),
                "{target}× 算出的终值是 {}",
                plan.rate
            );
            // 回读值会写进 Core.rate 并显示在 OSD 上，不能是 2.5000000000000004
            assert_eq!(plan.rate, (plan.rate * 10.0).round() / 10.0);
        }
    }

    /// 无档位 / 步长非法 / 目标非有限值：一律返回 None 让上层降级为步进，不 panic
    #[test]
    fn rejects_unusable_grids() {
        let (grid, keys) = baidu();
        let empty = KeyRateGrid {
            anchors: vec![],
            ..grid.clone()
        };
        assert!(empty.plan(&keys, 2.0).is_none());
        let zero_step = KeyRateGrid {
            step: 0.0,
            ..grid.clone()
        };
        assert!(zero_step.plan(&keys, 2.0).is_none());
        assert!(grid.plan(&keys, f64::NAN).is_none());
        assert!(grid.plan(&keys, f64::INFINITY).is_none());
    }

    /// 档位太稀疏时按键数会线性增长，超过上限宁可降级也不连按几十下
    #[test]
    fn refuses_when_too_many_keys() {
        let (grid, keys) = baidu();
        let sparse = KeyRateGrid {
            anchors: vec![RateAnchor {
                key: "1".into(),
                rate: 1.0,
            }],
            ..grid
        };
        assert!(sparse.plan(&keys, 2.0).is_some(), "差 10 格仍在上限内");
        assert!(sparse.plan(&keys, 5.0).is_none(), "差 40 格应当放弃");
    }

    /// 连发间隔属实测资产：百度网盘必须是实测过的 150ms，
    /// 没写这一项的自定义网格也要有个不贴着发的默认值，而不是 0
    #[test]
    fn step_gap_comes_from_the_grid() {
        let (grid, keys) = baidu();
        assert_eq!(grid.step_gap_ms, 150, "40ms 已被真机证伪，见 M4.8 取证");

        let slow = KeyRateGrid {
            step_gap_ms: 400,
            ..grid.clone()
        };
        assert_eq!(slow.plan(&keys, 2.5).unwrap().gap_ms, 400);

        // 自定义规则整条走 serde 往返，缺字段时不能落成「零间隔连发」
        let json = serde_json::to_value(&grid).unwrap();
        let mut bare = json.as_object().unwrap().clone();
        bare.remove("stepGapMs");
        let restored: KeyRateGrid = serde_json::from_value(bare.into()).unwrap();
        assert_eq!(restored.step_gap_ms, default_step_gap_ms());
        assert!(restored.step_gap_ms > 0);
    }

    /// 热键步进必须按整格走：默认 0.25 步长在 0.1 网格上要落到 1.3 而不是 1.25，
    /// 否则 OSD 先报一个播放器给不出的值，再被回读改口
    #[test]
    fn hotkey_steps_land_on_the_grid() {
        let (grid, _) = baidu();
        assert_eq!(grid.step_target(1.0, 0.25, 1), Some(1.3));
        assert_eq!(grid.step_target(1.3, 0.25, -1), Some(1.0));
        // 步长恰为一格时不该被放大
        assert_eq!(grid.step_target(2.0, 0.1, 1), Some(2.1));
    }

    /// 步长不足半格时（配置文件被手改成 0.05），就近取整会把目标吸回原值，
    /// 热键从此按下去没反应也不报错——必须保底挪一格
    #[test]
    fn sub_grid_step_still_moves() {
        let (grid, _) = baidu();
        assert_eq!(grid.step_target(1.1, 0.05, -1), Some(1.0), "不能原地不动");
        assert_eq!(grid.step_target(1.1, 0.05, 1), Some(1.2));
    }

    /// 目标越界时收敛到播放器自己的端点，OSD 不该显示它到不了的 5.2×
    #[test]
    fn hotkey_steps_stop_at_player_limits() {
        let (grid, _) = baidu();
        assert_eq!(grid.step_target(5.0, 0.5, 1), Some(5.0));
        assert_eq!(grid.step_target(0.5, 0.5, -1), Some(0.5));
    }

    /// 内置规则接线：百度网盘走按键通道且带网格，展示文案要与"只能步进"区分开
    #[test]
    fn baidu_rule_is_wired_for_exact_keys() {
        let rules = built_in_rules();
        let rule = rules.iter().find(|r| r.id == "baidu-netdisk").unwrap();
        assert_eq!(rule.kind, AppKind::Client);
        assert_eq!(rule.method, RuleMethod::Hotkey);
        assert_eq!(rule.ipc, IpcKind::None); // 无 <video>，CDP 那条路走不通
        assert_eq!(rule.status(), AppStatus::Adapted);
        assert_eq!(rule.method_label(), "播放器快捷键 · 精确设速");
        assert!(rule.matches("baidunetdiskunite.exe"));
        assert!(rule.matches("baidunetdisk.exe"));

        // 只能步进的播放器不应被误标成精确设速
        let mpc = rules.iter().find(|r| r.id == "mpc-hc").unwrap();
        assert!(mpc.key_rate.is_none());
    }

    /// 用户配置不得覆盖内置规则的倍速网格与档位表（同 process/kind，属代码资产）
    #[test]
    fn saved_config_cannot_clobber_builtin_grid() {
        let mut rules = built_in_rules();
        let mut stale = rules.iter().find(|r| r.id == "baidu-netdisk").unwrap().clone();
        stale.key_rate = None;
        stale.keys = Some(KeyBindings { up: "]".into(), down: "[".into(), reset: "=".into() });
        let mut stale_mpc = rules.iter().find(|r| r.id == "mpc-hc").unwrap().clone();
        stale_mpc.rate_ladder = None;
        merge_saved(&mut rules, vec![stale, stale_mpc]);

        let rule = rules.iter().find(|r| r.id == "baidu-netdisk").unwrap();
        assert!(rule.key_rate.is_some(), "网格来自代码，不该被旧配置抹掉");
        assert_eq!(rule.keys.as_ref().unwrap().up, "]", "键位仍应尊重用户改动");
        let mpc = rules.iter().find(|r| r.id == "mpc-hc").unwrap();
        assert!(mpc.rate_ladder.is_some(), "档位表同属代码资产，不该被旧配置抹掉");
    }

    fn rule(id: &str) -> AppRule {
        built_in_rules().into_iter().find(|r| r.id == id).unwrap_or_else(|| panic!("缺内置规则 {id}"))
    }

    /// VLC 的键位与网格逐条对应官方源码（`hotkeys.c` 的 AdjustRateFine +
    /// `libvlc-module.c` 的默认键位），改动这几个数字前先回源码核对
    #[test]
    fn vlc_grid_matches_the_source() {
        let vlc = rule("vlc");
        let keys = vlc.keys.clone().expect("VLC 应带按键");
        assert_eq!((keys.up.as_str(), keys.down.as_str(), keys.reset.as_str()), ("]", "[", "="));

        let grid = vlc.key_rate.clone().expect("VLC 应带倍速网格");
        assert_eq!(grid.step, 0.1, "AdjustRateFine 是 0.1 的整格步进");
        // ACTIONID_RATE_NORMAL 直接 var_SetFloat(rate, 1.f)，是 VLC 唯一的绝对锚点
        assert_eq!(grid.anchors.len(), 1);
        assert_eq!((grid.anchors[0].key.as_str(), grid.anchors[0].rate), ("=", 1.0));

        // 连发间隔是实测资产，不是默认值：VLC 3.0.23 上 0–40ms 连发 11 次一格没丢，
        // 与百度网盘那条异步读回的通道不是一回事，别再套用 150ms
        assert_eq!(grid.step_gap_ms, 20);
        assert_ne!(grid.step_gap_ms, default_step_gap_ms(), "已标定过，不该再退回默认值");

        // 1× 一键到位；两侧各能精确走到边界，全程不超过 MAX_KEYS
        assert_eq!(grid.plan(&keys, 1.0).unwrap().steps, 0);
        let up = grid.plan(&keys, 1.5).unwrap();
        assert_eq!((up.anchor.as_str(), up.step_key.as_str(), up.steps, up.rate), ("=", "]", 5, 1.5));
        let down = grid.plan(&keys, 0.6).unwrap();
        assert_eq!((down.step_key.as_str(), down.steps, down.rate), ("[", 4, 0.6));
    }

    /// 网格区间必须写「按键真够得着」的范围而不是 VLC 的理论区间：
    /// 单锚点 + MAX_KEYS=12 决定了精确半径只有 ±1.1，写宽了 plan() 会放弃、
    /// 退回开环单步——OSD 谎报正是从那里来的
    #[test]
    fn vlc_range_is_what_the_keys_can_actually_reach() {
        let vlc = rule("vlc");
        let keys = vlc.keys.clone().unwrap();
        let grid = vlc.key_rate.clone().unwrap();
        assert_eq!((grid.min, grid.max), (0.25, 2.1));

        // 区间内每一档都必须算得出计划，且终值精确等于对齐后的目标
        for tenths in 3..=21 {
            let target = f64::from(tenths) / 10.0;
            let plan = grid.plan(&keys, target).unwrap_or_else(|| panic!("{target}× 应可达"));
            assert_eq!((plan.rate * 10.0).round() as i64, i64::from(tenths));
        }
        // 超出区间收敛到端点，而不是返回 None 让上层瞎按
        assert_eq!(grid.plan(&keys, 6.0).unwrap().rate, 2.1);
    }

    /// MPC-HC 接线：档位表来自绝对倍速命令表，且每一档都能查到命令码
    #[test]
    fn mpc_hc_is_wired_to_the_absolute_rate_commands() {
        let mpc = rule("mpc-hc");
        assert_eq!(mpc.ipc, IpcKind::WmCommand);
        assert_eq!(mpc.method_label(), "控制消息 · 档位设速");
        let ladder = mpc.rate_ladder.clone().expect("MPC-HC 应带档位表");
        assert_eq!(ladder, mpc_hc::rate_ladder());
        for rung in &ladder {
            assert!(mpc_hc::command_for(*rung).is_some(), "{rung}× 查不到命令码");
        }
        // 就近取档的结果必须仍在表内——适配器拿它去查命令码，落在档间就发不出去
        for target in [0.1, 1.0, 1.2, 2.4, 2.5, 7.0, 99.0] {
            let snapped = snap_to_ladder(&ladder, target).unwrap();
            assert!(mpc_hc::command_for(snapped).is_some(), "{target}× 取到的档不在表内");
        }
        assert_eq!(snap_to_ladder(&ladder, 2.5), Some(2.0), "同距取小，结果要稳定");
        assert_eq!(snap_to_ladder(&ladder, 99.0), Some(8.0), "越界收到端点");
        assert_eq!(snap_to_ladder(&ladder, f64::NAN), None);
    }

    /// 热键沿档位表走：一次一档。档距不均匀（2 的上一档是 3 不是 2.25），
    /// 按 rate ± step 算会得到表里没有的值，下发时被吸回原档 —— 热键就此卡死
    #[test]
    fn mpc_hc_hotkey_walks_one_rung() {
        let mpc = rule("mpc-hc");
        assert_eq!(mpc.ladder_step(1.0, 1), Some(1.1));
        assert_eq!(mpc.ladder_step(1.1, 1), Some(1.25));
        assert_eq!(mpc.ladder_step(1.0, -1), Some(0.9));
        assert_eq!(mpc.ladder_step(2.0, 1), Some(3.0), "2× 之后是 3×，不是 2.25×");
        // 不在档上时取该方向第一个越过它的档
        assert_eq!(mpc.ladder_step(2.4, 1), Some(3.0));
        assert_eq!(mpc.ladder_step(2.4, -1), Some(2.0));
        // 端点停住：连按到顶不该绕回最低档
        assert_eq!(mpc.ladder_step(8.0, 1), Some(8.0));
        assert_eq!(mpc.ladder_step(0.25, -1), Some(0.25));
        assert_eq!(mpc.ladder_step(f64::NAN, 1), None);
        assert_eq!(mpc.ladder_step(1.0, 0), None);
        // 没有档位表的规则不该走这条路
        assert_eq!(rule("vlc").ladder_step(1.0, 1), None);
    }

    /// MPC-BE 必须与 MPC-HC 分家：类名、命令集都不同，
    /// 绝对倍速码发给 BE 是静默 no-op —— 混作 alias 就会「申报成功但倍速没动」
    #[test]
    fn mpc_be_is_a_separate_rule_without_the_ladder() {
        let be = rule("mpc-be");
        assert!(be.rate_ladder.is_none(), "BE 源码里没有 PLAYBACKRATE 命令");
        assert!(be.matches("mpc-be64.exe") && be.matches("mpc-be.exe"));

        let hc = rule("mpc-hc");
        assert!(hc.matches("mpc-hc64.exe") && hc.matches("mpc-hc.exe"));
        assert!(!hc.matches("mpc-be64.exe"), "BE 不能再被 MPC-HC 规则吃掉");
        assert!(!hc.matches("mpc-be.exe"));
    }

    /// 回读判据决定热键要不要先自我量化。判错的代价是双向的：
    /// 该量化没量化 → OSD 报播放器给不出的值；不该量化却量化 → 步长被网格撑大
    #[test]
    fn read_back_capability_is_per_channel() {
        for id in ["mpv", "vlc", "potplayer", "bilibili-client"] {
            assert!(rule(id).can_read_back_rate(), "{id} 有回读通道");
        }
        // MPC 系的 WM_COMMAND 一条回读也没有；百度网盘压根没有 IPC
        for id in ["mpc-hc", "mpc-be", "baidu-netdisk", "chrome"] {
            assert!(!rule(id).can_read_back_rate(), "{id} 不该被当作可回读");
        }
        // 用户把控制方式改成「仅快捷键」后 IPC 不再参与，回读也就没了
        let mut vlc = rule("vlc");
        vlc.method = RuleMethod::Hotkey;
        assert!(!vlc.can_read_back_rate());
    }
}
