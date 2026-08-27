# player-ipc

播放器控制通道客户端（[开发文档 §7.3](../../docs/开发文档.md)）：Tier 2 桌面播放器「IPC 优先、按键兜底」策略中 **IPC 优先** 的一半。

- **mpv** — JSON-IPC（Windows 命名管道）：`MpvClient`，可一步设精确倍速、可回读。
- **VLC** — HTTP 接口（status.xml）：`VlcHttpClient`，可一步设精确倍速、可回读。
- **PotPlayer / MPC-HC** — 窗口消息：只交付**消息码常量表 + 换算函数 + 本文档**。

## 架构约束：纯 IO，无 Win32 依赖

`FindWindow` / `SendMessage` 属 unsafe Win32，按开发文档 §5.1 集中在 platform-win；本 crate 保持纯 IO（命名管道 / HTTP / 常量表），任意平台可编译，全部逻辑离线单元测试。

统一错误 `IpcError`：`Unavailable`（未运行/接口未开，`method="auto"` 时触发按键兜底）、`AuthFailed`（VLC 密码问题，应用页显示「需要设置」）、`Protocol`（通道可达但本次操作失败）。

## mpv（`ipc.kind = "mpv-ipc"`）

mpv 需以 `--input-ipc-server=\\.\pipe\mpvsocket` 启动。协议为 JSON Lines；应答流里会混入事件行（含 `"event"` 字段），客户端已按 request_id 匹配并跳过事件。每次操作短连接（打开→写→读→关），并以行数上限防协议异常卡死（命名管道无 std 读超时可用；mpv 对每条命令立即应答）。

```rust
let mpv = MpvClient::new(r"\\.\pipe\mpvsocket");
if mpv.is_available() { mpv.set_speed(5.0)?; }
```

## VLC（`ipc.kind = "vlc-http"`）

VLC 需开启 Web 界面（工具→首选项→显示全部→界面→主界面 勾选 Web）并**设置密码**，默认端口 8080。Basic Auth 用户名为空。`set_rate` 走 `status.xml?command=rate&val=X`（浮点），`get_rate` 解析 `<rate>`。注意 `is_available` 对 401 返回 `true`（接口在监听，密码问题单独诊断）。

```rust
let vlc = VlcHttpClient::new(8080, "password");
vlc.set_rate(5.0)?;
```

## PotPlayer 消息码标定结论（Spike 交付）

主窗口类名：`PotPlayer64`（64 位）/ `PotPlayer`（32 位）。两条通道：

### 通道一：WM_USER 官方 SDK（推荐）

`SendMessage(hwnd, WM_USER /*0x0400*/, POT_*, lParam)`，返回值即查询结果。

| 常量 | 值 | 含义 |
| --- | --- | --- |
| `POT_GET_PLAY_STATUS` | 0x5006 | 0=停止（旧版资料记 -1，存疑）、1=暂停、2=播放 |
| `POT_SET_PLAY_STATUS` | 0x5007 | lParam：0=切换、1=暂停、2=播放 |
| `POT_GET_SPEED` | 0x5015 | 返回倍速×1000（200–12000） |
| `POT_SET_SPEED` | 0x5016 | **一步精确设速**：lParam=倍速×1000（200–12000，即 0.2×–12×） |

来源：PotPlayer 官方论坛「팟플레이어 실험실」置顶 [PotPlayer SDK（2023-08-29 更新）](https://m.cafe.daum.net/pot-tool/N88T/6)，并被 AutoHotkey 社区库、ld3l/PotPlayerControl 等项目长期使用。**置信度：高。**

关键结论：PotPlayer 有精确设速消息（`POT_SET_SPEED`），应优先于步进命令；但 **IPC 上限 12×**，低于产品全局 16×（§7.8），capabilities 按 12× 申报。换算用 `potplayer::speed_to_lparam` / `speed_from_lresult`。

### 通道二：WM_COMMAND 热键命令码（兜底）

`SendMessage(hwnd, WM_COMMAND /*0x0111*/, CMD_*, 0)`，等价按下热键，只能步进。

| 常量 | 值 | 含义 |
| --- | --- | --- |
| `CMD_PLAY_PAUSE` | 10014 | 播放/暂停切换 |
| `CMD_SPEED_NORMAL` | 10246 | 恢复 1× |
| `CMD_SPEED_DOWN` | 10247 | 减速约 0.1× |
| `CMD_SPEED_UP` | 10248 | 加速约 0.1× |

来源：社区提取的命令表（[ld3l/PotPlayerControl raw.md](https://github.com/ld3l/PotPlayerControl/blob/main/raw.md)、[AHK PotPlayer x64 Function Library](https://www.autohotkey.com/boards/viewtopic.php?t=45385)）。与官方 SDK 重叠条目数值吻合（20487=0x5007、24624=0x6030），交叉验证通过。**置信度：中**；验证方法：实机逐条 SendMessage 观察 OSD（M2 回归矩阵，§12）。

## MPC-HC 消息码标定结论

主窗口类名：`MediaPlayerClassicW`。`SendMessage(hwnd, WM_COMMAND, ID_*, 0)`，无需前台。

| 常量 | 值 | 含义 |
| --- | --- | --- |
| `ID_PLAY_PLAY` / `ID_PLAY_PAUSE` | 887 / 888 | 播放 / 暂停 |
| `ID_PLAY_PLAYPAUSE` | 889 | 播放/暂停切换 |
| `ID_PLAY_STOP` | 890 | 停止 |
| `ID_PLAY_DECRATE` / `ID_PLAY_INCRATE` | 894 / 895 | 减速 / 加速一档 |
| `ID_PLAY_RESETRATE` | 896 | 恢复 1× |

来源：官方源码 [clsid2/mpc-hc `src/mpc-hc/resource.h`](https://github.com/clsid2/mpc-hc/blob/develop/src/mpc-hc/resource.h)（原版数值一致），并与程序内「选项→播放器→键」ID 列、Web 界面 `/command.html?wm_command=` 一致。**置信度：高。**

能力边界：MPC-HC 无「一步精确设速」公开消息，只能步进 + 归一逼近（步长随版本/设置而变），状态以估计值申报（§3、§14）。

**MPC-BE 注意**：类名为 `MPC-BE`（常量 `WINDOW_CLASS_MPC_BE`），命令码与 MPC-HC 不完全一致，894–896 未在 BE 侧验证；M2 仅内置 MPC-HC 规则，BE 需另行标定（查 aleksoid1978/MPC-BE resource.h 或实机验证）。

## 测试

```
cargo check && cargo test   # 在本目录执行；全部离线（VLC 用本地一次性 HTTP 假服务器）
```

覆盖：mpv 命令序列化与应答解析（success/error/事件混入/行数上限）、VLC URL 构造与 `<rate>` 提取、Basic Auth（空用户名）、401→AuthFailed、连接拒绝→Unavailable、管道不存在→`is_available()==false`、PotPlayer 参数换算。真机连通性（mpv/VLC/PotPlayer/MPC-HC 回归矩阵）属 M2 集成测试（§12）。
