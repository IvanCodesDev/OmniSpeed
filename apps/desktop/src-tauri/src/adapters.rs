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

use crate::rules::{AppRule, IpcKind, KeyBindings, RuleMethod};
use crate::state::CurrentTarget;
use player_ipc::{mpc_hc, potplayer, MpvClient, VlcHttpClient, WM_COMMAND, WM_USER};

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
    /// 模拟播放器自身快捷键：需要目标窗口前台（开发文档 §7.3 兜底通道）
    Keys { hwnd: isize, keys: KeyBindings },
}

/// 按规则与当前目标构建通道列表（method=auto 时 IPC/消息在前、按键殿后）
pub fn adapters_for(rule: &AppRule, target: &CurrentTarget) -> Vec<Adapter> {
    let mut list = Vec::new();
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
            IpcKind::None => {}
        }
    }
    if want_keys {
        if let Some(keys) = rule.keys.clone() {
            list.push(Adapter::Keys { hwnd: target.hwnd, keys });
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
    /// 无副作用回读真实倍速（mpv / VLC / PotPlayer 支持）
    pub fn read_rate(&self) -> Option<f64> {
        match self {
            Adapter::Mpv(c) => c.get_speed().ok(),
            Adapter::Vlc(c) => c.get_rate().ok(),
            Adapter::PotPlayer { hwnd } => pot_read_speed(*hwnd),
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
            Adapter::MpcHc { .. } | Adapter::Keys { .. } => {
                Err("该通道仅支持步进，不支持设置精确倍速".into())
            }
        }
    }

    /// 按目标播放器自己的档位步进一档（dir: +1 / -1）
    pub fn step(&self, dir: i32) -> Result<(), String> {
        match self {
            // 可精确设值的通道不走播放器档位：上层直接用 set_rate
            Adapter::Mpv(_) | Adapter::Vlc(_) | Adapter::PotPlayer { .. } => {
                Err("该通道请使用 set_rate".into())
            }
            Adapter::MpcHc { hwnd } => {
                if !window_alive(*hwnd) {
                    return Err("MPC-HC 窗口不存在".into());
                }
                let cmd = if dir > 0 { mpc_hc::ID_PLAY_INCRATE } else { mpc_hc::ID_PLAY_DECRATE };
                platform_win::send_message(*hwnd, WM_COMMAND, cmd, 0);
                Ok(())
            }
            Adapter::Keys { hwnd, keys } => {
                let key = if dir > 0 { &keys.up } else { &keys.down };
                send_key_to(*hwnd, key)
            }
        }
    }

    pub fn reset(&self) -> Result<ReadBack, String> {
        match self {
            Adapter::Mpv(_) | Adapter::Vlc(_) | Adapter::PotPlayer { .. } => self.set_rate(1.0),
            Adapter::MpcHc { hwnd } => {
                if !window_alive(*hwnd) {
                    return Err("MPC-HC 窗口不存在".into());
                }
                platform_win::send_message(*hwnd, WM_COMMAND, mpc_hc::ID_PLAY_RESETRATE, 0);
                Ok(None)
            }
            Adapter::Keys { hwnd, keys } => {
                send_key_to(*hwnd, &keys.reset.clone())?;
                Ok(None)
            }
        }
    }

    pub fn play_pause(&self) -> Result<(), String> {
        match self {
            Adapter::Mpv(c) => c.play_pause().map_err(|e| e.to_string()),
            Adapter::Vlc(c) => c.play_pause().map_err(|e| e.to_string()),
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
    (raw >= potplayer::POT_SPEED_MIN && raw <= potplayer::POT_SPEED_MAX)
        .then(|| potplayer::speed_from_lresult(raw))
}

fn window_alive(hwnd: isize) -> bool {
    hwnd != 0 && platform_win::window_pid(hwnd) != 0
}

/// 模拟按键：目标窗口不在前台时先激活（开发文档 §7.3），再发送其自身快捷键
fn send_key_to(hwnd: isize, key: &str) -> Result<(), String> {
    let combo = platform_win::parse_key(key).ok_or_else(|| format!("无法识别的按键 {key}"))?;
    if !platform_win::is_foreground(hwnd) && !platform_win::bring_to_foreground(hwnd) {
        return Err("无法激活目标窗口，模拟按键需要其在前台".into());
    }
    platform_win::send_key_combo(combo).map_err(|e| e.to_string())
}
