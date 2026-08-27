//! VLC HTTP 接口客户端（开发文档 §7.3：`status.xml?command=rate&val=X` 一步设速、可回读）。
//!
//! 前置条件（应用页提供一键开启指引，§7.3）：
//! 1. VLC 勾选 Web 界面：工具 → 首选项 →（左下角显示全部）→ 界面 → 主界面 勾选 Web；
//! 2. 必须设置密码（Lua HTTP 密码；VLC ≥ 2.1 无密码则拒绝一切请求）；
//! 3. 默认端口 8080，对应 `appRules[].ipc = { "kind": "vlc-http", "port": 8080 }`（§8）。
//!
//! 协议参考：<https://wiki.videolan.org/VLC_HTTP_requests/>。
//! 认证是 HTTP Basic Auth，**用户名固定为空**，密码为用户配置值。
//! `rate` 命令的参数是浮点（`val=2.5`），只对当前播放项生效。

use crate::IpcError;
use std::time::Duration;

/// VLC HTTP 接口客户端（本机回环 `http://127.0.0.1:{port}`）。
pub struct VlcHttpClient {
    port: u16,
    password: String,
    agent: ureq::Agent,
}

impl VlcHttpClient {
    pub fn new(port: u16, password: impl Into<String>) -> Self {
        // 本机回环 + 极小的 XML 响应：连接 500ms、整体 2s 已给足余量，
        // 同时保证探测/调用失败能快速返回，不拖累热键路径（§6 调速时序）。
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_millis(500))
            .timeout(Duration::from_secs(2))
            .build();
        Self {
            port,
            password: password.into(),
            agent,
        }
    }

    /// 尝试连接判断可用性（不 panic，短超时）。
    ///
    /// 注意语义：只有传输层失败（连接拒绝/超时 → [`IpcError::Unavailable`]）才算不可用；
    /// 401 说明 Web 接口已在监听、只是密码不对——通道「可达」，让上层把 AuthFailed
    /// 作为独立的可诊断状态展示（应用页「需要设置」，开发文档 §3 状态对照）。
    /// 「端口拒绝 → false」的路径有单元测试覆盖。
    pub fn is_available(&self) -> bool {
        !matches!(self.get_status(None), Err(IpcError::Unavailable))
    }

    /// 一步设置精确倍速：`requests/status.xml?command=rate&val=X`（X 为浮点）。
    pub fn set_rate(&self, rate: f64) -> Result<(), IpcError> {
        self.get_status(Some(&rate_query(rate)))?;
        Ok(())
    }

    /// 回读当前倍速：解析 status.xml 中的 `<rate>` 值。
    pub fn get_rate(&self) -> Result<f64, IpcError> {
        extract_rate(&self.get_status(None)?)
    }

    /// 播放/暂停切换：`command=pl_pause`。
    pub fn play_pause(&self) -> Result<(), IpcError> {
        self.get_status(Some("command=pl_pause"))?;
        Ok(())
    }

    fn status_url(&self, query: Option<&str>) -> String {
        let base = format!("http://127.0.0.1:{}/requests/status.xml", self.port);
        match query {
            Some(q) => format!("{base}?{q}"),
            None => base,
        }
    }

    /// 发起一次 status.xml 请求并做错误归类：
    /// 401 → AuthFailed；其他非 2xx → Protocol；传输层失败（连接拒绝/超时）→ Unavailable。
    fn get_status(&self, query: Option<&str>) -> Result<String, IpcError> {
        let auth = format!("Basic {}", base64(format!(":{}", self.password).as_bytes()));
        match self
            .agent
            .get(&self.status_url(query))
            .set("Authorization", &auth)
            .call()
        {
            Ok(resp) => resp
                .into_string()
                .map_err(|e| IpcError::Protocol(format!("读取 VLC 响应失败：{e}"))),
            Err(ureq::Error::Status(401, _)) => Err(IpcError::AuthFailed),
            Err(ureq::Error::Status(code, _)) => {
                Err(IpcError::Protocol(format!("VLC 返回 HTTP {code}")))
            }
            Err(ureq::Error::Transport(_)) => Err(IpcError::Unavailable),
        }
    }
}

/// rate 命令的查询串。f64 的 Display 对整数值不带小数点（5 → "5"），
/// VLC 的 lua 侧用 tonumber 解析，两种写法都接受。
fn rate_query(rate: f64) -> String {
    format!("command=rate&val={rate}")
}

/// 从 status.xml 提取 `<rate>` 值。
///
/// 按开发文档要求不引 XML 库：status.xml 由 VLC 内置 lua http 接口生成，
/// 结构稳定（`<rate>` 无属性、无嵌套同名标签），简单字符串定位足够且省依赖。
fn extract_rate(xml: &str) -> Result<f64, IpcError> {
    let text = extract_tag_text(xml, "rate")
        .ok_or_else(|| IpcError::Protocol("status.xml 中没有 <rate> 标签".into()))?;
    text.trim()
        .parse::<f64>()
        .map_err(|e| IpcError::Protocol(format!("<rate> 的值不是数字：{e}")))
}

fn extract_tag_text<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(&xml[start..end])
}

/// RFC 4648 标准 base64（仅编码），用于拼 Basic Auth 头（"用户名:密码"，用户名为空）。
/// 只差这一个功能就引 base64 crate 不符合「依赖精简」，20 行手写 + RFC 测试向量更划算。
fn base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(TABLE[(n >> 18 & 63) as usize] as char);
        out.push(TABLE[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// 精简版 VLC 3.x status.xml 样例（真实结构的子集，字段顺序与缩进一致）
    const STATUS_XML_SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8" standalone="yes" ?>
<root>
  <fullscreen>false</fullscreen>
  <apiversion>3</apiversion>
  <currentplid>4</currentplid>
  <time>120</time>
  <volume>256</volume>
  <length>3600</length>
  <rate>2.5</rate>
  <state>playing</state>
</root>"#;

    #[test]
    fn status_url_without_command() {
        let client = VlcHttpClient::new(8080, "pw");
        assert_eq!(
            client.status_url(None),
            "http://127.0.0.1:8080/requests/status.xml"
        );
    }

    #[test]
    fn status_url_with_rate_command() {
        let client = VlcHttpClient::new(9090, "pw");
        assert_eq!(
            client.status_url(Some(&rate_query(5.0))),
            "http://127.0.0.1:9090/requests/status.xml?command=rate&val=5"
        );
        assert_eq!(
            client.status_url(Some(&rate_query(1.25))),
            "http://127.0.0.1:9090/requests/status.xml?command=rate&val=1.25"
        );
        assert_eq!(
            client.status_url(Some("command=pl_pause")),
            "http://127.0.0.1:9090/requests/status.xml?command=pl_pause"
        );
    }

    #[test]
    fn extract_rate_from_sample() {
        assert_eq!(extract_rate(STATUS_XML_SAMPLE).unwrap(), 2.5);
        // VLC 在 1.0 倍速时输出整数形式 <rate>1</rate>
        assert_eq!(extract_rate("<root><rate>1</rate></root>").unwrap(), 1.0);
    }

    #[test]
    fn extract_rate_missing_tag_is_protocol_error() {
        assert!(matches!(
            extract_rate("<root><state>playing</state></root>"),
            Err(IpcError::Protocol(_))
        ));
    }

    #[test]
    fn extract_rate_non_numeric_is_protocol_error() {
        assert!(matches!(
            extract_rate("<root><rate>fast</rate></root>"),
            Err(IpcError::Protocol(_))
        ));
    }

    /// RFC 4648 测试向量 + Basic Auth 实际拼法（用户名为空 → ":密码"）
    #[test]
    fn base64_rfc4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b":secret"), "OnNlY3JldA==");
    }

    /// 一次性 HTTP 测试服务器：收完请求头 → 回写 canned 响应 → 关闭。
    /// 返回（端口，收到的原始请求文本接收端），用于离线验证请求构造与错误归类。
    fn spawn_one_shot_server(response: String) -> (u16, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1:0");
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
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
            let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
            let _ = stream.write_all(response.as_bytes());
        });
        (port, rx)
    }

    fn http_200(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
    }

    fn http_401() -> String {
        "HTTP/1.1 401 Unauthorized\r\nWww-Authenticate: Basic realm=\"VLC stream\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_string()
    }

    /// 200 应答：get_rate 正常解析，且请求里带空用户名的 Basic Auth 头与正确路径
    #[test]
    fn get_rate_via_fake_server() {
        let (port, rx) = spawn_one_shot_server(http_200(STATUS_XML_SAMPLE));
        let client = VlcHttpClient::new(port, "secret");
        assert_eq!(client.get_rate().unwrap(), 2.5);

        let request = rx.recv().expect("应收到请求");
        assert!(request.starts_with("GET /requests/status.xml HTTP/1.1"));
        assert!(request.contains("Authorization: Basic OnNlY3JldA=="));
    }

    /// 401 → AuthFailed（密码错误是独立的可诊断状态，不与「未运行」混淆）
    #[test]
    fn wrong_password_maps_to_auth_failed() {
        let (port, _rx) = spawn_one_shot_server(http_401());
        let client = VlcHttpClient::new(port, "wrong");
        assert!(matches!(client.get_rate(), Err(IpcError::AuthFailed)));
    }

    /// 401 时接口在监听 → is_available 仍为 true（语义见方法注释）
    #[test]
    fn available_when_listening_even_if_auth_fails() {
        let (port, _rx) = spawn_one_shot_server(http_401());
        let client = VlcHttpClient::new(port, "wrong");
        assert!(client.is_available());
    }

    /// 端口拒绝连接（VLC 未运行/未开 Web 界面）→ Unavailable / is_available = false。
    /// 先 bind 拿一个空闲端口再立刻释放，保证该端口大概率无人监听。
    #[test]
    fn refused_port_maps_to_unavailable() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        }; // listener 在此释放，端口回到无人监听状态

        let client = VlcHttpClient::new(port, "pw");
        assert!(!client.is_available());
        assert!(matches!(client.get_rate(), Err(IpcError::Unavailable)));
        assert!(matches!(client.set_rate(2.0), Err(IpcError::Unavailable)));
        assert!(matches!(client.play_pause(), Err(IpcError::Unavailable)));
    }
}
