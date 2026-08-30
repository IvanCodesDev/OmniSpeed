//! 应用规则：内置播放器规则表、用户覆盖与前端 DTO（开发文档 §8 appRules）。
//! 规则回答三个问题：这个进程是谁、用什么通道控制它（IPC 优先 / 按键兜底）、按键是什么。
//! M2 内置 mpv / VLC / PotPlayer / MPC-HC 四家（开发文档 §11 M2 行）；
//! 浏览器条目仅占位展示，真实接管等 M3 扩展接入。

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
    pub builtin: bool,
}

impl AppRule {
    pub fn matches(&self, process_name: &str) -> bool {
        self.process == process_name || self.aliases.iter().any(|a| a == process_name)
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
            RuleMethod::Hotkey => "快捷键".into(),
            RuleMethod::Ipc if self.ipc == IpcKind::Cdp => "CDP 接管".into(),
            RuleMethod::Ipc => "IPC 接口".into(),
            RuleMethod::Auto => {
                if self.ipc == IpcKind::Cdp {
                    "CDP 接管".into()
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
            keys: keys("C", "X", "Z"),
            builtin: true,
        },
        AppRule {
            id: "mpc-hc".into(),
            name: "MPC-HC".into(),
            process: "mpc-hc64.exe".into(),
            aliases: vec!["mpc-hc.exe".into(), "mpc-be64.exe".into(), "mpc-be.exe".into()],
            kind: AppKind::Player,
            method: RuleMethod::Auto,
            ipc: IpcKind::WmCommand,
            ipc_config: None,
            keys: keys("Ctrl+Up", "Ctrl+Down", "R"),
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
