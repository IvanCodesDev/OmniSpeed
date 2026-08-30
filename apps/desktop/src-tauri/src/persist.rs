//! 配置持久化（tauri-plugin-store，JSON 文件）。
//! M1 持久化快捷键与总开关，M2 增加应用规则，
//! M4 增加设置页全部选项与按应用/网站的倍速记忆（开发文档 §8 / §11）。

use crate::rules::{merge_saved, AppRule};
use crate::state::{
    clamp_rate, default_shortcuts, merge_saved_site_rules, Core, CoreState, Settings, ShortcutMap,
    SiteRule,
};
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use tauri_plugin_store::StoreExt;

const FILE: &str = "config.json";

/// 启动时读取持久化配置；文件缺失或字段损坏时保留默认值
pub fn load(app: &AppHandle, core: &mut Core) {
    let Ok(store) = app.store(FILE) else { return };

    if let Some(value) = store.get("shortcuts") {
        if let Ok(saved) = serde_json::from_value::<ShortcutMap>(value) {
            // 以默认表为底、存量覆盖：新版本新增动作时旧配置仍兼容
            let mut merged = default_shortcuts();
            for (action, combo) in saved {
                if !combo.is_empty() {
                    merged.insert(action, combo);
                }
            }
            core.shortcuts = merged;
        }
    }

    if let Some(enabled) = store.get("hotkeysEnabled").and_then(|v| v.as_bool()) {
        core.hotkeys_enabled = enabled;
    }

    if let Some(value) = store.get("appRules") {
        if let Ok(saved) = serde_json::from_value::<Vec<AppRule>>(value) {
            // 内置表以代码为准，存量只覆盖可编辑字段 / 追加自定义规则
            merge_saved(&mut core.rules, saved);
        }
    }

    if let Some(value) = store.get("settings") {
        if let Ok(saved) = serde_json::from_value::<Settings>(value) {
            core.settings = saved;
        }
    }
    core.settings.normalize();

    if let Some(value) = store.get("siteRules") {
        if let Ok(saved) = serde_json::from_value::<Vec<SiteRule>>(value) {
            // 内置 8 站以代码为准，存量覆盖可编辑字段 / 追加自定义站点（M3.5）
            merge_saved_site_rules(&mut core.site_rules, saved);
        }
    }

    if let Some(value) = store.get("memory") {
        if let Ok(saved) = serde_json::from_value::<HashMap<String, f64>>(value) {
            core.memory = saved
                .into_iter()
                .filter(|(_, r)| r.is_finite())
                .map(|(k, r)| (k, clamp_rate(r, crate::state::RATE_MAX)))
                .collect();
        }
    }

    // 启动初始倍速 = 默认倍速设置（PRD 设置页「默认倍速」）
    core.rate = clamp_rate(core.settings.default_rate, core.settings.slider_max);
}

pub fn save(app: &AppHandle, core: &Core) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("shortcuts", json!(core.shortcuts));
    store.set("hotkeysEnabled", core.hotkeys_enabled);
    store.set("appRules", json!(core.rules));
    store.set("settings", json!(core.settings));
    store.set("siteRules", json!(core.site_rules));
    store.set("memory", json!(core.memory));
    store.save().map_err(|e| e.to_string())
}

/// 记忆表的防抖持久化：热键长按会高频触发 remember_rate，
/// 这里只在同一代数静默 1.2s 后落盘一次，避免每次步进都写文件。
pub fn save_memory_debounced(app: &AppHandle) {
    let seq = {
        let state = app.state::<CoreState>();
        let core = state.lock().expect("core state poisoned");
        core.memory_seq
    };
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let state = handle.state::<CoreState>();
        let core = state.lock().expect("core state poisoned");
        if core.memory_seq != seq {
            return; // 有更新的变更，让最后一个防抖任务负责落盘
        }
        if let Err(err) = save(&handle, &core) {
            eprintln!("[persist] 记忆持久化失败：{err}");
        }
    });
}
