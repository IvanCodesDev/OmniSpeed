//! OmniSpeed Rust 核心（M1：托盘 + 全局快捷键 + 冲突检测 + OSD）。
//! 后续模块规划见开发文档 §5.1：router / monitor / adapters / nm_bridge / platform。

mod commands;
mod hotkey;
mod osd;
mod persist;
mod state;

use state::{Core, CoreState};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};

/// 显示并聚焦主窗口（托盘唤起 / 全局快捷键唤起）
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "打开主面板", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 OmniSpeed", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().expect("缺少应用图标").clone())
        .tooltip("OmniSpeed —— 全局倍速遥控器")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // 左键单击托盘图标 = 唤起主面板
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(hotkey::on_shortcut)
                .build(),
        )
        .manage::<CoreState>(Mutex::new(Core::default()))
        .invoke_handler(tauri::generate_handler![
            commands::get_core_state,
            commands::set_rate,
            commands::sync_rate_config,
            commands::save_shortcuts,
            commands::set_hotkeys_enabled,
        ])
        .setup(|app| {
            setup_tray(app)?;
            osd::create_window(app.handle())?;

            // 读取持久化的快捷键配置并注册全局热键（冲突记录在 state 中，前端拉取后行内标红）
            let state = app.state::<CoreState>();
            let mut core = state.lock().expect("core state poisoned");
            persist::load(app.handle(), &mut core);
            hotkey::apply_shortcuts(app.handle(), &mut core);

            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭主窗口 = 隐藏到托盘（对应设置「关闭窗口时最小化到托盘」，
            // M0 默认开启；设置项接入 Rust 侧后改为读取配置）
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
