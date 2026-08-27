//! 全局 OSD 悬浮提示（PRD §7.5）：热键调速时在屏幕上短暂显示当前倍速，
//! 不抢焦点、点击穿透、约 1s 后淡出。窗口常驻（隐藏复用），显示时定位到光标所在屏幕底部居中。

use crate::hotkey::HotkeyPayload;
use crate::state::CoreState;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

pub const WINDOW_LABEL: &str = "osd";

/// 窗口逻辑尺寸（内容是居中的胶囊，窗口本身透明）
const WIDTH: f64 = 280.0;
const HEIGHT: f64 = 104.0;
/// 距屏幕底边的逻辑像素（留出任务栏高度）
const BOTTOM_MARGIN: f64 = 96.0;
/// 前端在 1100ms 开始淡出（约 350ms），这里 1500ms 后隐藏窗口
const HIDE_AFTER_MS: u64 = 1500;

pub fn create_window(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(
        app,
        WINDOW_LABEL,
        WebviewUrl::App("index.html?window=osd".into()),
    )
    .title("OmniSpeed OSD")
    .inner_size(WIDTH, HEIGHT)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .focusable(false)
    .visible(false)
    .build()?;

    window.set_ignore_cursor_events(true)?;
    Ok(())
}

/// 显示 OSD 并安排延时隐藏；期间再次触发热键会推进 osd_seq，使旧的隐藏任务失效
pub fn show(app: &AppHandle, payload: &HotkeyPayload) {
    let Some(window) = app.get_webview_window(WINDOW_LABEL) else { return };

    // 先下发内容再显示窗口，避免闪现上一次的数值
    let _ = app.emit_to(WINDOW_LABEL, "hotkey:triggered", payload.clone());
    position_near_cursor_monitor(app, &window);
    let _ = window.show();

    let seq = payload.seq;
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(HIDE_AFTER_MS)).await;
        let latest = handle
            .state::<CoreState>()
            .lock()
            .map(|core| core.osd_seq)
            .unwrap_or(seq);
        if latest == seq {
            if let Some(window) = handle.get_webview_window(WINDOW_LABEL) {
                let _ = window.hide();
            }
        }
    });
}

/// 定位到光标所在显示器的底部居中；取不到光标/显示器时回退主屏
fn position_near_cursor_monitor(app: &AppHandle, window: &tauri::WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let Ok(size) = window.outer_size() else { return };

    let m_pos = monitor.position();
    let m_size = monitor.size();
    let x = m_pos.x + (m_size.width as i32 - size.width as i32) / 2;
    let y = m_pos.y + m_size.height as i32
        - size.height as i32
        - (BOTTOM_MARGIN * monitor.scale_factor()) as i32;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}
