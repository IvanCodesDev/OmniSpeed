//! 全局快捷键：前端组合键 → 系统热键注册、冲突检测与动作分发（开发文档 §7.2）。
//! 冲突检测策略：RegisterHotKey 注册失败即判定被系统或其他程序占用。

use crate::osd;
use crate::router::{self, PushMode};
use crate::rules::AppKind;
use crate::state::{clamp_rate, Core, CoreState, ShortcutAction, RATE_MAX};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutEvent, ShortcutState};

/// 热键触发事件载荷（事件名 `hotkey:triggered`，开发文档 §9）
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyPayload {
    pub action: ShortcutAction,
    pub rate: f64,
    pub seq: u64,
    /// OSD 副文案（如 >4× 的浏览器静音提示，开发文档 §7.8）
    pub notice: Option<String>,
}

/// 前端修饰键 → 插件修饰键
fn modifier_token(key: &str) -> Option<&'static str> {
    Some(match key {
        "Ctrl" => "Control",
        "Alt" => "Alt",
        "Shift" => "Shift",
        "Win" => "Super",
        _ => return None,
    })
}

/// 前端主键（展示格式）→ keyboard-types 的 Code 名称
fn key_token(key: &str) -> Option<String> {
    let named = match key {
        "↑" => "ArrowUp",
        "↓" => "ArrowDown",
        "←" => "ArrowLeft",
        "→" => "ArrowRight",
        "=" => "Equal",
        "-" => "Minus",
        "," => "Comma",
        "." => "Period",
        "/" => "Slash",
        ";" => "Semicolon",
        "'" => "Quote",
        "[" => "BracketLeft",
        "]" => "BracketRight",
        "\\" => "Backslash",
        "`" => "Backquote",
        // e.key 原生名称与 Code 名称一致的直接放行
        "Space" | "Enter" | "Tab" | "Backspace" | "Home" | "End" | "PageUp" | "PageDown"
        | "Insert" | "Delete" => key,
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => {
            return Some(format!("Digit{k}"));
        }
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_uppercase()) => {
            return Some(format!("Key{k}"));
        }
        // F1–F24
        k if k.starts_with('F') && k[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => k,
        _ => return None,
    };
    Some(named.to_string())
}

/// ["Ctrl","Alt","↑"] → "Control+Alt+ArrowUp" → 系统热键
pub fn parse_combo(combo: &[String]) -> Result<Shortcut, String> {
    if combo.len() < 2 {
        return Err("需要包含修饰键的组合键".into());
    }
    let (main_key, modifiers) = combo.split_last().unwrap();
    let mut tokens = Vec::with_capacity(combo.len());
    for m in modifiers {
        tokens.push(modifier_token(m).ok_or_else(|| format!("无法识别的修饰键 {m}"))?.to_string());
    }
    tokens.push(key_token(main_key).ok_or_else(|| format!("无法识别的按键 {main_key}"))?);
    tokens
        .join("+")
        .parse::<Shortcut>()
        .map_err(|e| format!("无效的组合键：{e}"))
}

/// 按当前配置重新注册全部快捷键，并把注册失败写入 core.conflicts。
/// 调用方需已持有 CoreState 锁。
pub fn apply_shortcuts(app: &AppHandle, core: &mut crate::state::Core) {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    core.registered.clear();
    core.conflicts.clear();

    if !core.hotkeys_enabled {
        return;
    }

    for action in ShortcutAction::ALL {
        let Some(combo) = core.shortcuts.get(&action) else { continue };
        let shortcut = match parse_combo(combo) {
            Ok(s) => s,
            Err(reason) => {
                core.conflicts.insert(action, reason);
                continue;
            }
        };
        if core.registered.iter().any(|(s, _)| *s == shortcut) {
            core.conflicts.insert(action, "与其他动作的快捷键重复".into());
            continue;
        }
        match shortcuts.register(shortcut) {
            Ok(()) => {
                eprintln!("[hotkey] 注册成功 {action:?} = {combo:?}");
                core.registered.push((shortcut, action));
            }
            // RegisterHotKey 失败 = 已被系统或其他程序注册（PRD §7.3）
            Err(err) => {
                eprintln!("[hotkey] 注册 {action:?} = {combo:?} 失败：{err}");
                core.conflicts.insert(action, "与系统快捷键冲突".into());
            }
        }
    }
}

/// 全局热键回调（插件 with_handler）。按住不放时系统会重复触发 Pressed，天然支持长按连续调速。
pub fn on_shortcut(app: &AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    eprintln!("[hotkey] on_shortcut 触发 state={:?}", event.state());
    if event.state() != ShortcutState::Pressed {
        return;
    }

    let state = app.state::<CoreState>();
    let mut core = state.lock().expect("core state poisoned");
    let Some(action) = core
        .registered
        .iter()
        .find(|(s, _)| s == shortcut)
        .map(|(_, a)| *a)
    else {
        eprintln!("[hotkey] 未匹配到已注册动作");
        return;
    };
    eprintln!("[hotkey] 匹配动作 {action:?}");

    let mut remembered = false;
    match action {
        ShortcutAction::SpeedUp => {
            let cap = hotkey_rate_cap(&core);
            let next = stepped_rate(&core, 1);
            core.rate = clamp_rate(next, cap);
            remembered = core.remember_rate();
        }
        ShortcutAction::SpeedDown => {
            let cap = hotkey_rate_cap(&core);
            let next = stepped_rate(&core, -1);
            core.rate = clamp_rate(next, cap);
            remembered = core.remember_rate();
        }
        ShortcutAction::Reset => {
            core.rate = 1.0;
            remembered = core.remember_rate();
        }
        ShortcutAction::PlayPause => {}
        ShortcutAction::TogglePanel => {
            drop(core);
            toggle_main_window(app);
            return;
        }
    }

    core.osd_seq += 1;
    let payload = HotkeyPayload {
        action,
        rate: core.rate,
        seq: core.osd_seq,
        notice: high_speed_notice(&core),
    };
    drop(core);
    if remembered {
        crate::persist::save_memory_debounced(app);
    }

    // 同步拍到此为止：主窗口/OSD 立即反馈目标值；下发到播放器走异步拍（见 router 顶部说明）
    let _ = app.emit("hotkey:triggered", payload.clone());
    osd::show(app, &payload);

    match action {
        ShortcutAction::SpeedUp => router::push_rate_async(app, PushMode::Step { dir: 1 }),
        ShortcutAction::SpeedDown => router::push_rate_async(app, PushMode::Step { dir: -1 }),
        ShortcutAction::Reset => router::push_rate_async(app, PushMode::Reset),
        ShortcutAction::PlayPause => router::play_pause_async(app),
        ShortcutAction::TogglePanel => {}
    }
}

/// 当前热键的作用对象是否浏览器（含「焦点在别处但有已连接浏览器」的兜底场景）
fn is_browser_target(core: &Core) -> bool {
    core.current_rule()
        .map(|r| r.kind == AppKind::Browser)
        .unwrap_or(false)
        || (core.current.is_none() && !core.connected_browsers.is_empty())
}

/// 步进目标值。接管对象的倍速若只能落在有限几个值上，就按它自己的档位走：
/// OSD 当场显示的即是播放器真会到的值，不必等回读回来再改口。
///
/// 三条路，按「离播放器真相有多近」排序：
/// 1. 有档位表（MPC-HC 的绝对倍速命令）→ 一次挪一档。档距不均匀，`rate ± step`
///    会算出 2.25 这种表里没有的值，下发时被就近吸回 2.0，热键从此卡死在 2.0；
/// 2. 无回读 + 有按键网格（百度网盘）→ 按整格走，理由同上一条；
/// 3. 其余（mpv / VLC / PotPlayer / 浏览器）→ 原样加减。这些通道能回读，
///    异步拍会拿真实值校正，先量化只会把用户设的 0.25 步长白白撑成网格的 0.3。
fn stepped_rate(core: &Core, dir: i32) -> f64 {
    let step = core.settings.step;
    let fallback = core.rate + step * f64::from(dir);
    let Some(rule) = core.current_rule() else { return fallback };
    if let Some(rung) = rule.ladder_step(core.rate, dir) {
        return rung;
    }
    if rule.can_read_back_rate() {
        return fallback;
    }
    rule.key_rate
        .as_ref()
        .and_then(|grid| grid.step_target(core.rate, step, dir))
        .unwrap_or(fallback)
}

/// 浏览器内核上限 16×；滑块上限只约束控制页 UI，不该挡住全局热键打到网页视频。
fn hotkey_rate_cap(core: &Core) -> f64 {
    if is_browser_target(core) {
        RATE_MAX
    } else {
        core.settings.slider_max
    }
}

/// >4× 时 Chromium 不再做时间拉伸、音频静音（开发文档 §7.8），OSD 附带提示
fn high_speed_notice(core: &Core) -> Option<String> {
    (core.settings.high_speed_warning && core.rate > 4.0 && is_browser_target(core))
        .then(|| "浏览器已静音".to_string())
}

fn toggle_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    // 最小化视为「未显示」：此时按下快捷键应还原窗口而不是再次隐藏
    let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    if visible {
        let _ = window.hide();
    } else {
        crate::show_main_window(app);
    }
}
