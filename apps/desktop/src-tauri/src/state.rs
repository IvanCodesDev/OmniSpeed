//! 核心状态：当前倍速、设置项、全局快捷键配置（M1），
//! 应用规则表与前台接管对象（M2），以及按应用/网站的倍速记忆（M4）。
//! 倍速的权威值保存在 Rust 侧（热键在主窗口隐藏时也要工作），
//! 由 router/adapters 负责把倍速真正下发到目标应用。

use crate::rules::{AppKind, AppRule, AppStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use tauri_plugin_global_shortcut::Shortcut;

/// 浏览器内核倍速硬下限/上限（开发文档 §2.1，与前端 store.ts 保持一致）
pub const RATE_MIN: f64 = 0.25;
pub const RATE_MAX: f64 = 16.0;

/// 设置页全部选项（开发文档 §8 settings 节）。
/// serde 名与前端 store.ts 的 Settings 一一对应，Rust 侧为权威、持久化于 config.json。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 应用启动时的初始倍速
    pub default_rate: f64,
    pub step: f64,
    /// 滑块显示上限（约束 UI 与桌面播放器热键；浏览器热键上限见 hotkey_rate_cap）
    pub slider_max: f64,
    /// 控制页预设档位
    pub presets: Vec<f64>,
    /// >4× 时 OSD 附带「浏览器已静音」提示（开发文档 §7.8）
    pub high_speed_warning: bool,
    /// 高倍速缓冲不足时自动回落（开发文档 §7.8）
    pub smart_slowdown: bool,
    /// 变速不变调（下发给浏览器扩展）
    pub preserves_pitch: bool,
    /// 按应用/网站记忆倍速并在切回时恢复（开发文档 §7.5）
    pub remember_per_app: bool,
    pub start_on_boot: bool,
    pub minimize_to_tray: bool,
    pub auto_update: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_rate: 1.0,
            step: 0.25,
            slider_max: 6.0,
            presets: vec![1.0, 1.5, 2.0, 3.0, 4.0, 5.0],
            high_speed_warning: true,
            smart_slowdown: false,
            preserves_pitch: true,
            remember_per_app: true,
            start_on_boot: true,
            minimize_to_tray: true,
            auto_update: true,
        }
    }
}

impl Settings {
    /// 收口非法值。前端已做约束，这里防御被手改的持久化文件
    pub fn normalize(&mut self) {
        self.default_rate = clamp_rate(self.default_rate, RATE_MAX);
        self.step = if self.step.is_finite() { self.step.clamp(0.05, 1.0) } else { 0.25 };
        self.slider_max = if self.slider_max.is_finite() {
            self.slider_max.clamp(1.0, RATE_MAX)
        } else {
            6.0
        };
        self.presets.retain(|p| p.is_finite());
        for p in &mut self.presets {
            *p = clamp_rate(*p, RATE_MAX);
        }
        self.presets.sort_by(|a, b| a.partial_cmp(b).expect("presets 已过滤非有限值"));
        self.presets.dedup();
        if self.presets.is_empty() {
            self.presets = Self::default().presets;
        }
    }
}

/// 站点级规则（M3.5，「应用页 · 网站适配」；开发文档 §8 siteRules / §9 命令）。
/// serde 名与前端 store.ts 的 SiteRule 一一对应，host 为规则键（对 location.host 后缀匹配）。
/// rateLock / maxRate / follow 经 nm_bridge 下发扩展生效；defaultRate 用于进站恢复。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SiteRule {
    pub host: String,
    pub name: String,
    /// 进入该站且无按站记忆时的恢复倍速；None = 跟随全局（不主动改写）
    pub default_rate: Option<f64>,
    /// 站点倍速上限（扩展侧与全局上限取小）
    pub max_rate: f64,
    /// 站点级倍速锁定（扩展侧与全局锁定相与）
    pub rate_lock: bool,
    /// 短视频流/新出现媒体是否跟随会话倍速
    pub follow: bool,
}

impl Default for SiteRule {
    fn default() -> Self {
        Self {
            host: String::new(),
            name: String::new(),
            default_rate: None,
            max_rate: RATE_MAX,
            rate_lock: true,
            follow: true,
        }
    }
}

impl SiteRule {
    /// 收口非法值（同 Settings::normalize，防御手改的持久化文件）
    pub fn normalize(&mut self) {
        self.host = self.host.trim().to_lowercase();
        self.max_rate = if self.max_rate.is_finite() {
            self.max_rate.clamp(RATE_MIN, RATE_MAX)
        } else {
            RATE_MAX
        };
        self.default_rate =
            self.default_rate.filter(|r| r.is_finite()).map(|r| clamp_rate(r, self.max_rate));
    }
}

/// 内置站点规则：与扩展 sites/ 注册表的 v1.0 首发 8 站一一对应。
/// 长视频平台（腾讯/爱奇艺/优酷）默认关闭「新片跟随」：连播下一集回到常速更符合预期
pub fn default_site_rules() -> Vec<SiteRule> {
    let rule = |host: &str, name: &str, follow: bool| SiteRule {
        host: host.into(),
        name: name.into(),
        follow,
        ..SiteRule::default()
    };
    vec![
        rule("bilibili.com", "哔哩哔哩", true),
        rule("douyin.com", "抖音", true),
        rule("youtube.com", "YouTube", true),
        rule("v.qq.com", "腾讯视频", false),
        rule("iqiyi.com", "爱奇艺", false),
        rule("youku.com", "优酷", false),
        rule("ixigua.com", "西瓜视频", true),
        rule("kuaishou.com", "快手", true),
    ]
}

/// host 后缀匹配："bilibili.com" 命中 "www.bilibili.com"，不误伤 "xbilibili.com"
pub fn host_matches(host: &str, rule_host: &str) -> bool {
    host == rule_host
        || (host.len() > rule_host.len()
            && host.ends_with(rule_host)
            && host.as_bytes()[host.len() - rule_host.len() - 1] == b'.')
}

/// 持久化合并：内置表以代码为准，存量覆盖可编辑字段；未知 host 追加为自定义规则
pub fn merge_saved_site_rules(rules: &mut Vec<SiteRule>, saved: Vec<SiteRule>) {
    for mut s in saved {
        s.normalize();
        if s.host.is_empty() {
            continue;
        }
        if let Some(existing) = rules.iter_mut().find(|r| r.host == s.host) {
            existing.default_rate = s.default_rate;
            existing.max_rate = s.max_rate;
            existing.rate_lock = s.rate_lock;
            existing.follow = s.follow;
        } else {
            rules.push(s);
        }
    }
}

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

/// 前台匹配到的接管对象（monitor 写入，router 据此选择适配器）
#[derive(Debug, Clone)]
pub struct CurrentTarget {
    pub rule_id: String,
    pub hwnd: isize,
    pub process_name: String,
}

pub struct Core {
    /// 当前目标倍速（下发到适配器的依据；适配器可回读时以回读值校正）
    pub rate: f64,
    /// 设置页选项（权威值，含步长/滑块上限；持久化于 config.json）
    pub settings: Settings,
    pub hotkeys_enabled: bool,
    pub shortcuts: ShortcutMap,
    /// 注册失败的快捷键 → 用户可读的冲突原因（快捷键页行内标红）
    pub conflicts: HashMap<ShortcutAction, String>,
    /// 当前已注册的系统热键 → 动作，供热键回调反查
    pub registered: Vec<(Shortcut, ShortcutAction)>,
    /// OSD 显示代数：防止旧的延时隐藏任务盖掉新一次提示
    pub osd_seq: u64,
    /// 全局监听开关（控制页右上角）：关闭时不跟随前台切换、不下发控制
    pub listening: bool,
    /// 应用规则表（内置 + 用户覆盖/自定义）
    pub rules: Vec<AppRule>,
    /// 当前前台匹配到的接管对象
    pub current: Option<CurrentTarget>,
    /// 扩展已连接的浏览器（NM 桥维护，键为小写进程名如 "msedge.exe"）
    pub connected_browsers: HashSet<String>,
    /// 各浏览器活动标签页的媒体状态（NM 桥写入，键同上）
    pub browser_media: HashMap<String, BrowserMedia>,
    /// 站点级规则（M3.5）：内置 8 站 + 用户自定义，持久化于 config.json
    pub site_rules: Vec<SiteRule>,
    /// 按应用/网站记忆的倍速（开发文档 §7.5）：桌面软件按进程名、网页按 host
    pub memory: HashMap<String, f64>,
    /// 记忆变更代数：防抖持久化用（见 persist::save_memory_debounced）
    pub memory_seq: u64,
    /// 最近一次智能降速时刻（8s 冷却，避免与缓冲恢复震荡）
    pub slowdown_at: Option<std::time::Instant>,
}

/// 浏览器扩展上报的活动标签页媒体状态（协议见 apps/extension/src/shared/protocol.ts）。
/// 按协议完整映射：ad_playing / buffered_ahead / adapter 供 M3.5 的 OSD
/// 广告提示与高倍速缓冲警示消费（开发文档 §7.8），当前允许未读
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct BrowserMedia {
    pub has_media: bool,
    pub rate: f64,
    pub title: String,
    pub host: String,
    pub is_live: bool,
    pub ad_playing: bool,
    pub buffered_ahead: Option<f64>,
    pub adapter: String,
}

impl Default for Core {
    fn default() -> Self {
        Self {
            rate: 1.0,
            settings: Settings::default(),
            hotkeys_enabled: true,
            shortcuts: default_shortcuts(),
            conflicts: HashMap::new(),
            registered: Vec::new(),
            osd_seq: 0,
            listening: true,
            rules: crate::rules::built_in_rules(),
            current: None,
            connected_browsers: HashSet::new(),
            browser_media: HashMap::new(),
            site_rules: default_site_rules(),
            memory: HashMap::new(),
            memory_seq: 0,
            slowdown_at: None,
        }
    }
}

impl Core {
    pub fn snapshot(&self) -> CoreSnapshot {
        CoreSnapshot {
            rate: self.rate,
            hotkeys_enabled: self.hotkeys_enabled,
            shortcuts: self.shortcuts.clone(),
            conflicts: self.conflicts.clone(),
            listening: self.listening,
            settings: self.settings.clone(),
        }
    }

    /// 当前接管对象对应的规则
    pub fn current_rule(&self) -> Option<&AppRule> {
        let target = self.current.as_ref()?;
        self.rules.iter().find(|r| r.id == target.rule_id)
    }

    /// host 命中的站点规则（后缀匹配，M3.5）
    pub fn site_rule_for(&self, host: &str) -> Option<&SiteRule> {
        self.site_rules.iter().find(|r| host_matches(host, &r.host))
    }

    /// 记忆键（开发文档 §7.5）：网页按活动标签页的 host 细分，桌面软件按进程名。
    /// 浏览器尚无媒体上报时退回进程名，保证「有接管对象就有记忆位置」
    pub fn memory_key(&self) -> Option<String> {
        let target = self.current.as_ref()?;
        let rule = self.current_rule()?;
        if rule.kind == AppKind::Browser {
            if let Some(media) = self
                .browser_media
                .get(&target.process_name)
                .filter(|m| m.has_media && !m.host.is_empty())
            {
                return Some(media.host.clone());
            }
        }
        Some(target.process_name.clone())
    }

    /// 用户主动调速后记录到记忆表（热键 / 滑块 / 预设 / 页面内调速）。
    /// 返回 true 表示记忆有变化，调用方应安排防抖持久化
    pub fn remember_rate(&mut self) -> bool {
        if !self.settings.remember_per_app {
            return false;
        }
        let Some(key) = self.memory_key() else { return false };
        if self.memory.get(&key).is_some_and(|r| (r - self.rate).abs() < 0.001) {
            return false;
        }
        self.memory.insert(key, self.rate);
        self.memory_seq += 1;
        true
    }

    /// 组装控制页「当前媒体」会话（事件 media:changed / 命令 get_current_media 共用）。
    /// 监听暂停时对外表现为无媒体。播放器的 rate 由调用方按适配器回读能力填充；
    /// 浏览器目标以扩展上报为准（标题/站点/真实倍速，开发文档 §3「非对称控制」）。
    pub fn current_session(&self, rate: Option<f64>) -> Option<MediaSession> {
        if !self.listening {
            return None;
        }
        let target = self.current.as_ref()?;
        let rule = self.current_rule()?;

        if rule.kind == AppKind::Browser {
            let connected = self.connected_browsers.contains(&target.process_name);
            let media = self
                .browser_media
                .get(&target.process_name)
                .filter(|m| m.has_media);
            return Some(MediaSession {
                app_id: rule.id.clone(),
                name: media.map(|m| m.title.clone()).unwrap_or_else(|| rule.name.clone()),
                source: media
                    .map(|m| m.host.clone())
                    .unwrap_or_else(|| target.process_name.clone()),
                kind: rule.kind,
                status: if connected { AppStatus::Connected } else { AppStatus::NeedsSetup },
                rate: media.map(|m| m.rate),
                can_read_back: media.is_some(),
            });
        }

        Some(MediaSession {
            app_id: rule.id.clone(),
            name: rule.name.clone(),
            source: target.process_name.clone(),
            kind: rule.kind,
            status: rule.status(),
            rate,
            can_read_back: rate.is_some(),
        })
    }
}

pub type CoreState = Mutex<Core>;

/// 前端初始化时一次性拉取的核心状态
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CoreSnapshot {
    pub rate: f64,
    pub hotkeys_enabled: bool,
    pub shortcuts: ShortcutMap,
    pub conflicts: HashMap<ShortcutAction, String>,
    pub listening: bool,
    pub settings: Settings,
}

/// 控制页「当前媒体」会话 DTO（契约见前端 lib/ipc.ts MediaSession）
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MediaSession {
    pub app_id: String,
    pub name: String,
    pub source: String,
    pub kind: AppKind,
    pub status: AppStatus,
    /// 适配器可回读时为真实倍速，否则 null（前端显示目标倍速）
    pub rate: Option<f64>,
    pub can_read_back: bool,
}

/// 倍速统一收口：夹到 [0.25, min(max, 16)] 并保留两位小数（与前端 clampRate 一致）
pub fn clamp_rate(rate: f64, max: f64) -> f64 {
    (rate.clamp(RATE_MIN, max.min(RATE_MAX)) * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_rate_bounds_and_rounding() {
        assert_eq!(clamp_rate(0.1, 6.0), 0.25);
        assert_eq!(clamp_rate(7.0, 6.0), 6.0);
        assert_eq!(clamp_rate(20.0, 99.0), 16.0);
        assert_eq!(clamp_rate(1.2345, 16.0), 1.23);
    }

    #[test]
    fn settings_deserialize_missing_fields_fall_back_to_defaults() {
        // 旧版本 config.json 只有部分字段时，其余取默认值
        let s: Settings = serde_json::from_str(r#"{ "step": 0.5, "autoUpdate": false }"#).unwrap();
        assert_eq!(s.step, 0.5);
        assert!(!s.auto_update);
        assert_eq!(s.slider_max, 6.0);
        assert!(s.remember_per_app);
    }

    #[test]
    fn settings_normalize_repairs_broken_values() {
        let mut s = Settings {
            step: f64::NAN,
            slider_max: 99.0,
            presets: vec![f64::INFINITY, 3.0, 3.0, 0.01],
            default_rate: -5.0,
            ..Settings::default()
        };
        s.normalize();
        assert_eq!(s.step, 0.25);
        assert_eq!(s.slider_max, 16.0);
        assert_eq!(s.presets, vec![0.25, 3.0]);
        assert_eq!(s.default_rate, 0.25);
    }

    fn core_with_target(process: &str, rule_id: &str) -> Core {
        let mut core = Core::default();
        core.current = Some(CurrentTarget {
            rule_id: rule_id.into(),
            hwnd: 0,
            process_name: process.into(),
        });
        core
    }

    #[test]
    fn memory_key_uses_process_for_players_and_host_for_browsers() {
        let core = core_with_target("mpv.exe", "mpv");
        assert_eq!(core.memory_key().as_deref(), Some("mpv.exe"));

        // 浏览器无媒体上报 → 退回进程名
        let mut core = core_with_target("msedge.exe", "edge");
        assert_eq!(core.memory_key().as_deref(), Some("msedge.exe"));

        // 有媒体上报 → 按 host 细分
        core.browser_media.insert(
            "msedge.exe".into(),
            BrowserMedia {
                has_media: true,
                rate: 2.0,
                title: "t".into(),
                host: "bilibili.com".into(),
                is_live: false,
                ad_playing: false,
                buffered_ahead: None,
                adapter: "generic".into(),
            },
        );
        assert_eq!(core.memory_key().as_deref(), Some("bilibili.com"));
    }

    #[test]
    fn host_matches_is_suffix_only_at_label_boundary() {
        assert!(host_matches("bilibili.com", "bilibili.com"));
        assert!(host_matches("www.bilibili.com", "bilibili.com"));
        assert!(host_matches("live.bilibili.com", "bilibili.com"));
        assert!(!host_matches("xbilibili.com", "bilibili.com"));
        assert!(!host_matches("bilibili.com.evil.com", "bilibili.com"));
        assert!(host_matches("v.qq.com", "v.qq.com"));
        assert!(!host_matches("weixin.qq.com", "v.qq.com"));
    }

    #[test]
    fn merge_saved_site_rules_overrides_builtin_and_appends_custom() {
        let mut rules = default_site_rules();
        merge_saved_site_rules(
            &mut rules,
            vec![
                SiteRule {
                    host: "Bilibili.com ".into(), // 手改大小写/空白也能对上内置项
                    name: "改名不生效".into(),
                    default_rate: Some(2.0),
                    max_rate: 99.0, // 越界收口到 16
                    rate_lock: false,
                    follow: false,
                },
                SiteRule { host: "example.com".into(), name: "自定义".into(), ..SiteRule::default() },
                SiteRule { host: "  ".into(), ..SiteRule::default() }, // 空 host 丢弃
            ],
        );
        let b = rules.iter().find(|r| r.host == "bilibili.com").unwrap();
        assert_eq!(b.name, "哔哩哔哩"); // 内置名以代码为准
        assert_eq!(b.default_rate, Some(2.0));
        assert_eq!(b.max_rate, 16.0);
        assert!(!b.rate_lock);
        assert!(!b.follow);
        assert_eq!(rules.len(), default_site_rules().len() + 1);
        assert!(rules.iter().any(|r| r.host == "example.com"));
    }

    #[test]
    fn site_rule_for_matches_current_host() {
        let core = Core::default();
        assert_eq!(core.site_rule_for("www.iqiyi.com").map(|r| r.host.as_str()), Some("iqiyi.com"));
        assert_eq!(core.site_rule_for("v.qq.com").map(|r| r.host.as_str()), Some("v.qq.com"));
        assert!(core.site_rule_for("example.org").is_none());
    }

    #[test]
    fn remember_rate_respects_toggle_and_dedups() {
        let mut core = core_with_target("mpv.exe", "mpv");
        core.rate = 2.0;
        assert!(core.remember_rate());
        assert_eq!(core.memory.get("mpv.exe"), Some(&2.0));
        // 同值不重复记录（避免无谓的持久化）
        assert!(!core.remember_rate());

        core.settings.remember_per_app = false;
        core.rate = 3.0;
        assert!(!core.remember_rate());
        assert_eq!(core.memory.get("mpv.exe"), Some(&2.0));
    }
}
