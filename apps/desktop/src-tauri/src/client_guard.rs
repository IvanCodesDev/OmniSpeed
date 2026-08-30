//! 客户端防复位守护 keeper（M4.6）。
//!
//! [`player_ipc::GuardSession`] 经 `Page.addScriptToEvaluateOnNewDocument` 注册的 guard
//! 随调试会话断开而失效（Chromium 按 session 保存注册），所以必须有人**长期持有连接**。
//! 本模块在后台线程维护这些会话：
//! - 每 3s 快照一次 CDP 规则端口（锁内不做 IO），枚举在线调试口的页面目标；
//! - 新目标 → 建 [`GuardSession`]（注册 on-new-document guard + 当前文档立即注入）；
//! - 既有目标 → 心跳探活，失败丢弃下轮重建；目标消失（客户端退出/页面关闭）→ 清理。
//!
//! 会话按 `webSocketDebuggerUrl` 为键：同一目标内的整页导航不换 ws url，会话天然存续，
//! 新文档由 on-new-document 注入的 guard 从 localStorage 自动续用目标倍速。

use crate::adapters::CDP_DEFAULT_PORT;
use crate::rules::IpcKind;
use crate::state::CoreState;
use player_ipc::{CdpClient, GuardSession};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// 发现/心跳节奏。接管后最迟一个 tick 内装上 guard；对本机回环轮询这点开销可忽略
const TICK: Duration = Duration::from_secs(3);

pub fn start(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || keeper_loop(&handle));
}

fn keeper_loop(app: &AppHandle) {
    let mut sessions: HashMap<String, GuardSession> = HashMap::new();
    loop {
        std::thread::sleep(TICK);

        // 同步拍：只从 Core 抄走 CDP 端口列表，IO 全部在锁外做（同 router 的两拍原则）
        let ports: Vec<u16> = {
            let state = app.state::<CoreState>();
            let Ok(core) = state.lock() else { continue };
            core.rules
                .iter()
                .filter(|r| r.ipc == IpcKind::Cdp)
                .map(|r| {
                    r.ipc_config.as_ref().and_then(|c| c.port).unwrap_or(CDP_DEFAULT_PORT)
                })
                .collect()
        };

        // 枚举所有在线调试口的页面目标（未接管的端口 page_targets 快速失败，直接跳过）
        let mut live_targets: Vec<String> = Vec::new();
        for port in ports {
            if let Ok(targets) = CdpClient::new(port).page_targets() {
                live_targets.extend(targets);
            }
        }
        live_targets.sort();
        live_targets.dedup();

        // 目标已消失的会话直接丢弃（连接随 drop 关闭）
        sessions.retain(|ws_url, _| live_targets.binary_search(ws_url).is_ok());

        for ws_url in live_targets {
            match sessions.entry(ws_url) {
                Entry::Occupied(mut occupied) => {
                    if occupied.get_mut().heartbeat().is_err() {
                        occupied.remove();
                    }
                }
                // open 失败（目标可能正在关闭/导航）下轮重试，不刷屏
                Entry::Vacant(vacant) => {
                    if let Ok(session) = GuardSession::open(vacant.key()) {
                        eprintln!("[client-guard] guard 已注册：{}", vacant.key());
                        vacant.insert(session);
                    }
                }
            }
        }
    }
}
