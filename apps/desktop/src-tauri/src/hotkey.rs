//! 全局快捷键：前端组合键 → 系统热键注册、冲突检测与动作分发（开发文档 §7.2）。
//! 冲突检测策略：RegisterHotKey 注册失败即判定被系统或其他程序占用。

use crate::osd;
use crate::router::{self, PushMode};
use crate::state::{clamp_rate, CoreState, ShortcutAction};
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
            Ok(()) => core.registered.push((shortcut, action)),
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
        return;
    };

    match action {
        ShortcutAction::SpeedUp => {
            core.rate = clamp_rate(core.rate + core.step, core.slider_max);
        }
        ShortcutAction::SpeedDown => {
            core.rate = clamp_rate(core.rate - core.step, core.slider_max);
        }
        ShortcutAction::Reset => core.rate = 1.0,
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
    };
    drop(core);

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
