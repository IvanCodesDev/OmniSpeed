//! Chromium 套壳客户端 CDP 控制通道（开发文档「平台桌面客户端适配」）。
//!
//! 哔哩哔哩桌面端 / 抖音桌面版等客户端是 Electron/CEF 套壳：无公开控制接口、
//! 浏览器扩展也装不进去，但只要以 `--remote-debugging-port={port}` 启动，
//! 就会开启仅本机回环可见的 Chrome DevTools Protocol 调试口。本客户端经由它
//! `Runtime.evaluate` 直接读写页面里 `HTMLMediaElement.playbackRate`——
//! 能力与浏览器扩展同级：0.25×–16× 精确设速 + 回读，无需窗口前台。
//!
//! 通道分两段（都在 127.0.0.1，不出本机）：
//! 1. HTTP `GET /json/list` 枚举页面目标（B 站客户端的播放器是独立 player.html 页）；
//! 2. WebSocket 连各目标的 `webSocketDebuggerUrl` 发 `Runtime.evaluate`。
//!
//! 带参启动/重启客户端（「接管」）由主程序完成；本模块只做纯 IO，可离线单测。

use crate::IpcError;
use serde::Deserialize;
use std::net::TcpStream;
use std::time::Duration;

/// 本机回环上的小 JSON 应答：连接 400ms、整体 1.5s 已给足余量，
/// 同时保证未接管（端口未开）时热键路径能快速失败返回。
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
const IO_TIMEOUT: Duration = Duration::from_millis(1500);

/// CDP 调试口客户端（`http://127.0.0.1:{port}`，由「接管」时的启动参数决定）。
pub struct CdpClient {
    port: u16,
}

/// `/json/list` 条目（只取用到的字段）
#[derive(Debug, Clone, Deserialize)]
struct RawTarget {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    url: String,
    #[serde(rename = "webSocketDebuggerUrl", default)]
    ws_url: Option<String>,
}

impl CdpClient {
    pub fn new(port: u16) -> Self {
        Self { port }
    }

    /// 调试口是否在线（= 客户端已被接管）。传输层失败才算不可用。
    pub fn is_available(&self) -> bool {
        self.http_get("/json/version").is_ok()
    }

    /// 一步设置精确倍速：对所有含媒体元素的页面生效，回读活跃媒体的真实值。
    /// 所有页面都没有媒体元素 → Protocol 错误（客户端开着但没在播放）。
    pub fn set_rate(&self, rate: f64) -> Result<Option<f64>, IpcError> {
        let expr = format!(
            "{MEDIA_PRELUDE} let applied = null; for (const m of media) {{ try {{ m.playbackRate = {rate}; applied = m.playbackRate; }} catch (e) {{}} }} return active.playbackRate ?? applied; }})()"
        );
        let mut read_back = None;
        let mut any_media = false;
        for ws_url in self.page_targets()? {
            // 单页失败不拖累其它页（页面可能正在关闭/导航）
            match self.evaluate(&ws_url, &expr) {
                Ok(serde_json::Value::Null) | Err(_) => {}
                Ok(v) => {
                    any_media = true;
                    if read_back.is_none() {
                        read_back = v.as_f64();
                    }
                }
            }
        }
        if !any_media {
            return Err(IpcError::Protocol("客户端页面里没有媒体元素".into()));
        }
        Ok(read_back)
    }

    /// 回读当前倍速：取第一个含媒体元素页面的活跃媒体（优先未暂停者）。
    pub fn get_rate(&self) -> Result<f64, IpcError> {
        let expr = format!("{MEDIA_PRELUDE} return active.playbackRate; }})()");
        for ws_url in self.page_targets()? {
            if let Ok(v) = self.evaluate(&ws_url, &expr) {
                if let Some(rate) = v.as_f64() {
                    return Ok(rate);
                }
            }
        }
        Err(IpcError::Protocol("客户端页面里没有媒体元素".into()))
    }

    /// 播放/暂停活跃媒体（第一个命中媒体元素的页面）。
    pub fn play_pause(&self) -> Result<(), IpcError> {
        let expr = format!(
            "{MEDIA_PRELUDE} if (active.paused) {{ const p = active.play(); if (p && p.catch) p.catch(() => {{}}); }} else {{ active.pause(); }} return true; }})()"
        );
        for ws_url in self.page_targets()? {
            if let Ok(v) = self.evaluate(&ws_url, &expr) {
                if v.as_bool() == Some(true) {
                    return Ok(());
                }
            }
        }
        Err(IpcError::Protocol("客户端页面里没有媒体元素".into()))
    }

    /// 枚举可注入的页面目标，返回排好序的 WebSocket 调试地址：
    /// url 含 "player" 的页面优先（B 站客户端的播放器是独立 player.html 窗口），
    /// OOPIF/webview 目标一并纳入以防媒体活在子框架里。
    fn page_targets(&self) -> Result<Vec<String>, IpcError> {
        let body = self.http_get("/json/list")?;
        let raw: Vec<RawTarget> = serde_json::from_str(&body)
            .map_err(|e| IpcError::Protocol(format!("/json/list 不是合法 JSON：{e}")))?;
        let mut targets: Vec<(bool, String)> = raw
            .into_iter()
            .filter(|t| matches!(t.kind.as_str(), "page" | "iframe" | "webview"))
            .filter_map(|t| t.ws_url.map(|ws| (t.url.contains("player"), ws)))
            .collect();
        targets.sort_by_key(|(is_player, _)| !*is_player);
        Ok(targets.into_iter().map(|(_, ws)| ws).collect())
    }

    /// 调试口 HTTP GET；错误归类与 VLC 通道一致：传输层失败 → Unavailable。
    fn http_get(&self, path: &str) -> Result<String, IpcError> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout(IO_TIMEOUT)
            .build();
        match agent.get(&format!("http://127.0.0.1:{}{path}", self.port)).call() {
            Ok(resp) => resp
                .into_string()
                .map_err(|e| IpcError::Protocol(format!("读取 CDP 应答失败：{e}"))),
            Err(ureq::Error::Status(code, _)) => {
                Err(IpcError::Protocol(format!("CDP 调试口返回 HTTP {code}")))
            }
            Err(ureq::Error::Transport(_)) => Err(IpcError::Unavailable),
        }
    }

    /// 对单个目标执行 `Runtime.evaluate`，返回 `result.result.value`。
    /// 只等本次调用的应答（id 匹配），跳过事件帧；读超时由底层 TcpStream 保证。
    fn evaluate(&self, ws_url: &str, expr: &str) -> Result<serde_json::Value, IpcError> {
        let addr = ws_addr(ws_url)
            .ok_or_else(|| IpcError::Protocol(format!("无法解析调试地址：{ws_url}")))?;
        let sock_addr = addr
            .parse()
            .map_err(|e| IpcError::Protocol(format!("调试地址不合法（{addr}）：{e}")))?;
        let stream =
            TcpStream::connect_timeout(&sock_addr, CONNECT_TIMEOUT).map_err(|_| IpcError::Unavailable)?;
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let (mut ws, _) = tungstenite::client(ws_url, stream)
            .map_err(|e| IpcError::Protocol(format!("CDP WebSocket 握手失败：{e}")))?;

        let call = serde_json::json!({
            "id": 1,
            "method": "Runtime.evaluate",
            "params": { "expression": expr, "returnByValue": true },
        });
        ws.send(tungstenite::Message::Text(call.to_string().into()))
            .map_err(|e| IpcError::Protocol(format!("CDP 发送失败：{e}")))?;

        loop {
            let msg = ws
                .read()
                .map_err(|e| IpcError::Protocol(format!("CDP 读取失败：{e}")))?;
            let text = match msg {
                tungstenite::Message::Text(t) => t,
                tungstenite::Message::Close(_) => {
                    return Err(IpcError::Protocol("CDP 连接被目标关闭".into()))
                }
                _ => continue,
            };
            let v: serde_json::Value = serde_json::from_str(text.as_str())
                .map_err(|e| IpcError::Protocol(format!("CDP 应答不是 JSON：{e}")))?;
            if v.get("id").and_then(serde_json::Value::as_i64) != Some(1) {
                continue; // 未订阅域也可能收到零星事件帧，跳过
            }
            if let Some(ex) = v.pointer("/result/exceptionDetails") {
                return Err(IpcError::Protocol(format!("页面脚本异常:{ex}")));
            }
            return Ok(v
                .pointer("/result/result/value")
                .cloned()
                .unwrap_or(serde_json::Value::Null));
        }
    }
}

/// 页面媒体枚举序言（IIFE 开头）：收集本页与可达同源 iframe 里的全部媒体元素，
/// `media` 为空直接返回 null；`active` 优先取未暂停者（多实例时它才是正在看的那个）。
const MEDIA_PRELUDE: &str = "(() => { const collect = (doc, out) => { for (const m of doc.querySelectorAll('video,audio')) out.push(m); for (const f of doc.querySelectorAll('iframe')) { try { if (f.contentDocument) collect(f.contentDocument, out); } catch (e) {} } return out; }; const media = collect(document, []); if (!media.length) return null; const active = media.find((m) => !m.paused) || media[0];";

/// 从 `ws://host:port/devtools/page/XX` 提取 `host:port`。
/// 生产环境恒为 127.0.0.1:{port}；单测用它把 WS 假服务器放在独立端口。
fn ws_addr(ws_url: &str) -> Option<&str> {
    let rest = ws_url.strip_prefix("ws://")?;
    Some(rest.split('/').next().unwrap_or(rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn ws_addr_extracts_host_port() {
        assert_eq!(
            ws_addr("ws://127.0.0.1:9333/devtools/page/AB12"),
            Some("127.0.0.1:9333")
        );
        assert_eq!(ws_addr("http://127.0.0.1:9333/x"), None);
    }

    /// 一次性 HTTP 服务器：收完请求头 → 回写 canned JSON → 关闭（同 vlc.rs 测试模式）
    fn spawn_json_server(body: String) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = Vec::new();
            let mut byte = [0u8; 1];
            while !buf.ends_with(b"\r\n\r\n") {
                match stream.read(&mut byte) {
                    Ok(1) => buf.push(byte[0]),
                    _ => break,
                }
            }
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        });
        port
    }

    /// WS 假目标：完成握手 → 收到 Runtime.evaluate → 先回一帧事件再回 canned 应答。
    /// 返回（端口，收到的 evaluate 请求文本接收端）。
    fn spawn_ws_target(result_json: String) -> (u16, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ws");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept ws");
            let mut ws = tungstenite::accept(stream).expect("ws handshake");
            let received = loop {
                match ws.read().expect("ws read") {
                    tungstenite::Message::Text(t) => break t.as_str().to_owned(),
                    _ => continue,
                }
            };
            let _ = tx.send(received);
            let event = r#"{"method":"Some.event","params":{}}"#;
            let _ = ws.send(tungstenite::Message::Text(event.into()));
            let _ = ws.send(tungstenite::Message::Text(result_json.into()));
            // 等对端读完再退出，避免连接提前断开
            std::thread::sleep(Duration::from_millis(300));
        });
        (port, rx)
    }

    /// /json/list 过滤与排序：只留 page/iframe/webview，url 含 player 的排最前
    #[test]
    fn page_targets_filters_and_sorts() {
        let body = r#"[
            {"type":"page","url":"https://x/index.html","webSocketDebuggerUrl":"ws://h/devtools/page/A"},
            {"type":"service_worker","url":"https://x/sw.js","webSocketDebuggerUrl":"ws://h/devtools/page/B"},
            {"type":"page","url":"https://x/player.html","webSocketDebuggerUrl":"ws://h/devtools/page/C"},
            {"type":"page","url":"https://x/no-ws.html"}
        ]"#;
        let port = spawn_json_server(body.into());
        let client = CdpClient::new(port);
        let targets = client.page_targets().unwrap();
        assert_eq!(
            targets,
            vec![
                "ws://h/devtools/page/C".to_string(),
                "ws://h/devtools/page/A".to_string()
            ]
        );
    }

    /// evaluate 全链路：握手 → 发送表达式 → 跳过事件帧 → 解析 id 匹配的应答值
    #[test]
    fn evaluate_roundtrip_skips_event_frames() {
        let (ws_port, rx) = spawn_ws_target(
            r#"{"id":1,"result":{"result":{"type":"number","value":1.5}}}"#.into(),
        );
        let client = CdpClient::new(0); // evaluate 的地址来自 ws_url，本端口不参与
        let value = client
            .evaluate(&format!("ws://127.0.0.1:{ws_port}/devtools/page/T"), "1+0.5")
            .unwrap();
        assert_eq!(value.as_f64(), Some(1.5));
        let sent = rx.recv().expect("应收到 evaluate 请求");
        assert!(sent.contains("Runtime.evaluate"));
        assert!(sent.contains("1+0.5"));
    }

    /// set_rate 端到端（假 HTTP 列表 + 假 WS 目标）：表达式带目标倍速，回读真实值
    #[test]
    fn set_rate_end_to_end_via_fakes() {
        let (ws_port, rx) = spawn_ws_target(
            r#"{"id":1,"result":{"result":{"type":"number","value":2}}}"#.into(),
        );
        let list = format!(
            r#"[{{"type":"page","url":"https://x/player.html","webSocketDebuggerUrl":"ws://127.0.0.1:{ws_port}/devtools/page/T"}}]"#
        );
        let http_port = spawn_json_server(list);
        let client = CdpClient::new(http_port);
        assert_eq!(client.set_rate(2.0).unwrap(), Some(2.0));
        let sent = rx.recv().unwrap();
        assert!(sent.contains("playbackRate = 2"), "表达式应带目标倍速：{sent}");
    }

    /// 页面脚本异常 → Protocol 错误（不至于误当成功）
    #[test]
    fn evaluate_exception_maps_to_protocol() {
        let (ws_port, _rx) = spawn_ws_target(
            r#"{"id":1,"result":{"result":{"type":"object"},"exceptionDetails":{"text":"boom"}}}"#
                .into(),
        );
        let client = CdpClient::new(0);
        let err = client
            .evaluate(&format!("ws://127.0.0.1:{ws_port}/devtools/page/T"), "throw 1")
            .unwrap_err();
        assert!(matches!(err, IpcError::Protocol(_)));
    }

    /// 端口拒绝连接（未接管）→ Unavailable / is_available = false
    #[test]
    fn refused_port_maps_to_unavailable() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let client = CdpClient::new(port);
        assert!(!client.is_available());
        assert!(matches!(client.set_rate(2.0), Err(IpcError::Unavailable)));
        assert!(matches!(client.get_rate(), Err(IpcError::Unavailable)));
    }
}
