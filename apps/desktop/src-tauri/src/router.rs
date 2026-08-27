//! 命令路由（开发文档 §5.1 / §6）：把「加速/减速/设为 X/播放暂停」路由到
//! 当前接管对象的适配器链（IPC 优先、按键兜底）。
//!
//! 线程模型：热键回调跑在主事件循环线程上，而 IPC 是带超时的阻塞 IO——
//! 绝不能在持锁或事件循环里做。因此路由分两拍：
//!   1) 同步拍（持锁，微秒级）：更新 Core.rate 目标值，UI/OSD 立即反馈；
//!   2) 异步拍（tauri 异步运行时）：快照通道列表后逐通道下发，可回读时
//!      以真实值校正 Core.rate 并广播 media:changed。

use crate::adapters::{adapters_for, Adapter};
use crate::state::{clamp_rate, Core, CoreState, RATE_MAX};
use tauri::{AppHandle, Emitter, Manager};

/// 无副作用回读当前接管对象的真实倍速（monitor 切换目标时用；调用方持锁）
pub fn read_current_rate(core: &Core) -> Option<f64> {
    let target = core.current.as_ref()?;
    let rule = core.current_rule()?;
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
    let (target_rate, adapters) = {
        let core = state.lock().expect("core state poisoned");
        if !core.listening {
            return;
        }
        let (Some(target), Some(rule)) = (core.current.as_ref(), core.current_rule()) else {
            return;
        };
        (core.rate, adapters_for(rule, target))
    };
    if adapters.is_empty() {
        return;
    }

    // 逐通道尝试：任一通道成功即止
    let mut read_back = None;
    let mut applied = false;
    for adapter in &adapters {
        let result = match mode {
            PushMode::Reset => adapter.reset(),
            PushMode::Step { dir } => match adapter.set_rate(target_rate) {
                ok @ Ok(_) => ok,
                // IPC 不可用/通道不支持精确值 → 按播放器自身档位步进一档
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
            let (Some(target), Some(rule)) = (core.current.as_ref(), core.current_rule()) else {
                return;
            };
            adapters_for(rule, target)
        };
        for adapter in &adapters {
            if adapter.play_pause().is_ok() {
                break;
            }
        }
    });
}
