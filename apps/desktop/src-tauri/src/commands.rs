//! Tauri 命令：前端 ↔ 核心状态（接口清单见开发文档 §9）。

use crate::rules::{running_processes, to_app_info, AppInfo, AppRulePatch};
use crate::state::{
    clamp_rate, Core, CoreSnapshot, CoreState, MediaSession, Settings, ShortcutAction,
    ShortcutMap, SiteRule, RATE_MAX,
};
use crate::{hotkey, nm_bridge, persist};
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, State};

/// 前端启动时一次性拉取核心状态
#[tauri::command]
pub fn get_core_state(state: State<CoreState>) -> CoreSnapshot {
    state.lock().expect("core state poisoned").snapshot()
}

/// UI 侧（滑块/预设/最近媒体）调速后同步权威值；返回收口后的实际倍速
#[tauri::command]
pub fn set_rate(app: AppHandle, state: State<CoreState>, rate: f64) -> f64 {
    let (rate, remembered) = {
        let mut core = state.lock().expect("core state poisoned");
        core.rate = clamp_rate(rate, RATE_MAX);
        (core.rate, core.remember_rate())
    };
    if remembered {
        persist::save_memory_debounced(&app);
    }
    rate
}

/// 设置页保存（前端每次变更推送完整设置对象；Rust 侧为权威并落盘）。
/// 涉及系统状态的项（开机自启）在此对齐；preservesPitch 变化即时广播给已连接的浏览器扩展。
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<CoreState>,
    mut settings: Settings,
) -> Result<(), String> {
    settings.normalize();
    let (pitch_changed, autostart_changed) = {
        let mut core = state.lock().expect("core state poisoned");
        let prev = core.settings.clone();
        core.settings = settings.clone();
        // 上限调低时收拢当前倍速（与前端 store 行为一致）
        core.rate = clamp_rate(core.rate, core.settings.slider_max);
        persist::save(&app, &core)?;
        (
            prev.preserves_pitch != settings.preserves_pitch,
            prev.start_on_boot != settings.start_on_boot,
        )
    };
    if pitch_changed {
        nm_bridge::set_preserves_pitch(settings.preserves_pitch);
    }
    if autostart_changed {
        crate::sync_autostart(&app, settings.start_on_boot);
    }
    Ok(())
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

// ---------- M2：应用规则与当前媒体 ----------

pub(crate) fn apps_snapshot(core: &Core) -> Vec<AppInfo> {
    // 先扫进程再组装，进程扫描（数十毫秒）不放在持锁热路径里由调用方保证
    let running = running_processes();
    core.rules
        .iter()
        .map(|r| to_app_info(r, &running, &core.connected_browsers))
        .collect()
}

/// 应用页列表：内置 + 用户规则，附运行状态
#[tauri::command]
pub fn list_apps(state: State<CoreState>) -> Vec<AppInfo> {
    let core = state.lock().expect("core state poisoned");
    apps_snapshot(&core)
}

/// 保存应用规则（应用页右侧编辑器）。内置规则只允许改控制方式/按键/IPC 参数，
/// 未知 id 视为自定义规则整条新增（PRD US-4：给未知播放器绑定按键）。
/// 返回更新后的完整列表，并向所有窗口广播 apps:status-changed。
#[tauri::command]
pub fn save_app_rule(
    app: AppHandle,
    state: State<CoreState>,
    rule: AppRulePatch,
) -> Result<Vec<AppInfo>, String> {
    let mut core = state.lock().expect("core state poisoned");
    if let Some(existing) = core.rules.iter_mut().find(|r| r.id == rule.id) {
        existing.method = rule.method;
        existing.keys = rule.keys;
        existing.ipc_config = rule.ipc_config;
        if !existing.builtin {
            existing.name = rule.name;
            existing.process = rule.process.to_lowercase();
        }
    } else {
        core.rules.push(crate::rules::AppRule {
            id: rule.id,
            name: rule.name,
            process: rule.process.to_lowercase(),
            aliases: Vec::new(),
            kind: crate::rules::AppKind::Unknown,
            method: rule.method,
            // 自定义规则 M2 只支持按键通道（IPC 类型选择随内置规则走）
            ipc: crate::rules::IpcKind::None,
            ipc_config: rule.ipc_config,
            keys: rule.keys,
            // 倍速网格/档位表来自内置规则的真机调研与源码取证，自定义规则暂不开放配置
            key_rate: None,
            rate_ladder: None,
            builtin: false,
        });
    }
    persist::save(&app, &core)?;
    let infos = apps_snapshot(&core);
    drop(core);
    let _ = app.emit("apps:status-changed", &infos);
    Ok(infos)
}

// ---------- M3.5：站点级规则 ----------

/// 应用页「网站适配」列表（开发文档 §9 list_site_rules）
#[tauri::command]
pub fn list_site_rules(state: State<CoreState>) -> Vec<SiteRule> {
    state.lock().expect("core state poisoned").site_rules.clone()
}

/// 保存单条站点规则（host 为键；未知 host 追加为自定义站点）。
/// 落盘后把新规则表即时推送给所有已连接浏览器扩展；返回更新后的完整列表
#[tauri::command]
pub fn save_site_rule(
    app: AppHandle,
    state: State<CoreState>,
    mut rule: SiteRule,
) -> Result<Vec<SiteRule>, String> {
    rule.normalize();
    if rule.host.is_empty() {
        return Err("站点 host 不能为空".into());
    }
    let rules = {
        let mut core = state.lock().expect("core state poisoned");
        if let Some(existing) = core.site_rules.iter_mut().find(|r| r.host == rule.host) {
            let name = existing.name.clone();
            *existing = rule;
            existing.name = name; // 内置站点名以代码为准（自定义站点首存时已带名）
        } else {
            core.site_rules.push(rule);
        }
        persist::save(&app, &core)?;
        core.site_rules.clone()
    };
    nm_bridge::set_site_rules(&rules);
    Ok(rules)
}

// ---------- M4.5：平台桌面客户端（CDP 接管） ----------

/// 接管 Chromium 套壳客户端：确保其带 CDP 调试口运行（应用页「接管」按钮）。
/// 调试口已在线 → 直接返回；在运行但无调试口 → 结束进程后带参重启；
/// 未运行 → 报错引导用户先打开客户端。轮询等待较久，放 blocking 池不占事件循环。
#[tauri::command]
pub async fn takeover_client(app: AppHandle, id: String) -> Result<String, String> {
    let (name, process, aliases, port) = {
        let state = app.state::<CoreState>();
        let core = state.lock().expect("core state poisoned");
        let rule = core
            .rules
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| format!("未知应用规则：{id}"))?;
        if rule.ipc != crate::rules::IpcKind::Cdp {
            return Err(format!("{} 不使用 CDP 接管", rule.name));
        }
        let port = rule
            .ipc_config
            .as_ref()
            .and_then(|c| c.port)
            .unwrap_or(crate::adapters::CDP_DEFAULT_PORT);
        (rule.name.clone(), rule.process.clone(), rule.aliases.clone(), port)
    };
    tauri::async_runtime::spawn_blocking(move || takeover_blocking(&name, &process, &aliases, port))
        .await
        .map_err(|e| e.to_string())?
}

fn takeover_blocking(
    name: &str,
    process: &str,
    aliases: &[String],
    port: u16,
) -> Result<String, String> {
    use player_ipc::CdpClient;
    use std::time::Duration;
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let client = CdpClient::new(port);
    if client.is_available() {
        return Ok(format!("「{name}」已在接管中（本机控制端口 {port} 在线）"));
    }

    let name_matches = |n: &str| n == process || aliases.iter().any(|a| a == n);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    let running: Vec<_> = sys
        .processes()
        .values()
        .filter(|p| name_matches(&p.name().to_string_lossy().to_lowercase()))
        .collect();
    let Some(exe) = running.iter().find_map(|p| p.exe().map(|e| e.to_path_buf())) else {
        return Err(format!(
            "「{name}」未在运行。请先打开它再点「接管」（接管会重启客户端并开启仅本机可见的控制端口）"
        ));
    };

    // Electron 单实例锁：不退干净的话，新实例会把参数转交旧实例后自行退出，参数即丢失
    for p in &running {
        p.kill();
    }
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(200));
        let mut probe = System::new();
        probe.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        if !probe
            .processes()
            .values()
            .any(|p| name_matches(&p.name().to_string_lossy().to_lowercase()))
        {
            break;
        }
    }

    std::process::Command::new(&exe)
        .arg(format!("--remote-debugging-port={port}"))
        .spawn()
        .map_err(|e| format!("重启 {} 失败：{e}", exe.display()))?;

    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(200));
        if client.is_available() {
            return Ok(format!(
                "已接管「{name}」：控制端口 {port} 在线，播放视频后全局快捷键即刻可用"
            ));
        }
    }
    Err(format!(
        "「{name}」已重启，但控制端口 {port} 未上线；该客户端版本可能不接受调试参数"
    ))
}

/// 控制页「当前媒体」。倍速回读在 router 集成后填充（可回读的适配器以真实值为准）
#[tauri::command]
pub fn get_current_media(state: State<CoreState>) -> Option<MediaSession> {
    let core = state.lock().expect("core state poisoned");
    core.current_session(None)
}

/// 暂停/恢复全局监听（控制页右上角）。暂停时对外表现为无媒体
#[tauri::command]
pub fn set_listening(app: AppHandle, state: State<CoreState>, enabled: bool) {
    let mut core = state.lock().expect("core state poisoned");
    core.listening = enabled;
    let session = core.current_session(None);
    drop(core);
    let _ = app.emit("media:changed", &session);
}

/// 「应用到当前媒体」：把选定倍速下发到当前接管对象；返回目标倍速
/// （下发是异步的，可回读通道的真实值随后经 media:changed 校正）
#[tauri::command]
pub fn apply_to_current(app: AppHandle, state: State<CoreState>, rate: f64) -> Result<f64, String> {
    let (target, remembered) = {
        let mut core = state.lock().expect("core state poisoned");
        core.rate = clamp_rate(rate, RATE_MAX);
        (core.rate, core.remember_rate())
    };
    if remembered {
        persist::save_memory_debounced(&app);
    }
    crate::router::push_rate_async(&app, crate::router::PushMode::ExactOnly);
    Ok(target)
}
