//! Native Messaging 桥（开发文档 §5.3）。
//!
//! 链路：浏览器 ⟷ (stdio，Chrome NM 帧) ⟷ 中继进程（`omnispeed.exe --nm-host`，
//! 由浏览器按注册表清单拉起）⟷ (命名管道，同帧格式) ⟷ 本模块的管道服务端（主程序内）。
//!
//! 为什么需要中继：Chrome 为每个浏览器实例自行拉起 host 进程并用 stdio 通信，
//! 而倍速状态的权威在常驻的主程序里——中继只做字节级双向转发，并在首帧前注入
//! `{"type":"hostInfo","browser":"msedge.exe"}`（沿父进程链探测浏览器身份）。
//!
//! 帧格式（Chrome NM 标准，管道上沿用）：4 字节小端长度前缀 + UTF-8 JSON。
//! 消息契约：apps/extension/src/shared/protocol.ts（冻结）。

use crate::state::{clamp_rate, BrowserMedia, Core, CoreState, CurrentTarget, RATE_MAX};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::sync::mpsc;

pub const PIPE_NAME: &str = r"\\.\pipe\omnispeed-nm";
pub const HOST_NAME: &str = "com.omnispeed.host";
/// 扩展 manifest.json 的 key 字段固定后得到的确定性 ID（见 apps/extension/README.md）
pub const EXTENSION_ID: &str = "ejpnpjbhmgckjfdednjgfhdpobencmpb";
/// Firefox 的 Gecko 扩展 ID（M3.5）：与扩展 build.mjs 的 GECKO_ID、
/// dist-firefox manifest 的 browser_specific_settings.gecko.id 保持一致
pub const GECKO_EXTENSION_ID: &str = "connector@omnispeed.app";

/// 单帧上限：NM 规范中扩展方向 1MB，这里给足余量防御异常帧
const MAX_FRAME: u32 = 4 * 1024 * 1024;

// ---------------------------------------------------------------------------
// 连接注册表：浏览器进程名 → 发送通道（adapters 经此下发命令）
// 同一浏览器多开（多 profile）时后连接者覆盖，M3 按单会话处理
// ---------------------------------------------------------------------------

static SENDERS: Mutex<Option<HashMap<String, mpsc::UnboundedSender<Value>>>> = Mutex::new(None);

fn senders_insert(browser: &str, tx: mpsc::UnboundedSender<Value>) {
    let mut guard = SENDERS.lock().expect("senders poisoned");
    guard.get_or_insert_with(HashMap::new).insert(browser.to_string(), tx);
}

fn senders_remove(browser: &str) {
    if let Some(map) = SENDERS.lock().expect("senders poisoned").as_mut() {
        map.remove(browser);
    }
}

fn sender_of(browser: &str) -> Option<mpsc::UnboundedSender<Value>> {
    SENDERS
        .lock()
        .expect("senders poisoned")
        .as_ref()
        .and_then(|m| m.get(browser).cloned())
}

/// 「变速不变调」设置的镜像：adapters 下发走的 config_frame 不持有 Core 锁，
/// 由设置加载/保存时经 set_preserves_pitch 同步
static PRESERVES_PITCH: AtomicBool = AtomicBool::new(true);

/// 站点级规则的镜像（M3.5，同上不持有 Core 锁）：已按扩展协议 SiteRuleConfig
/// 组好的 JSON 数组，由规则加载/保存时经 set_site_rules 同步
static SITE_RULES: Mutex<Vec<Value>> = Mutex::new(Vec::new());

/// 设置变化时更新镜像，并把新 config 推给所有已连接浏览器（targetRate 留空不改倍速）
pub fn set_preserves_pitch(on: bool) {
    PRESERVES_PITCH.store(on, Ordering::Relaxed);
    broadcast_config();
}

/// 站点规则加载/保存时同步镜像并即时推送（扩展协议 protocol.ts SiteRuleConfig：
/// 只带扩展侧要用的字段，defaultRate 的进站恢复由桌面侧 on_frame 负责）
pub fn set_site_rules(rules: &[crate::state::SiteRule]) {
    let frames: Vec<Value> = rules
        .iter()
        .map(|r| {
            json!({
                "host": r.host,
                "maxRate": r.max_rate,
                "rateLock": r.rate_lock,
                "follow": r.follow,
            })
        })
        .collect();
    *SITE_RULES.lock().expect("site rules poisoned") = frames;
    broadcast_config();
}

/// 把当前 config 推给所有已连接浏览器（targetRate 留空不改倍速）
fn broadcast_config() {
    let senders: Vec<mpsc::UnboundedSender<Value>> = SENDERS
        .lock()
        .expect("senders poisoned")
        .as_ref()
        .map(|m| m.values().cloned().collect())
        .unwrap_or_default();
    for tx in senders {
        let _ = tx.send(config_frame(None));
    }
}

fn config_frame(target_rate: Option<f64>) -> Value {
    // 全局 rateLock 恒开；站点级锁定/上限/跟随由 siteRules 承载，
    // 扩展内容脚本按自身 host 合成生效值（M3.5）
    json!({
        "type": "config",
        "config": {
            "targetRate": target_rate,
            "rateLock": true,
            "maxRate": RATE_MAX,
            "preservesPitch": PRESERVES_PITCH.load(Ordering::Relaxed),
            "siteRules": SITE_RULES.lock().expect("site rules poisoned").clone(),
        }
    })
}

/// 向浏览器下发「设为精确倍速」：setRate 立即生效 + config 更新会话目标
/// （新页面 / 短视频流滑动的跟随恢复依赖 config.targetRate）
pub fn send_set_rate(browser: &str, rate: f64) -> Result<(), String> {
    let Some(tx) = sender_of(browser) else {
        return Err("浏览器扩展未连接".into());
    };
    tx.send(json!({ "type": "setRate", "rate": rate }))
        .and_then(|_| tx.send(config_frame(Some(rate))))
        .map_err(|_| "扩展连接已断开".to_string())
}

pub fn send_play_pause(browser: &str) -> Result<(), String> {
    let tx = sender_of(browser).ok_or("浏览器扩展未连接")?;
    tx.send(json!({ "type": "playPause" }))
        .map_err(|_| "扩展连接已断开".to_string())
}

/// 前台尚未匹配到播放器时，把已连接的浏览器收为当前接管对象。
/// 这样焦点在编辑器里也能继续遥控网页视频（开发文档 §7.1「统一遥控器」）。
fn adopt_browser_if_idle(core: &mut Core, browser: &str) {
    if core.current.is_some() {
        return;
    }
    let Some(rule_id) = core
        .rules
        .iter()
        .find(|r| r.matches(browser))
        .map(|r| r.id.clone())
    else {
        return;
    };
    let hwnd = platform_win::find_window_by_process(browser).unwrap_or(0);
    core.current = Some(CurrentTarget {
        rule_id,
        hwnd,
        process_name: browser.to_string(),
    });
    eprintln!("[nm] 空闲接管 → {browser}");
}

// ---------------------------------------------------------------------------
// 管道服务端（主程序内）
// ---------------------------------------------------------------------------

/// 启动命名管道服务端（应用生命周期内常驻）
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut first = true;
        loop {
            let server = match ServerOptions::new()
                .first_pipe_instance(first)
                .create(PIPE_NAME)
            {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("[nm] 管道创建失败（{PIPE_NAME}）：{err}");
                    return;
                }
            };
            first = false;
            if server.connect().await.is_err() {
                continue;
            }
            let conn_app = handle.clone();
            tauri::async_runtime::spawn(async move {
                handle_connection(conn_app, server).await;
            });
        }
    });
}

async fn handle_connection(app: AppHandle, pipe: NamedPipeServer) {
    let (mut reader, mut writer) = tokio::io::split(pipe);

    // 首帧必须是中继注入的 hostInfo，据此确定这条连接属于哪个浏览器
    let browser = match read_frame(&mut reader).await {
        Ok(Some(frame)) if frame["type"] == "hostInfo" => frame["browser"]
            .as_str()
            .unwrap_or("unknown")
            .to_lowercase(),
        _ => return,
    };

    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    senders_insert(&browser, tx);

    // 写任务：命令队列 → 管道
    let write_task = tauri::async_runtime::spawn(async move {
        while let Some(frame) = rx.recv().await {
            if write_frame(&mut writer, &frame).await.is_err() {
                break;
            }
        }
    });

    // 读循环：hello / media 帧
    while let Ok(Some(frame)) = read_frame(&mut reader).await {
        on_frame(&app, &browser, frame);
    }

    // 断开清理：连接表、媒体状态、前端状态广播
    eprintln!("[nm] 扩展连接断开：{browser}");
    senders_remove(&browser);
    write_task.abort();
    let (session, apps) = {
        let state = app.state::<CoreState>();
        let mut core = state.lock().expect("core state poisoned");
        core.connected_browsers.remove(&browser);
        core.browser_media.remove(&browser);
        let is_current = core
            .current
            .as_ref()
            .map(|t| t.process_name == browser)
            .unwrap_or(false);
        (
            is_current.then(|| core.current_session(None)),
            crate::commands::apps_snapshot(&core),
        )
    };
    let _ = app.emit("apps:status-changed", &apps);
    if let Some(session) = session {
        let _ = app.emit("media:changed", &session);
    }
}

fn on_frame(app: &AppHandle, browser: &str, frame: Value) {
    match frame["type"].as_str() {
        Some("hello") => {
            eprintln!("[nm] 扩展已连接：{browser}");
            let (apps, session) = {
                let state = app.state::<CoreState>();
                let mut core = state.lock().expect("core state poisoned");
                core.connected_browsers.insert(browser.to_string());
                adopt_browser_if_idle(&mut core, browser);
                let is_current = core
                    .current
                    .as_ref()
                    .map(|t| t.process_name == browser)
                    .unwrap_or(false);
                (
                    crate::commands::apps_snapshot(&core),
                    is_current.then(|| core.current_session(None)),
                )
            };
            // 连接即下发当前配置（targetRate 为空：未调速前不主动改写页面倍速）
            if let Some(tx) = sender_of(browser) {
                let _ = tx.send(config_frame(None));
            }
            let _ = app.emit("apps:status-changed", &apps);
            if let Some(session) = session {
                let _ = app.emit("media:changed", &session);
            }
        }
        Some("media") => {
            let media = serde_json::from_value::<Option<BrowserMedia>>(frame["state"].clone())
                .unwrap_or(None);
            // 锁内只改状态；管道下发 / OSD 在锁外执行
            let mut restore: Option<f64> = None;
            let mut slowdown: Option<crate::hotkey::HotkeyPayload> = None;
            let session = {
                let state = app.state::<CoreState>();
                let mut core = state.lock().expect("core state poisoned");
                match media {
                    Some(mut m) => {
                        if m.has_media {
                            adopt_browser_if_idle(&mut core, browser);
                        }
                        let is_current = core
                            .current
                            .as_ref()
                            .map(|t| t.process_name == browser)
                            .unwrap_or(false);

                        // 进站恢复：只在「上一帧 host → 本帧 host」的变化沿恢复一次，
                        // 心跳帧不会重复触发，也不会打断热键正在下发的目标。
                        // 优先级：按网站记忆（设置开启时，开发文档 §7.5）
                        //   → 站点规则默认倍速（M3.5 siteRules.defaultRate）→ 不干预
                        if is_current && m.has_media && !m.is_live {
                            let prev_host = core
                                .browser_media
                                .get(browser)
                                .filter(|p| p.has_media)
                                .map(|p| p.host.clone());
                            if prev_host.as_deref() != Some(m.host.as_str()) {
                                let remembered = if core.settings.remember_per_app {
                                    core.memory.get(&m.host).copied()
                                } else {
                                    None
                                };
                                let site_default = core
                                    .site_rule_for(&m.host)
                                    .and_then(|r| r.default_rate);
                                if let Some(saved) = remembered.or(site_default) {
                                    let target = clamp_rate(saved, RATE_MAX);
                                    core.rate = target;
                                    // 预写缓存：UI 即刻显示恢复值，页面随 setRate 收敛
                                    m.rate = target;
                                    restore = Some(target);
                                }
                            }
                        }

                        // 热键目标值由 router 下发；此处只缓存扩展上报，不回写 core.rate，
                        // 否则滞后的 1× 心跳会把正在步进的目标打回去。
                        core.browser_media.insert(browser.to_string(), m);

                        if restore.is_none() {
                            slowdown = maybe_smart_slowdown(&mut core, browser, is_current);
                        }
                        is_current.then(|| core.current_session(None))
                    }
                    None => {
                        core.browser_media.remove(browser);
                        let is_current = core
                            .current
                            .as_ref()
                            .map(|t| t.process_name == browser)
                            .unwrap_or(false);
                        is_current.then(|| core.current_session(None))
                    }
                }
            };
            if let Some(rate) = restore {
                let _ = send_set_rate(browser, rate);
            }
            if let Some(payload) = slowdown {
                let _ = send_set_rate(browser, payload.rate);
                // 广播给主窗口同步倍速显示，OSD 弹出降速提示
                let _ = app.emit("hotkey:triggered", payload.clone());
                crate::osd::show(app, &payload);
            }
            if let Some(session) = session {
                let _ = app.emit("media:changed", &session);
            }
        }
        _ => {}
    }
}

/// 智能降速（开发文档 §7.8，设置项）：缓冲前沿余量 < 目标倍速 × 5s 时
/// 自动回落到「缓冲能撑住」的档位（0.25 一档，至少 1×）。
/// 8s 冷却避免与缓冲恢复来回震荡；v1 不自动回升，由用户手动升速。
fn maybe_smart_slowdown(
    core: &mut Core,
    browser: &str,
    is_current: bool,
) -> Option<crate::hotkey::HotkeyPayload> {
    if !is_current || !core.settings.smart_slowdown {
        return None;
    }
    let media = core.browser_media.get(browser)?;
    if !media.has_media || media.is_live || media.ad_playing {
        return None;
    }
    let buffered = media.buffered_ahead?;
    if core.rate <= 1.05 || buffered >= core.rate * 5.0 {
        return None;
    }
    if core
        .slowdown_at
        .is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(8))
    {
        return None;
    }
    let sustainable = ((buffered / 5.0) * 4.0).floor() / 4.0;
    let target = clamp_rate(sustainable.max(1.0), RATE_MAX);
    if target >= core.rate - 0.01 {
        return None;
    }
    core.rate = target;
    core.slowdown_at = Some(std::time::Instant::now());
    core.osd_seq += 1;
    Some(crate::hotkey::HotkeyPayload {
        action: crate::state::ShortcutAction::SpeedDown,
        rate: target,
        seq: core.osd_seq,
        notice: Some("缓冲不足，已自动降速".into()),
    })
}

// ---------------------------------------------------------------------------
// 帧编解码（异步：管道服务端用）
// ---------------------------------------------------------------------------

async fn read_frame<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> std::io::Result<Option<Value>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf);
    if len == 0 || len > MAX_FRAME {
        return Ok(None);
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body).await?;
    Ok(serde_json::from_slice(&body).ok())
}

async fn write_frame<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, frame: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(frame)?;
    w.write_all(&(body.len() as u32).to_le_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await
}

// ---------------------------------------------------------------------------
// 注册（应用启动时执行）：宿主清单 + HKCU 注册表项，无需管理员
// ---------------------------------------------------------------------------

/// 写宿主清单与 .bat 启动器，并注册到 Chrome / Edge / Firefox 的 HKCU 路径。
/// Chrome 的 NM 清单不支持给 host 传参，因此用生成的 .bat 包一层 `--nm-host`。
/// Firefox（M3.5）清单格式不同（allowed_extensions ↔ Gecko ID），单独一份文件，
/// 注册到 HKCU\Software\Mozilla\NativeMessagingHosts（清单键名两家一致，均为 HOST_NAME）。
pub fn register_host(app: &AppHandle) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| e.to_string())?
        .join("nm-host");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let launcher = dir.join("omnispeed-nm-host.bat");
    std::fs::write(
        &launcher,
        format!("@echo off\r\n\"{}\" --nm-host %*\r\n", exe.display()),
    )
    .map_err(|e| e.to_string())?;

    let write_manifest = |file: &str, allow_key: &str, allow_value: String| -> Result<std::path::PathBuf, String> {
        let path = dir.join(file);
        let manifest = json!({
            "name": HOST_NAME,
            "description": "OmniSpeed Native Messaging host（浏览器扩展 ⟷ 桌面核心）",
            "path": launcher.to_string_lossy(),
            "type": "stdio",
            allow_key: [allow_value],
        });
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(path)
    };

    let chromium_manifest = write_manifest(
        &format!("{HOST_NAME}.json"),
        "allowed_origins",
        format!("chrome-extension://{EXTENSION_ID}/"),
    )?;
    let firefox_manifest = write_manifest(
        &format!("{HOST_NAME}.firefox.json"),
        "allowed_extensions",
        GECKO_EXTENSION_ID.to_string(),
    )?;

    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let registrations = [
        (r"Software\Google\Chrome\NativeMessagingHosts", &chromium_manifest),
        (r"Software\Microsoft\Edge\NativeMessagingHosts", &chromium_manifest),
        (r"Software\Mozilla\NativeMessagingHosts", &firefox_manifest),
    ];
    for (root, manifest_path) in registrations {
        let (key, _) = hkcu
            .create_subkey(format!(r"{root}\{HOST_NAME}"))
            .map_err(|e| e.to_string())?;
        key.set_value("", &manifest_path.to_string_lossy().to_string())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 中继模式（`omnispeed.exe --nm-host`，由浏览器拉起的独立进程）
// ---------------------------------------------------------------------------

/// 字节级双向转发：stdio ⟷ 主程序管道。帧格式两端一致，无需解析，
/// 仅在开头向主程序注入 hostInfo 帧。任一方向断开即退出。
pub fn relay_main() {
    // 主程序可能刚在启动，小幅重试
    let mut pipe = None;
    for _ in 0..6 {
        match std::fs::OpenOptions::new().read(true).write(true).open(PIPE_NAME) {
            Ok(f) => {
                pipe = Some(f);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
    let Some(mut pipe_w) = pipe else { std::process::exit(1) };
    let mut pipe_r = pipe_w.try_clone().unwrap_or_else(|_| std::process::exit(1));

    // hostInfo 前导帧：沿父进程链找到拉起本进程的浏览器
    let host = detect_browser();
    let host_info = json!({ "type": "hostInfo", "browser": host });
    if write_frame_sync(&mut pipe_w, &host_info).is_err() {
        std::process::exit(1);
    }

    // stdin → 管道
    let up = std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let _ = std::io::copy(&mut stdin, &mut pipe_w);
    });
    // 管道 → stdout
    let down = std::thread::spawn(move || {
        let mut stdout = std::io::stdout().lock();
        let _ = std::io::copy(&mut pipe_r, &mut stdout);
    });

    // 任一方向结束（浏览器关闭 / 主程序退出）即整体退出
    while !up.is_finished() && !down.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    std::process::exit(0);
}

fn write_frame_sync<W: Write>(w: &mut W, frame: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(frame)?;
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// 沿父进程链探测浏览器（中继由浏览器经 .bat → cmd.exe 拉起，通常在 2–3 级内）
fn detect_browser() -> String {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    const BROWSERS: [&str; 4] = ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe"];

    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

    let mut pid = Pid::from_u32(std::process::id());
    for _ in 0..5 {
        let Some(proc_) = sys.process(pid) else { break };
        let name = proc_.name().to_string_lossy().to_lowercase();
        if BROWSERS.contains(&name.as_str()) {
            return name;
        }
        match proc_.parent() {
            Some(parent) => pid = parent,
            None => break,
        }
    }
    "unknown".into()
}
