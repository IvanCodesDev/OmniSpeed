//! 适配器层（开发文档 §5.1 / §7.3）：把「设速/步进/播放暂停」翻译成
//! 具体播放器的控制通道调用。通道优先级：IPC / 控制消息（无需焦点，
//! mpv/VLC/PotPlayer 可设精确值且可回读）→ 模拟按键（需要前台焦点，最后兜底）。
//!
//! 各通道能力（player-ipc 调研结论）：
//! | 通道 | 精确设速 | 回读 | 上限 |
//! | --- | --- | --- | --- |
//! | mpv JSON-IPC | ✅ | ✅ | 远超 16× |
//! | VLC HTTP | ✅ | ✅ | ~16× |
//! | PotPlayer WM_USER SDK | ✅（lParam=倍速×1000） | ✅ | 12×（SDK 区间） |
//! | MPC-HC WM_COMMAND | ✅（14 档绝对倍速码，就近取档） | ❌（终值由档位本身确定） | 8× |
//! | MPC-BE WM_COMMAND | ❌ 仅步进（无绝对倍速码） | ❌ | 步长随设置 |
//! | 模拟按键 | ❌ 仅步进（有 [`KeyRateGrid`] 时可精确） | ❌ | 随播放器 |

use crate::rules::{AppRule, IpcKind, KeyBindings, KeyPlan, KeyRateGrid, RuleMethod};
use crate::state::CurrentTarget;
use player_ipc::{mpc_hc, potplayer, CdpClient, MpvClient, VlcHttpClient, WM_COMMAND, WM_USER};

/// CDP 接管的默认调试端口（内置规则均显式带 port，这里兜底自定义规则漏配）
pub const CDP_DEFAULT_PORT: u16 = 9333;

/// 单次控制成功后的回读值（仅可回读通道为 Some）
pub type ReadBack = Option<f64>;

/// 当前接管对象的可用控制通道（按优先级排列）
pub enum Adapter {
    Mpv(MpvClient),
    Vlc(VlcHttpClient),
    /// PotPlayer 官方 WM_USER SDK：一步精确设速 + 回读，无需前台
    PotPlayer { hwnd: isize },
    /// MPC 系（HC / BE）的 WM_COMMAND 命令码，无需前台。
    /// `ladder` 有值时该版本支持绝对倍速命令，可一条消息落到确定档位；
    /// 为空（MPC-BE、原版 1.7.13）则只能 INC/DECRATE 步进
    MpcHc { hwnd: isize, ladder: Option<Vec<f64>> },
    /// 浏览器扩展（Native Messaging）：精确设速，真实状态经扩展异步上报（开发文档 §3 非对称控制）
    Browser { process: String },
    /// Chromium 套壳客户端（B 站桌面端等）的 CDP 调试口：精确设速 + 回读，
    /// 需先「接管」（带 --remote-debugging-port 启动，见 commands::takeover_client）
    Cdp(CdpClient),
    /// 模拟播放器自身快捷键：需要目标窗口前台（开发文档 §7.3 兜底通道）。
    /// `grid` 有值时该播放器的快捷键含绝对档位，可拼出精确倍速（见 [`KeyRateGrid`]）
    Keys { hwnd: isize, keys: KeyBindings, grid: Option<KeyRateGrid> },
}

/// 按规则与当前目标构建通道列表（method=auto 时 IPC/消息在前、按键殿后）
pub fn adapters_for(rule: &AppRule, target: &CurrentTarget) -> Vec<Adapter> {
    let mut list = Vec::new();
    if rule.method == RuleMethod::Extension {
        list.push(Adapter::Browser { process: target.process_name.clone() });
        return list;
    }
    let want_ipc = matches!(rule.method, RuleMethod::Auto | RuleMethod::Ipc);
    let want_keys = matches!(rule.method, RuleMethod::Auto | RuleMethod::Hotkey);

    if want_ipc {
        match rule.ipc {
            IpcKind::MpvIpc => {
                if let Some(pipe) = rule.ipc_config.as_ref().and_then(|c| c.pipe.clone()) {
                    list.push(Adapter::Mpv(MpvClient::new(pipe)));
                }
            }
            IpcKind::VlcHttp => {
                if let Some(cfg) = rule.ipc_config.as_ref() {
                    if let Some(port) = cfg.port {
                        list.push(Adapter::Vlc(VlcHttpClient::new(
                            port,
                            cfg.password.clone().unwrap_or_default(),
                        )));
                    }
                }
            }
            IpcKind::WmCommand => match rule.id.as_str() {
                "potplayer" => list.push(Adapter::PotPlayer {
                    hwnd: resolve_hwnd(
                        &[potplayer::WINDOW_CLASS_64, potplayer::WINDOW_CLASS_32],
                        target,
                    ),
                }),
                "mpc-hc" => list.push(Adapter::MpcHc {
                    hwnd: resolve_hwnd(&[mpc_hc::WINDOW_CLASS], target),
                    ladder: rule.rate_ladder.clone(),
                }),
                "mpc-be" => list.push(Adapter::MpcHc {
                    hwnd: resolve_hwnd(&[mpc_hc::WINDOW_CLASS_MPC_BE], target),
                    ladder: rule.rate_ladder.clone(),
                }),
                _ => {}
            },
            IpcKind::Cdp => {
                let port = rule
                    .ipc_config
                    .as_ref()
                    .and_then(|c| c.port)
                    .unwrap_or(CDP_DEFAULT_PORT);
                list.push(Adapter::Cdp(CdpClient::new(port)));
            }
            IpcKind::None => {}
        }
    }
    if want_keys {
        if let Some(keys) = rule.keys.clone() {
            list.push(Adapter::Keys { hwnd: target.hwnd, keys, grid: rule.key_rate.clone() });
        }
    }
    list
}

/// 控制消息的目标窗口：优先按类名现查（前台记录的 hwnd 可能因窗口重建而失效），
/// 找不到再回退监听器记录的句柄
fn resolve_hwnd(classes: &[&str], target: &CurrentTarget) -> isize {
    classes
        .iter()
        .find_map(|c| platform_win::find_window(Some(c), None))
        .unwrap_or(target.hwnd)
}

impl Adapter {
    /// 无副作用回读真实倍速（mpv / VLC / PotPlayer 支持；
    /// 浏览器的真实状态由扩展异步上报，router 直接读 Core.browser_media）
    pub fn read_rate(&self) -> Option<f64> {
        match self {
            Adapter::Mpv(c) => c.get_speed().ok(),
            Adapter::Vlc(c) => c.get_rate().ok(),
            Adapter::PotPlayer { hwnd } => pot_read_speed(*hwnd),
            Adapter::Cdp(c) => c.get_rate().ok(),
            _ => None,
        }
    }

    /// 设为精确倍速；不支持精确值的通道返回 Err，由上层降级为步进
    pub fn set_rate(&self, rate: f64) -> Result<ReadBack, String> {
        match self {
            Adapter::Mpv(c) => {
                c.set_speed(rate).map_err(|e| e.to_string())?;
                Ok(c.get_speed().ok())
            }
            Adapter::Vlc(c) => {
                c.set_rate(rate).map_err(|e| e.to_string())?;
                Ok(c.get_rate().ok())
            }
            Adapter::PotPlayer { hwnd } => {
                if !window_alive(*hwnd) {
                    return Err("PotPlayer 窗口不存在".into());
                }
                // SDK 区间 0.2×–12×，speed_to_lparam 已收敛；实际生效值以回读为准
                platform_win::send_message(
                    *hwnd,
                    WM_USER,
                    potplayer::POT_SET_SPEED,
                    potplayer::speed_to_lparam(rate),
                );
                match pot_read_speed(*hwnd) {
                    Some(real) => Ok(Some(real)),
                    None => Err("PotPlayer 未响应速度回读".into()),
                }
            }
            // 下发即返回；真实生效值由扩展经 media 帧异步上报（nm_bridge 负责回写与广播）
            Adapter::Browser { process } => {
                crate::nm_bridge::send_set_rate(process, rate).map(|_| None)
            }
            // 未接管（调试口不在线）→ Unavailable 错误，由应用页「接管」引导开通
            Adapter::Cdp(c) => c.set_rate(rate).map_err(|e| e.to_string()),
            // 绝对档位 + 步进：先按档位键把倍速钉死，再补足差额。终值由计划本身决定，
            // 不依赖播放器原状态，所以这个「算出来的确切值」就是合法回读值
            Adapter::Keys { hwnd, keys, grid: Some(grid) } => {
                let plan = grid.plan(keys, rate).ok_or("该播放器的按键档位拼不出目标倍速")?;
                send_key_plan(*hwnd, &plan)?;
                Ok(Some(plan.rate))
            }
            // 绝对倍速命令：一条 WM_COMMAND 直接落到该档，与播放器原状态无关。
            // 因此这个「就近取到的档位值」本身就是确切的生效值，可以当回读值申报——
            // MPC-HC 没有任何回读通道，不这么算 OSD 就只能报开环估算
            Adapter::MpcHc { hwnd, ladder: Some(ladder) } => {
                if !window_alive(*hwnd) {
                    return Err("MPC-HC 窗口不存在".into());
                }
                let exact = crate::rules::snap_to_ladder(ladder, rate)
                    .ok_or("倍速非法，取不到 MPC-HC 档位")?;
                let cmd = mpc_hc::command_for(exact).ok_or("该档位没有对应的绝对倍速命令")?;
                platform_win::send_message(*hwnd, WM_COMMAND, cmd, 0);
                Ok(Some(exact))
            }
            Adapter::MpcHc { .. } | Adapter::Keys { .. } => {
                Err("该通道仅支持步进，不支持设置精确倍速".into())
            }
        }
    }

    /// 按目标播放器自己的档位步进一档（dir: +1 / -1）
    pub fn step(&self, dir: i32) -> Result<(), String> {
        match self {
            // 可精确设值的通道不走播放器档位：上层直接用 set_rate
            Adapter::Mpv(_) | Adapter::Vlc(_) | Adapter::PotPlayer { .. }
            | Adapter::Browser { .. } | Adapter::Cdp(_) => Err("该通道请使用 set_rate".into()),
            Adapter::MpcHc { hwnd, .. } => {
                if !window_alive(*hwnd) {
                    return Err("MPC-HC 窗口不存在".into());
                }
                let cmd = if dir > 0 { mpc_hc::ID_PLAY_INCRATE } else { mpc_hc::ID_PLAY_DECRATE };
                platform_win::send_message(*hwnd, WM_COMMAND, cmd, 0);
                Ok(())
            }
            Adapter::Keys { hwnd, keys, .. } => {
                let key = if dir > 0 { &keys.up } else { &keys.down };
                send_key_to(*hwnd, key)
            }
        }
    }

    pub fn reset(&self) -> Result<ReadBack, String> {
        match self {
            Adapter::Mpv(_) | Adapter::Vlc(_) | Adapter::PotPlayer { .. }
            | Adapter::Browser { .. } | Adapter::Cdp(_) => self.set_rate(1.0),
            // 1× 通常正是某个档位键，一键到位且终值确定，比 reset 键多一个回读值
            Adapter::Keys { grid: Some(_), .. } => self.set_rate(1.0),
            // RESETRATE 是唯一一条在 MPC 全系（HC / BE / 原版）上都必然把倍速钉到 1.0 的命令，
            // 比绝对倍速码适用面更广，所以恢复走它；终值确定，可以申报 1.0 作回读值
            Adapter::MpcHc { hwnd, .. } => {
                if !window_alive(*hwnd) {
                    return Err("MPC-HC 窗口不存在".into());
                }
                platform_win::send_message(*hwnd, WM_COMMAND, mpc_hc::ID_PLAY_RESETRATE, 0);
                Ok(Some(1.0))
            }
            Adapter::Keys { hwnd, keys, .. } => {
                send_key_to(*hwnd, &keys.reset.clone())?;
                Ok(None)
            }
        }
    }

    pub fn play_pause(&self) -> Result<(), String> {
        match self {
            Adapter::Mpv(c) => c.play_pause().map_err(|e| e.to_string()),
            Adapter::Vlc(c) => c.play_pause().map_err(|e| e.to_string()),
            Adapter::Browser { process } => crate::nm_bridge::send_play_pause(process),
            Adapter::Cdp(c) => c.play_pause().map_err(|e| e.to_string()),
            Adapter::PotPlayer { hwnd } => {
                if !window_alive(*hwnd) {
                    return Err("PotPlayer 窗口不存在".into());
                }
                platform_win::send_message(
                    *hwnd,
                    WM_USER,
                    potplayer::POT_SET_PLAY_STATUS,
                    potplayer::POT_PLAY_STATUS_TOGGLE,
                );
                Ok(())
            }
            Adapter::MpcHc { hwnd, .. } => {
                if !window_alive(*hwnd) {
                    return Err("MPC-HC 窗口不存在".into());
                }
                platform_win::send_message(*hwnd, WM_COMMAND, mpc_hc::ID_PLAY_PLAYPAUSE, 0);
                Ok(())
            }
            // 播放/暂停兜底用空格：绝大多数播放器的默认键
            Adapter::Keys { hwnd, .. } => send_key_to(*hwnd, "Space"),
        }
    }
}

/// PotPlayer 速度回读：0 通常意味着窗口未处理该消息（如 hwnd 已失效）
fn pot_read_speed(hwnd: isize) -> Option<f64> {
    let raw = platform_win::send_message(hwnd, WM_USER, potplayer::POT_GET_SPEED, 0);
    (potplayer::POT_SPEED_MIN..=potplayer::POT_SPEED_MAX)
        .contains(&raw)
        .then(|| potplayer::speed_from_lresult(raw))
}

fn window_alive(hwnd: isize) -> bool {
    hwnd != 0 && platform_win::window_pid(hwnd) != 0
}

/// 按计划连发：档位键把倍速钉到确定值，随后逐次步进补足差额。
/// 中途某一键失败即整体报错——半截序列会把播放器停在一个我们并不知道的倍速上，
/// 与其假装成功，不如让上层降级重试。
///
/// 间隔取自计划（[`KeyRateGrid::step_gap_ms`]）：贴着发会丢步进，而丢掉的那一格
/// 在无回读的按键通道上永远补不回来，所以宁可整串多花几百毫秒。
fn send_key_plan(hwnd: isize, plan: &KeyPlan) -> Result<(), String> {
    let gap = std::time::Duration::from_millis(plan.gap_ms);
    send_key_to(hwnd, &plan.anchor)?;
    for _ in 0..plan.steps {
        std::thread::sleep(gap);
        send_key_to(hwnd, &plan.step_key)?;
    }
    Ok(())
}

/// 模拟按键：目标窗口不在前台时先激活（开发文档 §7.3），再发送其自身快捷键
fn send_key_to(hwnd: isize, key: &str) -> Result<(), String> {
    let combo = platform_win::parse_key(key).ok_or_else(|| format!("无法识别的按键 {key}"))?;
    if !platform_win::is_foreground(hwnd) && !platform_win::bring_to_foreground(hwnd) {
        return Err("无法激活目标窗口，模拟按键需要其在前台".into());
    }
    platform_win::send_key_combo(combo).map_err(|e| e.to_string())
}

/// 百度网盘按键通道的真机核对（M4.8）。默认 `#[ignore]`，需要人工备好现场后单独跑：
///
/// ```text
/// # 1. 起一个带调试口的播放器（不碰用户已登录的网盘主窗口）
/// & "D:\BaiduNetdisk\module\BrowserEngine\BaiduNetdiskUnite.exe" `
///     --remote-debugging-port=9555 --mode=video_player --video-path=<某个本地 mp4>
/// # 2. 点一下播放器窗口让它在前台（见测试里的前台断言），再跑核对
/// cargo test -- --ignored --nocapture baidu
/// ```
///
/// 为什么非得真机：这条通道**没有回读**，倍速对不对全靠「按下去的键确实被收到」。
/// 单测能证明计划算得对，证明不了播放器真会照做——40ms 连发丢步进那次就是单测全绿、
/// 真机差一格。这里走的是与生产完全相同的 [`adapters_for`] → [`Adapter::set_rate`]
/// → [`send_key_plan`]，回读则绕到播放器自己的 Vue 组件上取 `currSpeed`，
/// 与我们下发的值相互独立，能真正判对错。
#[cfg(test)]
mod real_device {
    use super::*;
    use crate::rules::built_in_rules;
    use crate::state::CurrentTarget;

    /// 播放器窗口（`--mode=video_player` 起出来的那个）；网盘主窗口同类名，靠标题区分
    const PLAYER_WINDOW_TITLE: &str = "视频播放";
    const PLAYER_WINDOW_CLASS: &str = "Chrome_WidgetWin_1";
    const DEBUG_PORT: u16 = 9555;

    /// 从播放器自己的 `videoCtrlRight` 组件取当前倍速——这是它渲染倍速按钮用的那个值，
    /// 不是我们的一厢情愿
    fn player_speed(cdp: &CdpClient) -> f64 {
        let expr = r"(() => {
            let hit = null; const seen = new Set();
            const walk = (vm, depth) => {
                if (!vm || depth > 12 || seen.has(vm) || hit) return;
                seen.add(vm);
                if ((vm.$options && vm.$options.name) === 'videoCtrlRight') { hit = vm; return; }
                for (const c of vm.$children || []) walk(c, depth + 1);
            };
            for (const el of document.querySelectorAll('*')) if (el.__vue__) walk(el.__vue__.$root, 0);
            return hit ? hit.currSpeed : null;
        })()";
        cdp.eval_in_player(expr)
            .expect("回读失败：播放器没带 --remote-debugging-port=9555 起？")
            .as_f64()
            .expect("播放器里没找到 videoCtrlRight 组件（视频没加载完？）")
    }

    #[test]
    #[ignore = "真机：需先起一个带调试口的百度网盘播放器，并让其窗口在前台"]
    fn baidu_keys_land_on_the_exact_rate() {
        let cdp = CdpClient::new(DEBUG_PORT);
        assert!(cdp.is_available(), "调试口 {DEBUG_PORT} 不在线，先备好现场");

        let hwnd = platform_win::find_window(Some(PLAYER_WINDOW_CLASS), Some(PLAYER_WINDOW_TITLE))
            .expect("没找到播放器窗口");
        // 播放器必须已在前台。生产里这是天然成立的（watcher 只报前台窗口），但 cargo test
        // 是个后台进程、从没收到过用户输入，Windows 不会把抢前台的权限给它——
        // 连 AttachThreadInput 也救不回来，只会失败在一句含糊的「无法激活目标窗口」上。
        // 所以这里提前拦一道，把话说清楚。
        assert_eq!(
            platform_win::foreground_info().map(|i| i.hwnd),
            Some(hwnd),
            "请先点一下播放器窗口让它在前台，再跑本测试"
        );
        let rule = built_in_rules()
            .into_iter()
            .find(|r| r.id == "baidu-netdisk")
            .expect("内置规则应含百度网盘");
        let target = CurrentTarget {
            rule_id: rule.id.clone(),
            hwnd,
            process_name: rule.process.clone(),
        };
        let adapters = adapters_for(&rule, &target);
        let keys = adapters
            .iter()
            .find(|a| matches!(a, Adapter::Keys { grid: Some(_), .. }))
            .expect("百度网盘应当解析出带档位网格的按键通道");

        // 覆盖三类走法：纯档位（0 步）、向上补格、向下退格（后两者正是会丢步进的那段）
        for (target_rate, want) in [(3.0, 3.0), (2.5, 2.5), (1.8, 1.8), (4.7, 4.7), (1.0, 1.0)] {
            let read_back = keys
                .set_rate(target_rate)
                .unwrap_or_else(|e| panic!("{target_rate}× 下发失败：{e}"));
            assert_eq!(read_back, Some(want), "{target_rate}× 的计划回读值不对");

            // 末键之后播放器还要把倍速经 IPC 落到原生解码器，留一拍再取值
            std::thread::sleep(std::time::Duration::from_millis(700));
            let real = player_speed(&cdp);
            assert!(
                (real - want).abs() < 1e-6,
                "{target_rate}× 下发后播放器实际是 {real}×，差了 {} 格",
                ((real - want) / 0.1).round()
            );
        }
    }

    /// 极简 HTTP GET（播放器本地回读口专用）：这两个回读通道必须独立于我们
    /// 自己的 player-ipc 客户端，否则「下发」与「验收」共用一套代码，验不出谎报
    fn http_get(port: u16, path: &str, basic_auth: Option<&str>) -> String {
        use std::io::{Read, Write};
        let mut s = std::net::TcpStream::connect(("127.0.0.1", port))
            .unwrap_or_else(|e| panic!("连不上 127.0.0.1:{port}（{e}），现场没备好？"));
        s.set_read_timeout(Some(std::time::Duration::from_secs(4))).unwrap();
        let auth = basic_auth.map(|a| format!("Authorization: Basic {a}\r\n")).unwrap_or_default();
        // HTTP/1.0：响应即连接关闭，读到 EOF 就是读完，不用解析 Content-Length/分块
        write!(s, "GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\n{auth}\r\n").unwrap();
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).unwrap_or_else(|e| panic!("读 {path} 失败：{e}"));
        String::from_utf8_lossy(&buf).into_owned()
    }

    /// 从 MPC-HC `/variables.html` 里取 `<p id="...">值</p>`
    fn mpc_variable(html: &str, id: &str) -> String {
        let pat = format!("id=\"{id}\">");
        let start = html
            .find(&pat)
            .map(|i| i + pat.len())
            .unwrap_or_else(|| panic!("回读页里没有 {id} 字段"));
        html[start..].chars().take_while(|c| *c != '<').collect()
    }

    /// MPC-HC 绝对倍速命令的生产链路核对（M4.9）。默认 `#[ignore]`，现场：
    ///
    /// ```text
    /// # 起 MPC-HC 并带上 Web 回读口（无需前台，命令走 WM_COMMAND）
    /// & "C:\Program Files\MPC-HC\mpc-hc64.exe" /webport 13579 <某个较长的媒体文件>
    /// cargo test -- --ignored --nocapture mpc_hc_absolute
    /// ```
    ///
    /// 协议层的 14 档逐条扫描已在 2026-08-31 做过（.tmp-mpc-probe.ps1，13/14 实测吻合，
    /// 8× 是播放器渲染链上限、自报 8.0）。这里补的是**生产代码**那一段：
    /// [`adapters_for`] 按类名现查窗口 → [`Adapter::set_rate`] 就近取档 → 查命令码 →
    /// platform-win `send_message`，回读走播放器自己的 Web 口，与下发路径完全独立。
    #[test]
    #[ignore = "真机：需先起 MPC-HC（/webport 13579）并载入一个媒体文件"]
    fn mpc_hc_absolute_commands_land_on_the_exact_rung() {
        const PORT: u16 = 13579;
        // 停→播：把可能已播完暂停的现场拉回「正在播放」，也顺便把进度倒回开头
        http_get(PORT, "/command.html?wm_command=890", None);
        std::thread::sleep(std::time::Duration::from_millis(300));
        http_get(PORT, "/command.html?wm_command=887", None);
        std::thread::sleep(std::time::Duration::from_millis(800));
        let vars = http_get(PORT, "/variables.html", None);
        assert_eq!(mpc_variable(&vars, "state"), "2", "MPC-HC 应处于播放中（state=2）");

        let rule = built_in_rules()
            .into_iter()
            .find(|r| r.id == "mpc-hc")
            .expect("内置规则应含 MPC-HC");
        // hwnd 给 0：逼着 resolve_hwnd 走「按类名现查」，这正是生产里窗口重建后的路径
        let target = CurrentTarget {
            rule_id: rule.id.clone(),
            hwnd: 0,
            process_name: rule.process.clone(),
        };
        let adapters = adapters_for(&rule, &target);
        let mpc = adapters
            .iter()
            .find(|a| matches!(a, Adapter::MpcHc { ladder: Some(_), .. }))
            .expect("MPC-HC 应解析出带档位表的控制消息通道");

        // 档内值原样落档 + 档间值就近取档：2.4 距 2.0 比距 3.0 近，必须吸到 2.0
        for (target_rate, want) in [(2.0, 2.0), (2.4, 2.0), (1.1, 1.1), (6.0, 6.0), (0.25, 0.25)] {
            let declared = mpc
                .set_rate(target_rate)
                .unwrap_or_else(|e| panic!("{target_rate}× 下发失败：{e}"));
            assert_eq!(declared, Some(want), "{target_rate}× 的申报档位不对");

            std::thread::sleep(std::time::Duration::from_millis(200));
            let vars = http_get(PORT, "/variables.html", None);
            let real: f64 = mpc_variable(&vars, "playbackrate").parse().expect("倍速应是数字");
            assert!(
                (real - want).abs() < 1e-3,
                "{target_rate}× 下发后播放器自报 {real}×，应为 {want}×"
            );
        }

        // reset 走 RESETRATE（MPC 全系通用），申报值与真实值都必须钉在 1.0
        let declared = mpc.reset().expect("reset 失败");
        assert_eq!(declared, Some(1.0));
        std::thread::sleep(std::time::Duration::from_millis(200));
        let vars = http_get(PORT, "/variables.html", None);
        let real: f64 = mpc_variable(&vars, "playbackrate").parse().unwrap();
        assert!((real - 1.0).abs() < 1e-3, "reset 后播放器自报 {real}×");
    }

    /// 从 VLC `status.xml` 里取 `<rate>` 值
    fn vlc_rate(port: u16) -> f64 {
        // ":omnispeed" 的 Base64（VLC HTTP 口用户名恒为空）
        let xml = http_get(port, "/requests/status.xml", Some("Om9tbmlzcGVlZA=="));
        let start = xml.find("<rate>").map(|i| i + "<rate>".len()).expect("status.xml 里没有 rate");
        let end = xml[start..].find("</rate>").unwrap() + start;
        xml[start..end].trim().parse().expect("rate 应是数字")
    }

    /// VLC 按键网格的生产链路核对（M4.9）。默认 `#[ignore]`，现场：
    ///
    /// ```text
    /// # 1. 起 VLC：HTTP 回读口 + 循环播放一个较长的媒体文件
    /// & "C:\Program Files\VideoLAN\VLC\vlc.exe" --extraintf http --http-host 127.0.0.1 `
    ///     --http-port 8080 --http-password omnispeed --repeat <媒体文件>
    /// # 2. 预编译测试二进制（cargo test --no-run），再把 VLC 点到前台后立即跑：
    /// cargo test -- --ignored --nocapture vlc_key_grid
    /// ```
    ///
    /// 与百度那条一样：按键通道要求目标前台，而 cargo test 是后台进程抢不到前台，
    /// 所以前台得提前备好，测试开头会拦一道。协议层网格（=/]/[ 步进、0ms 连发不丢键、
    /// 边界 2.1/0.3 可达）已由 .tmp-vlc-probe.ps1 实测；这里走生产的
    /// [`adapters_for`] → `Keys{grid}` → [`KeyRateGrid::plan`] → `send_key_plan`，
    /// 回读走 VLC 自己的 HTTP 口（它读的是 input 层真实速率，与按键写的 playlist 层
    /// 分属两层，恰好构成独立验收）。
    #[test]
    #[ignore = "真机：需先起 VLC（HTTP 口 + 播放中）并把它点到前台"]
    fn vlc_key_grid_lands_on_the_exact_rate() {
        const PORT: u16 = 8080;
        let fg = platform_win::foreground_info().expect("桌面应有前台窗口");
        assert_eq!(fg.process_name, "vlc.exe", "请先把 VLC 点到前台再跑本测试（当前前台：{}）", fg.title);

        let rule = built_in_rules().into_iter().find(|r| r.id == "vlc").expect("内置规则应含 VLC");
        let target = CurrentTarget {
            rule_id: rule.id.clone(),
            hwnd: fg.hwnd,
            process_name: rule.process.clone(),
        };
        let adapters = adapters_for(&rule, &target);
        let keys = adapters
            .iter()
            .find(|a| matches!(a, Adapter::Keys { grid: Some(_), .. }))
            .expect("VLC 应解析出带倍速网格的按键通道");

        // 纯锚点、向上/向下补格、非网格值吸附（1.37→1.4）、两侧边界（2.1 / 0.3）
        for (target_rate, want) in
            [(2.0, 2.0), (1.37, 1.4), (0.5, 0.5), (2.1, 2.1), (0.3, 0.3), (1.0, 1.0)]
        {
            let declared = keys
                .set_rate(target_rate)
                .unwrap_or_else(|e| panic!("{target_rate}× 下发失败：{e}"));
            assert_eq!(declared, Some(want), "{target_rate}× 的计划回读值不对");

            std::thread::sleep(std::time::Duration::from_millis(300));
            let real = vlc_rate(PORT);
            // VLC 内部以 1000/rate 的整数存倍速，读回带 ≤0.2% 量化误差（实测 1.5→1.5015），
            // 网格吸附已把它收敛住，这里按 0.01 验收即可
            assert!(
                (real - want).abs() < 0.01,
                "{target_rate}× 下发后 VLC 实际是 {real}×，应为 {want}×"
            );
        }
    }
}
