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
//! | MPC-HC WM_COMMAND | ❌ 仅步进 | ❌ | 步长随版本/设置 |
//! | 模拟按键 | ❌ 仅步进 | ❌ | 随播放器 |

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
    /// MPC-HC WM_COMMAND 命令码：仅档位步进，无需前台
    MpcHc { hwnd: isize },
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
            Adapter::MpcHc { hwnd } => {
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
            Adapter::MpcHc { hwnd } => {
                if !window_alive(*hwnd) {
                    return Err("MPC-HC 窗口不存在".into());
                }
                platform_win::send_message(*hwnd, WM_COMMAND, mpc_hc::ID_PLAY_RESETRATE, 0);
                Ok(None)
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
            Adapter::MpcHc { hwnd } => {
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
}
