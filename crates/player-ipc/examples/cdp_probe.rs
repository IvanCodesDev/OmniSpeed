//! CDP 通道真机冒烟工具（开发用，不进产物）：
//!
//! ```text
//! cargo run --example cdp_probe -- <port>              # 状态：调试口在线 + 回读倍速
//! cargo run --example cdp_probe -- <port> set <rate>   # 设速并回读
//! cargo run --example cdp_probe -- <port> pp           # 播放/暂停
//! cargo run --example cdp_probe -- <port> nav <url>    # 首个 page 目标 location.assign（造测试现场用）
//! cargo run --example cdp_probe -- <port> ls           # 列出调试目标
//! cargo run --example cdp_probe -- <port> eval <js>    # 播放器优先首目标执行任意 JS（模拟站点复位等）
//! cargo run --example cdp_probe -- <port> guard <secs> # 持有 GuardSession N 秒（验证导航续速）
//! ```
//!
//! 前置：目标客户端已以 `--remote-debugging-port=<port>` 启动。

use player_ipc::{CdpClient, GuardSession};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port: u16 = args
        .first()
        .and_then(|p| p.parse().ok())
        .expect("用法：cdp_probe <port> [set <rate> | pp | nav <url> | ls]");
    let client = CdpClient::new(port);

    match args.get(1).map(String::as_str) {
        None => {
            println!("available = {}", client.is_available());
            match client.get_rate() {
                Ok(rate) => println!("get_rate  = {rate}"),
                Err(e) => println!("get_rate  ! {e}"),
            }
        }
        Some("set") => {
            let rate: f64 = args.get(2).and_then(|r| r.parse().ok()).expect("set 需要倍速参数");
            match client.set_rate(rate) {
                Ok(read_back) => println!("set_rate({rate}) = 回读 {read_back:?}"),
                Err(e) => println!("set_rate({rate}) ! {e}"),
            }
        }
        Some("pp") => match client.play_pause() {
            Ok(()) => println!("play_pause = ok"),
            Err(e) => println!("play_pause ! {e}"),
        },
        Some("ls") => match http_get(port, "/json/list") {
            Ok(body) => println!("{body}"),
            Err(e) => println!("ls ! {e}"),
        },
        Some("nav") => {
            let url = args.get(2).expect("nav 需要 URL 参数");
            match navigate_first_page(port, url) {
                Ok(target) => println!("nav → {target}"),
                Err(e) => println!("nav ! {e}"),
            }
        }
        Some("eval") => {
            let expr = args.get(2).expect("eval 需要 JS 表达式参数");
            match client.eval_in_player(expr) {
                Ok(v) => println!("eval = {v}"),
                Err(e) => println!("eval ! {e}"),
            }
        }
        Some("guard") => {
            let secs: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            let targets = client.page_targets().expect("枚举目标失败");
            let ws_url = targets.first().expect("客户端没有页面目标");
            let mut session = GuardSession::open(ws_url).expect("GuardSession::open 失败");
            println!("guard 已注册（on-new-document + 当前文档），持有 {secs}s：{ws_url}");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
            while std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_secs(2));
                match session.heartbeat() {
                    Ok(()) => println!("  heartbeat ok"),
                    Err(e) => {
                        println!("  heartbeat ! {e}（会话失效，退出）");
                        break;
                    }
                }
            }
            println!("guard 会话结束");
        }
        Some(other) => println!("未知子命令：{other}"),
    }
}

fn http_get(port: u16, path: &str) -> Result<String, String> {
    ureq::get(&format!("http://127.0.0.1:{port}{path}"))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())
}

/// 冒烟专用的独立导航实现（生产 API 不需要导航能力，这里自带一份最小 WS evaluate）
fn navigate_first_page(port: u16, url: &str) -> Result<String, String> {
    let body = http_get(port, "/json/list")?;
    let targets: Vec<serde_json::Value> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let target = targets
        .iter()
        .find(|t| t["type"] == "page" && t["webSocketDebuggerUrl"].is_string())
        .ok_or("没有 page 目标")?;
    let ws_url = target["webSocketDebuggerUrl"].as_str().unwrap();

    let addr = ws_url.strip_prefix("ws://").and_then(|r| r.split('/').next()).ok_or("ws url 不合法")?;
    let stream = std::net::TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("{e}"))?,
        std::time::Duration::from_millis(500),
    )
    .map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).ok();
    let (mut ws, _) = tungstenite::client(ws_url, stream).map_err(|e| e.to_string())?;
    let call = serde_json::json!({
        "id": 1,
        "method": "Runtime.evaluate",
        "params": { "expression": format!("location.assign({url:?}); 'ok'"), "returnByValue": true },
    });
    ws.send(tungstenite::Message::Text(call.to_string().into())).map_err(|e| e.to_string())?;
    loop {
        match ws.read().map_err(|e| e.to_string())? {
            tungstenite::Message::Text(t) => {
                let v: serde_json::Value = serde_json::from_str(t.as_str()).map_err(|e| e.to_string())?;
                if v["id"] == 1 {
                    return Ok(target["url"].as_str().unwrap_or("?").to_string());
                }
            }
            tungstenite::Message::Close(_) => return Err("连接被关闭".into()),
            _ => {}
        }
    }
}
