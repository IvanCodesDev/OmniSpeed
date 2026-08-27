//! 自动更新（开发文档 §7.7）：tauri-plugin-updater + GitHub Releases 上的签名清单。
//! 启动后延时检查一次、之后每 24 小时复查（PRD 设置页「自动检查更新」）；
//! 发现新版本只广播 update:available，下载安装由用户在设置页主动触发。

use crate::state::CoreState;
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
}

pub fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        // 起步延时：让窗口/托盘/NM 桥先就绪
        tokio::time::sleep(Duration::from_secs(15)).await;
        loop {
            let enabled = handle
                .state::<CoreState>()
                .lock()
                .map(|core| core.settings.auto_update)
                .unwrap_or(false);
            if enabled {
                check_once(&handle).await;
            }
            tokio::time::sleep(Duration::from_secs(60 * 60 * 24)).await;
        }
    });
}

async fn check_once(app: &AppHandle) {
    let updater = match app.updater() {
        Ok(updater) => updater,
        Err(err) => {
            eprintln!("[updater] 初始化失败：{err}");
            return;
        }
    };
    match updater.check().await {
        Ok(Some(update)) => {
            eprintln!("[updater] 发现新版本 {}", update.version);
            let _ = app.emit(
                "update:available",
                UpdateInfo { version: update.version.clone() },
            );
        }
        Ok(None) => {}
        // 尚无发布 / 离线属预期情况，仅记录不打扰
        Err(err) => eprintln!("[updater] 检查失败：{err}"),
    }
}

/// 设置页「更新并重启」：下载、安装并重启应用
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or("当前已是最新版本")?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    // Windows 上安装器会接管并退出进程；其余平台由这里重启
    app.restart();
}
