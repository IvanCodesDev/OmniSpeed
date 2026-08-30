//! OmniSpeed Rust 核心。模块规划见开发文档 §5.1：
//! router / monitor / adapters / nm_bridge / updater / platform。

mod adapters;
mod commands;
mod hotkey;
mod monitor;
mod nm_bridge;
mod osd;
mod persist;
mod router;
mod rules;
mod state;
mod updater;

/// `omnispeed.exe --nm-host`：作为 Native Messaging 中继运行（由浏览器拉起，无 GUI）
pub fn nm_host_main() {
    nm_bridge::relay_main();
}

use state::{Core, CoreState};
use std::sync::Mutex;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};

/// 显示并聚焦主窗口（托盘唤起 / 全局快捷键唤起 / 二次启动唤起）
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// 把系统的开机自启状态对齐到设置（HKCU Run 项，tauri-plugin-autostart）。
/// 失败不致命（如被组策略禁止），记录后继续
pub(crate) fn sync_autostart(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    if autolaunch.is_enabled().unwrap_or(false) == enabled {
        return;
    }
    let result = if enabled { autolaunch.enable() } else { autolaunch.disable() };
    if let Err(err) = result {
        eprintln!("[autostart] 同步失败（目标 {enabled}）：{err}");
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
        // 单实例必须最先注册：二次启动只唤起已有窗口（开发文档 §7.7）
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // 开机自启时静默进托盘，不弹主窗口
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(hotkey::on_shortcut)
                .build(),
        )
        .manage::<CoreState>(Mutex::new(Core::default()))
        .invoke_handler(tauri::generate_handler![
            commands::get_core_state,
            commands::set_rate,
            commands::save_settings,
            commands::save_shortcuts,
            commands::set_hotkeys_enabled,
            commands::list_apps,
            commands::save_app_rule,
            commands::list_site_rules,
            commands::save_site_rule,
            commands::get_current_media,
            commands::set_listening,
            commands::apply_to_current,
            updater::install_update,
        ])
        .setup(|app| {
            setup_tray(app)?;
            osd::create_window(app.handle())?;

            // 读取持久化配置（快捷键/规则/设置/记忆）并注册全局热键
            let start_on_boot = {
                let state = app.state::<CoreState>();
                let mut core = state.lock().expect("core state poisoned");
                persist::load(app.handle(), &mut core);
                hotkey::apply_shortcuts(app.handle(), &mut core);
                nm_bridge::set_preserves_pitch(core.settings.preserves_pitch);
                nm_bridge::set_site_rules(&core.site_rules);
                core.settings.start_on_boot
            };
            sync_autostart(app.handle(), start_on_boot);

            // 开机自启带 --minimized：静默进托盘
            if std::env::args().any(|a| a == "--minimized") {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
            }

            // 前台窗口监听：把前台进程映射到应用规则，驱动「当前媒体」（开发文档 §7.1）
            if let Err(err) = monitor::start(app.handle()) {
                eprintln!("[monitor] 前台监听启动失败：{err}");
            }

            // Native Messaging 桥：注册宿主（HKCU）+ 启动管道服务端（开发文档 §5.3）
            if let Err(err) = nm_bridge::register_host(app.handle()) {
                eprintln!("[nm] 宿主注册失败：{err}");
            }
            nm_bridge::start(app.handle());

            // 自动更新：启动后延时检查 + 每 24h 复查（设置「自动检查更新」控制）
            updater::start(app.handle());

            Ok(())
        })
        .on_window_event(|window, event| {
            // 关闭主窗口：按设置决定隐藏到托盘还是退出（PRD 设置页「关闭窗口时最小化到托盘」）
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    let to_tray = window
                        .app_handle()
                        .state::<CoreState>()
                        .lock()
                        .map(|core| core.settings.minimize_to_tray)
                        .unwrap_or(true);
                    if to_tray {
                        api.prevent_close();
                        let _ = window.hide();
                    } else {
                        window.app_handle().exit(0);
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
