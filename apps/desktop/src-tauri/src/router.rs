//! 命令路由（开发文档 §5.1 / §6）：把「加速/减速/设为 X/播放暂停」路由到
//! 当前接管对象的适配器链（IPC 优先、按键兜底）。
//!
//! 线程模型：热键回调跑在主事件循环线程上，而 IPC 是带超时的阻塞 IO——
//! 绝不能在持锁或事件循环里做。因此路由分两拍：
//!   1) 同步拍（持锁，微秒级）：更新 Core.rate 目标值，UI/OSD 立即反馈；
//!   2) 异步拍（tauri 异步运行时）：快照通道列表后逐通道下发，可回读时
//!      以真实值校正 Core.rate 并广播 media:changed。

use crate::adapters::{adapters_for, Adapter};
use crate::hotkey::HotkeyPayload;
use crate::rules::AppKind;
use crate::state::{clamp_rate, Core, CoreState, ShortcutAction, RATE_MAX};
use tauri::{AppHandle, Emitter, Manager};

/// 当前接管对象的通道；前台尚未匹配到播放器时（焦点在编辑器等），
/// 回退到「已连接且活动标签有媒体」的唯一浏览器——全局热键仍应遥控正在看的网页视频。
pub(crate) fn adapters_for_current_or_browser(core: &Core) -> Vec<Adapter> {
    if let (Some(target), Some(rule)) = (core.current.as_ref(), core.current_rule()) {
        if rule.kind == AppKind::Browser
            && core
                .browser_media
                .get(&target.process_name)
                .map(|m| m.is_live)
                .unwrap_or(false)
        {
            return Vec::new();
        }
        let list = adapters_for(rule, target);
        if !list.is_empty() {
            return list;
        }
    }
    browser_fallback_adapters(core)
}

fn browser_fallback_adapters(core: &Core) -> Vec<Adapter> {
    let mut found = Vec::new();
    for process in &core.connected_browsers {
        let media = core.browser_media.get(process);
        if media.map(|m| m.is_live).unwrap_or(false) {
            continue;
        }
        // 尚无 media 帧时也允许下发：hello 已证明扩展在线，setRate 由 SW 路由到有媒体的标签
        found.push(Adapter::Browser {
            process: process.clone(),
        });
    }
    if found.len() == 1 {
        found
    } else {
        Vec::new()
    }
}

/// 无副作用回读当前接管对象的真实倍速（monitor 切换目标时用；调用方持锁）。
/// 浏览器目标直接读扩展最近一次上报（Core.browser_media），不发起 IO
pub fn read_current_rate(core: &Core) -> Option<f64> {
    let target = core.current.as_ref()?;
    let rule = core.current_rule()?;
    if rule.kind == crate::rules::AppKind::Browser {
        return core
            .browser_media
            .get(&target.process_name)
            .filter(|m| m.has_media)
            .map(|m| m.rate);
    }
    adapters_for(rule, target)
        .iter()
        .find_map(Adapter::read_rate)
}

/// 把 Core.rate 目标值异步下发到当前接管对象。
/// mode 决定按键/消息通道的降级动作：精确值下发失败时步进（Step）或放弃（ExactOnly）。
pub fn push_rate_async(app: &AppHandle, mode: PushMode) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || push_rate_blocking(&handle, mode));
}

#[derive(Clone, Copy, PartialEq)]
pub enum PushMode {
    /// 热键步进：IPC 通道设精确目标值，无 IPC 时按播放器自身档位步进一档
    Step { dir: i32 },
    /// 恢复 1.0×：优先精确设值，否则用播放器的恢复键/消息
    Reset,
    /// 滑块/预设/「应用到当前媒体」：只接受精确设值（按键通道无法保证，直接放弃）
    ExactOnly,
}

fn push_rate_blocking(app: &AppHandle, mode: PushMode) {
    let state = app.state::<CoreState>();

    // 同步快照：目标倍速与通道列表，锁内不做任何 IO
    let (target_rate, adapters, is_client) = {
        let core = state.lock().expect("core state poisoned");
        if !core.listening {
            return;
        }
        let adapters = adapters_for_current_or_browser(&core);
        if adapters.is_empty() {
            return;
        }
        let is_client =
            core.current_rule().map(|r| r.kind == AppKind::Client).unwrap_or(false);
        (core.rate, adapters, is_client)
    };

    // 逐通道尝试：任一通道成功即止
    let mut read_back = None;
    let mut applied = false;
    for adapter in &adapters {
        let result = match mode {
            PushMode::Reset => adapter.reset(),
            PushMode::Step { dir } => match adapter.set_rate(target_rate) {
                ok @ Ok(_) => ok,
                // IPC/扩展通道不可用时退回播放器自身的步进档位
                Err(_) => adapter.step(dir).map(|_| None),
            },
            PushMode::ExactOnly => adapter.set_rate(target_rate),
        };
        if let Ok(rb) = result {
            read_back = rb;
            applied = true;
            break;
        }
    }
    if !applied {
        // M4.6：客户端（Client 规则）未接管时，热键静默失败会让用户误以为坏了。
        // 仅热键路径提示（UI 的滑块/预设由应用页 CdpPanel 自己引导），且必须确认
        // 确实是调试口离线（未接管），而不是「接管了但没在播视频」等其它失败
        if is_client && matches!(mode, PushMode::Step { .. } | PushMode::Reset) {
            let offline =
                adapters.iter().any(|a| matches!(a, Adapter::Cdp(c) if !c.is_available()));
            if offline {
                notify_client_not_taken_over(app, target_rate, mode);
            }
        }
        return;
    }

    // 可回读的通道以真实值校正目标值，并让控制页刷新
    let session = {
        let mut core = state.lock().expect("core state poisoned");
        if let Some(real) = read_back {
            core.rate = clamp_rate(real, RATE_MAX);
        }
        core.current_session(read_back)
    };
    if session.is_some() {
        let _ = app.emit("media:changed", &session);
    }
}

/// 未接管客户端的热键 OSD 引导（M4.6）：复用 OSD 通道再发一帧带提示的载荷，
/// 约半秒后盖掉同步拍那帧「看似成功」的倍速显示。rate 仍显示目标值——接管后即生效
fn notify_client_not_taken_over(app: &AppHandle, rate: f64, mode: PushMode) {
    let payload = {
        let state = app.state::<CoreState>();
        let mut core = state.lock().expect("core state poisoned");
        core.osd_seq += 1;
        HotkeyPayload {
            action: match mode {
                PushMode::Step { dir } if dir < 0 => ShortcutAction::SpeedDown,
                PushMode::Reset => ShortcutAction::Reset,
                _ => ShortcutAction::SpeedUp,
            },
            rate,
            seq: core.osd_seq,
            notice: Some("客户端未接管 · 应用页一键接管".into()),
        }
    };
    let _ = app.emit("hotkey:triggered", payload.clone());
    crate::osd::show(app, &payload);
}

/// 播放/暂停：直接走通道，无状态可回读
pub fn play_pause_async(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = handle.state::<CoreState>();
        let adapters = {
            let core = state.lock().expect("core state poisoned");
            if !core.listening {
                return;
            }
            adapters_for_current_or_browser(&core)
        };
        for adapter in &adapters {
            if adapter.play_pause().is_ok() {
                break;
            }
        }
    });
}
