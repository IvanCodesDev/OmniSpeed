//! 配置持久化（tauri-plugin-store，JSON 文件）。
//! M1 仅持久化快捷键与总开关；设置页其余项在 M4 落地（开发文档 §11）。

use crate::state::{default_shortcuts, Core, ShortcutMap};
use serde_json::json;
use tauri::AppHandle;
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
}

pub fn save(app: &AppHandle, core: &Core) -> Result<(), String> {
    let store = app.store(FILE).map_err(|e| e.to_string())?;
    store.set("shortcuts", json!(core.shortcuts));
    store.set("hotkeysEnabled", core.hotkeys_enabled);
    store.save().map_err(|e| e.to_string())
}
