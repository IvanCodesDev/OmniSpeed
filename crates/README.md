# crates

项目内可复用的 Rust 库（开发文档 §5.1 / §10）：

| crate | 职责 | 说明 |
| --- | --- | --- |
| `platform-win` | 所有 unsafe Win32 封装集中于此 | 前台窗口监听（SetWinEventHook 事件驱动）、进程识别、SendInput 模拟按键、窗口消息发送 |
| `player-ipc` | 播放器控制通道客户端（纯 IO，无 Win32 依赖） | mpv JSON-IPC（命名管道）、VLC HTTP 接口，以及 PotPlayer / MPC-HC 窗口消息码标定结论 |

两个 crate 均可在各自目录内独立 `cargo check` / `cargo test`，由桌面应用（`apps/desktop/src-tauri`）以路径依赖引用。
