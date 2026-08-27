//! 前台窗口监听（开发文档 §7.1）：SetWinEventHook 事件驱动捕捉前台切换，
//! 把前台进程映射到应用规则，维护 Core.current 并广播 media:changed。
//!
//! 目标语义是「统一遥控器」：切到未匹配的应用（资源管理器/编辑器等）时**保留**
//! 上一个接管对象——IPC 通道不要求播放器在前台，用户在别的窗口里也应能继续遥控；
//! 只有切到另一个匹配规则的应用时才切换接管对象。

use crate::router;
use crate::state::{CoreState, CurrentTarget};
use platform_win::{ForegroundInfo, ForegroundWatcher};
use tauri::{AppHandle, Emitter, Manager};

/// 启动前台监听。watcher 生命周期与应用等长，泄漏持有避免 Drop 反注册。
pub fn start(app: &AppHandle) -> Result<(), String> {
    let handle = app.clone();
    let watcher = ForegroundWatcher::start(move |info| on_foreground_change(&handle, info))
        .map_err(|e| e.to_string())?;
    std::mem::forget(watcher);
    Ok(())
}

fn on_foreground_change(app: &AppHandle, info: ForegroundInfo) {
    let state = app.state::<CoreState>();
    let mut core = state.lock().expect("core state poisoned");

    let matched = core
        .rules
        .iter()
        .find(|r| r.matches(&info.process_name))
        .map(|r| r.id.clone());

    // 未匹配的前台不清空接管对象（保留遥控），也就无需广播
    let Some(rule_id) = matched else { return };

    let old_key = core.memory_key();
    core.current = Some(CurrentTarget {
        rule_id,
        hwnd: info.hwnd,
        process_name: info.process_name,
    });

    if !core.listening {
        return;
    }

    // 按应用记忆（开发文档 §7.5）：切到另一个记忆键（新应用/新站点）且有记录时，
    // 恢复上次用的倍速并主动下发。同一应用来回聚焦不触发（避免覆盖用户在播放器里的调整）
    let new_key = core.memory_key();
    if core.settings.remember_per_app && new_key != old_key {
        if let Some(saved) = new_key.as_ref().and_then(|k| core.memory.get(k).copied()) {
            core.rate = crate::state::clamp_rate(saved, crate::state::RATE_MAX);
            let session = core.current_session(Some(core.rate));
            drop(core);
            let _ = app.emit("media:changed", &session);
            router::push_rate_async(app, router::PushMode::ExactOnly);
            return;
        }
    }

    // 每次匹配的应用回到前台都回读同步（不只在切换时）：
    // 覆盖同一播放器的新实例（旧目标倍速不应冒充新实例的状态），
    // 也让用户在播放器 UI 里的主动调速被尊重并同步（开发文档 §7.4 同一原则）
    let rate = router::read_current_rate(&core);
    if let Some(real) = rate {
        core.rate = crate::state::clamp_rate(real, crate::state::RATE_MAX);
    }
    let session = core.current_session(rate);
    drop(core);
    let _ = app.emit("media:changed", &session);
}
