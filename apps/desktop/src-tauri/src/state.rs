//! M1 核心状态：当前倍速、步长与全局快捷键配置。
//! 倍速的权威值保存在 Rust 侧（热键在主窗口隐藏时也要工作）；
//! M2 起由 router/adapters 负责把倍速真正下发到目标应用。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri_plugin_global_shortcut::Shortcut;

/// 浏览器内核倍速硬下限/上限（开发文档 §2.1，与前端 store.ts 保持一致）
pub const RATE_MIN: f64 = 0.25;
pub const RATE_MAX: f64 = 16.0;

/// 快捷键动作，serde 名称与前端 `ShortcutId` 一一对应
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShortcutAction {
    SpeedUp,
    SpeedDown,
    Reset,
    PlayPause,
    TogglePanel,
}

impl ShortcutAction {
    pub const ALL: [ShortcutAction; 5] = [
        ShortcutAction::SpeedUp,
        ShortcutAction::SpeedDown,
        ShortcutAction::Reset,
        ShortcutAction::PlayPause,
        ShortcutAction::TogglePanel,
    ];
}

/// 组合键使用前端的展示格式存储（如 ["Ctrl","Alt","↑"]），
/// 注册时由 hotkey::parse_combo 转换为系统热键。
pub type ShortcutMap = HashMap<ShortcutAction, Vec<String>>;

pub fn default_shortcuts() -> ShortcutMap {
    let combo = |keys: &[&str]| keys.iter().map(|k| k.to_string()).collect::<Vec<_>>();
    HashMap::from([
        (ShortcutAction::SpeedUp, combo(&["Ctrl", "Alt", "↑"])),
        (ShortcutAction::SpeedDown, combo(&["Ctrl", "Alt", "↓"])),
        (ShortcutAction::Reset, combo(&["Ctrl", "Alt", "0"])),
        (ShortcutAction::PlayPause, combo(&["Ctrl", "Alt", "Space"])),
        (ShortcutAction::TogglePanel, combo(&["Ctrl", "Alt", "S"])),
    ])
}

pub struct Core {
    /// 当前目标倍速（M1 为应用内状态，M2 起下发到适配器）
    pub rate: f64,
    /// 快捷键步长（由前端设置页同步）
    pub step: f64,
    /// 步进/滑块的当前上限（由前端设置页同步）
    pub slider_max: f64,
    pub hotkeys_enabled: bool,
    pub shortcuts: ShortcutMap,
    /// 注册失败的快捷键 → 用户可读的冲突原因（快捷键页行内标红）
    pub conflicts: HashMap<ShortcutAction, String>,
    /// 当前已注册的系统热键 → 动作，供热键回调反查
    pub registered: Vec<(Shortcut, ShortcutAction)>,
    /// OSD 显示代数：防止旧的延时隐藏任务盖掉新一次提示
    pub osd_seq: u64,
}

impl Default for Core {
    fn default() -> Self {
        Self {
            rate: 1.0,
            step: 0.25,
            slider_max: 6.0,
            hotkeys_enabled: true,
            shortcuts: default_shortcuts(),
            conflicts: HashMap::new(),
            registered: Vec::new(),
            osd_seq: 0,
        }
    }
}

impl Core {
    pub fn snapshot(&self) -> CoreSnapshot {
        CoreSnapshot {
            rate: self.rate,
            step: self.step,
            slider_max: self.slider_max,
            hotkeys_enabled: self.hotkeys_enabled,
            shortcuts: self.shortcuts.clone(),
            conflicts: self.conflicts.clone(),
        }
    }
}

pub type CoreState = Mutex<Core>;

/// 前端初始化时一次性拉取的核心状态
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CoreSnapshot {
    pub rate: f64,
    pub step: f64,
    pub slider_max: f64,
    pub hotkeys_enabled: bool,
    pub shortcuts: ShortcutMap,
    pub conflicts: HashMap<ShortcutAction, String>,
}

/// 倍速统一收口：夹到 [0.25, min(max, 16)] 并保留两位小数（与前端 clampRate 一致）
pub fn clamp_rate(rate: f64, max: f64) -> f64 {
    (rate.clamp(RATE_MIN, max.min(RATE_MAX)) * 100.0).round() / 100.0
}
