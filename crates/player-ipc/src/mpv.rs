//! mpv JSON-IPC 客户端（开发文档 §7.3：IPC 通道可一步设精确倍速、可回读真实值）。
//!
//! mpv 需以 `--input-ipc-server=\\.\pipe\mpvsocket` 启动（或写入 mpv.conf 的
//! `input-ipc-server`），Windows 上该「socket」就是一条命名管道，客户端用
//! `CreateFile` 语义（即 std 的 `OpenOptions`）读写即可，无需任何 Win32 绑定。
//!
//! 协议（<https://mpv.io/manual/master/#json-ipc>）是 JSON Lines：
//! - 请求：`{"command":[...],"request_id":N}` + `\n`；
//! - 应答：含 `"error"` 字段（`"success"` 才算成功）并回显 `request_id`；
//! - 事件：含 `"event"` 字段，mpv 会向所有客户端广播（如 pause、end-file），
//!   **不请自来地混在应答流里**，解析时必须跳过，直到读到匹配的应答行。
//!
//! 连接模型：每次操作短连接（打开 → 写 → 读 → 关）。mpv 的 IPC 服务支持多客户端
//! 并发连接，短连接省去保活/断线重连的状态机，对 M2 低频的调速命令足够且更稳。

use crate::IpcError;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicU64, Ordering};

/// 放弃前最多读取的行数。命名管道的阻塞式 `read` 没有 std 层超时可用
/// （`set_read_timeout` 只有 TcpStream 提供），因此用行数上限兜底：
/// mpv 对每条命令都会立即应答，事件行只在状态广播时偶发混入，
/// 正常情况下前几行内必有应答；超过上限说明协议异常，放弃而不是卡死。
const MAX_REPLY_LINES: usize = 64;

/// request_id 进程内单调递增，保证应答匹配不受管道里残留行干扰。
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

/// mpv JSON-IPC 客户端（Windows 命名管道，如 `\\.\pipe\mpvsocket`）。
///
/// 对应 `appRules[].ipc = { "kind": "mpv-ipc", "pipe": "\\\\.\\pipe\\mpvsocket" }`
/// （开发文档 §8）。mpv 的 `speed` 属性范围 0.01–100，远超产品全局上限；
/// [0.25, 16] 的产品级 clamp 由 core 统一执行（开发文档 §7.8），本 crate 不重复做。
#[derive(Debug, Clone)]
pub struct MpvClient {
    /// 命名管道路径（`--input-ipc-server` 的值）
    pipe: String,
}

impl MpvClient {
    pub fn new(pipe: impl Into<String>) -> Self {
        Self { pipe: pipe.into() }
    }

    /// 尝试连接判断可用性（不 panic，短超时）。
    ///
    /// mpv 启动时创建管道、退出后管道消失，所以「能打开」即可认定 IPC 就绪，
    /// 不必发命令：open 对本机命名管道要么立即成功、要么立即失败（NotFound 等），
    /// 天然满足「短超时」。管道不存在/被拒 → false（有单元测试覆盖 NotFound 路径）。
    pub fn is_available(&self) -> bool {
        self.open().is_ok()
    }

    /// 一步设置精确倍速：`set_property speed`（§7.3「mpv 可一步设 5×」的落点）。
    pub fn set_speed(&self, speed: f64) -> Result<(), IpcError> {
        self.request(&[json!("set_property"), json!("speed"), json!(speed)])?;
        Ok(())
    }

    /// 回读真实倍速：`get_property speed`。
    ///
    /// Tier 2 里 mpv 是可回读的通道之一——回读后状态权威以播放器为准（开发文档 §3）。
    pub fn get_speed(&self) -> Result<f64, IpcError> {
        let data = self.request(&[json!("get_property"), json!("speed")])?;
        data.as_f64()
            .ok_or_else(|| IpcError::Protocol(format!("speed 属性不是数字：{data}")))
    }

    /// 播放/暂停切换：`cycle pause`。
    pub fn play_pause(&self) -> Result<(), IpcError> {
        self.request(&[json!("cycle"), json!("pause")])?;
        Ok(())
    }

    fn open(&self) -> std::io::Result<std::fs::File> {
        // Windows 命名管道对 CreateFile（即此处的 open）语义透明，read+write 拿到双工句柄。
        // 非 Windows 平台该路径不存在 → Err → Unavailable，crate 仍可跨平台编译测试。
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.pipe)
    }

    /// 短连接执行一条命令并返回应答的 `data` 字段。
    ///
    /// 错误归类约定：打开管道失败 = 播放器未运行/未开 IPC → `Unavailable`；
    /// 打开成功之后的任何失败（写入、读取、解析、mpv 报错）都属于协议层 → `Protocol`。
    fn request(&self, command: &[Value]) -> Result<Value, IpcError> {
        let request_id = REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let mut pipe = self.open().map_err(|_| IpcError::Unavailable)?;
        pipe.write_all(encode_request(command, request_id).as_bytes())
            .map_err(|e| IpcError::Protocol(format!("写入 mpv 请求失败：{e}")))?;
        take_reply(BufReader::new(pipe).lines(), request_id)
    }
}

#[derive(Serialize)]
struct MpvRequest<'a> {
    command: &'a [Value],
    request_id: u64,
}

/// 把命令序列化成一行 JSON（含结尾换行，mpv 按行分帧）。
fn encode_request(command: &[Value], request_id: u64) -> String {
    let mut line = serde_json::to_string(&MpvRequest {
        command,
        request_id,
    })
    .expect("固定结构序列化不会失败");
    line.push('\n');
    line
}

/// 从应答流中找到匹配 `request_id` 的应答行，成功时返回其 `data` 字段（缺省为 Null）。
///
/// 跳过规则（见模块头协议说明）：
/// - 含 `"event"` 字段 → mpv 广播的事件行，跳过；
/// - `request_id` 存在但不匹配 → 他人应答（短连接下理论上不会出现，防御性跳过）；
/// - `"error"` 为 `"success"` → 成功；为其他文本 → mpv 明确报错。
fn take_reply(
    lines: impl Iterator<Item = std::io::Result<String>>,
    request_id: u64,
) -> Result<Value, IpcError> {
    for line in lines.take(MAX_REPLY_LINES) {
        let line = line.map_err(|e| IpcError::Protocol(format!("读取 mpv 应答失败：{e}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|e| IpcError::Protocol(format!("mpv 应答不是合法 JSON：{e}")))?;
        if value.get("event").is_some() {
            continue;
        }
        if let Some(id) = value.get("request_id").and_then(Value::as_u64) {
            if id != request_id {
                continue;
            }
        }
        let Some(error) = value.get("error").and_then(Value::as_str) else {
            continue;
        };
        if error == "success" {
            return Ok(value.get("data").cloned().unwrap_or(Value::Null));
        }
        return Err(IpcError::Protocol(format!("mpv 返回错误：{error}")));
    }
    Err(IpcError::Protocol(
        "未在应答流中读到匹配的 mpv 应答".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(raw: &[&str]) -> impl Iterator<Item = std::io::Result<String>> {
        raw.iter()
            .map(|s| Ok(s.to_string()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// set_property 命令的 JSON 序列化逐字节可控（浮点保留 .0，字段顺序固定）
    #[test]
    fn encode_set_speed_request() {
        let line = encode_request(
            &[json!("set_property"), json!("speed"), json!(5.0)],
            7,
        );
        assert_eq!(
            line,
            "{\"command\":[\"set_property\",\"speed\",5.0],\"request_id\":7}\n"
        );
    }

    #[test]
    fn encode_cycle_pause_request() {
        let line = encode_request(&[json!("cycle"), json!("pause")], 42);
        assert_eq!(line, "{\"command\":[\"cycle\",\"pause\"],\"request_id\":42}\n");
    }

    /// "error":"success" 的应答返回 data；set_property 没有 data 时给 Null
    #[test]
    fn reply_success_without_data() {
        let reply = take_reply(lines(&[r#"{"request_id":7,"error":"success"}"#]), 7);
        assert_eq!(reply.unwrap(), Value::Null);
    }

    #[test]
    fn reply_success_with_data() {
        let reply = take_reply(
            lines(&[r#"{"data":2.5,"request_id":8,"error":"success"}"#]),
            8,
        );
        assert_eq!(reply.unwrap(), json!(2.5));
    }

    /// 应答流里混入事件行（含 "event" 字段）必须跳过（mpv 手册 JSON IPC 一节）
    #[test]
    fn reply_skips_interleaved_events() {
        let reply = take_reply(
            lines(&[
                r#"{"event":"property-change","id":1,"name":"pause","data":false}"#,
                r#"{"event":"seek"}"#,
                r#"{"data":1.0,"request_id":9,"error":"success"}"#,
            ]),
            9,
        );
        assert_eq!(reply.unwrap(), json!(1.0));
    }

    /// request_id 不匹配的应答行跳过，继续等待自己的应答
    #[test]
    fn reply_skips_mismatched_request_id() {
        let reply = take_reply(
            lines(&[
                r#"{"request_id":3,"error":"success","data":99.0}"#,
                r#"{"request_id":4,"error":"success","data":1.25}"#,
            ]),
            4,
        );
        assert_eq!(reply.unwrap(), json!(1.25));
    }

    /// mpv 明确报错（如属性不存在）→ Protocol，并携带 mpv 的错误文本
    #[test]
    fn reply_error_maps_to_protocol() {
        let reply = take_reply(
            lines(&[r#"{"request_id":5,"error":"property not found"}"#]),
            5,
        );
        match reply {
            Err(IpcError::Protocol(msg)) => assert!(msg.contains("property not found")),
            other => panic!("应为 Protocol 错误，实际 {other:?}"),
        }
    }

    /// 非法 JSON 行 → Protocol（协议已错乱，不再继续读）
    #[test]
    fn reply_invalid_json_maps_to_protocol() {
        let reply = take_reply(lines(&["not-json"]), 1);
        assert!(matches!(reply, Err(IpcError::Protocol(_))));
    }

    /// 流结束仍未见应答 → Protocol
    #[test]
    fn reply_empty_stream_maps_to_protocol() {
        let reply = take_reply(lines(&[]), 1);
        assert!(matches!(reply, Err(IpcError::Protocol(_))));
    }

    /// 事件行刷屏超过行数上限 → 放弃（防卡死兜底，见 MAX_REPLY_LINES）
    #[test]
    fn reply_gives_up_after_line_cap() {
        let event_lines = vec![r#"{"event":"tick"}"#; MAX_REPLY_LINES + 8];
        let reply = take_reply(lines(&event_lines), 1);
        assert!(matches!(reply, Err(IpcError::Protocol(_))));
    }

    /// 读取中途 IO 失败（管道断开等）→ Protocol
    #[test]
    fn reply_io_error_maps_to_protocol() {
        let stream = vec![Err(std::io::Error::other("pipe broken"))].into_iter();
        let reply = take_reply(stream, 1);
        assert!(matches!(reply, Err(IpcError::Protocol(_))));
    }

    /// 管道不存在（mpv 未运行）时 is_available = false 且不 panic：
    /// Windows 上打开不存在的 \\.\pipe\ 路径、其他平台打开不存在的文件路径都是 NotFound
    #[test]
    fn is_available_false_when_pipe_missing() {
        let client = MpvClient::new(r"\\.\pipe\omnispeed-test-definitely-missing");
        assert!(!client.is_available());
    }

    /// 管道不存在时具体操作返回 Unavailable（而不是 Protocol / panic）
    #[test]
    fn request_unavailable_when_pipe_missing() {
        let client = MpvClient::new(r"\\.\pipe\omnispeed-test-definitely-missing");
        assert!(matches!(client.get_speed(), Err(IpcError::Unavailable)));
        assert!(matches!(client.set_speed(2.0), Err(IpcError::Unavailable)));
        assert!(matches!(client.play_pause(), Err(IpcError::Unavailable)));
    }
}
