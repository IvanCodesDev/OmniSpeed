//! Tauri 命令：前端 ↔ 核心状态（接口清单见开发文档 §9）。

use crate::state::{clamp_rate, CoreSnapshot, CoreState, ShortcutAction, ShortcutMap, RATE_MAX, RATE_MIN};
use crate::{hotkey, persist};
use std::collections::HashMap;
use tauri::{AppHandle, State};

/// 前端启动时一次性拉取核心状态
#[tauri::command]
pub fn get_core_state(state: State<CoreState>) -> CoreSnapshot {
    state.lock().expect("core state poisoned").snapshot()
}

/// UI 侧（滑块/预设/最近媒体）调速后同步权威值；返回收口后的实际倍速
#[tauri::command]
pub fn set_rate(state: State<CoreState>, rate: f64) -> f64 {
    let mut core = state.lock().expect("core state poisoned");
    core.rate = clamp_rate(rate, RATE_MAX);
    core.rate
}

/// 设置页变更步长/滑块上限时同步（热键步进依赖这两个值）
#[tauri::command]
pub fn sync_rate_config(state: State<CoreState>, step: f64, slider_max: f64) {
    let mut core = state.lock().expect("core state poisoned");
    core.step = step.clamp(0.05, 1.0);
    core.slider_max = slider_max.clamp(RATE_MIN, RATE_MAX);
    // 上限调低时收拢当前倍速（与前端 store 行为一致）
    core.rate = clamp_rate(core.rate, core.slider_max);
}

/// 保存并重新注册全部快捷键；返回冲突表（动作 → 原因），空表 = 全部注册成功
#[tauri::command]
pub fn save_shortcuts(
    app: AppHandle,
    state: State<CoreState>,
    shortcuts: ShortcutMap,
) -> Result<HashMap<ShortcutAction, String>, String> {
    let mut core = state.lock().expect("core state poisoned");
    for (action, combo) in shortcuts {
        core.shortcuts.insert(action, combo);
    }
    hotkey::apply_shortcuts(&app, &mut core);
    persist::save(&app, &core)?;
    Ok(core.conflicts.clone())
}

/// 全局快捷键总开关；开启时立即注册并返回冲突表
#[tauri::command]
pub fn set_hotkeys_enabled(
    app: AppHandle,
    state: State<CoreState>,
    enabled: bool,
) -> Result<HashMap<ShortcutAction, String>, String> {
    let mut core = state.lock().expect("core state poisoned");
    core.hotkeys_enabled = enabled;
    hotkey::apply_shortcuts(&app, &mut core);
    persist::save(&app, &core)?;
    Ok(core.conflicts.clone())
}
