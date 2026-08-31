//! 模拟按键（开发文档 §7.3 播放器控制通道·模拟按键兜底）。
//!
//! 把规则里配置的按键字符串（如 "]"、"Ctrl+Up"）解析为组合键，再用 SendInput 发送
//! **目标播放器自身**的调速快捷键。注意这与 OmniSpeed 全局热键（hotkey 模块，§7.2）
//! 方向相反：那边是"收用户的键"，这边是"给播放器发键"。按键通道要求目标窗口在前台，
//! 必要时先经 [`crate::bring_to_foreground`] 激活。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD,
    KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC,
    VIRTUAL_KEY, VK_APPS, VK_BACK, VK_CONTROL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE,
    VK_F1, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_NEXT, VK_NUMLOCK, VK_PRIOR, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_SHIFT, VK_SNAPSHOT, VK_SPACE, VK_TAB, VK_UP,
};

use crate::Error;

/// 按键组合（发给目标播放器自身的快捷键）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyCombo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Win32 虚拟键码（VK_*）
    pub vk: u16,
}

/// 解析规则里的按键字符串 → [`KeyCombo`]，无法识别返回 None。
///
/// 支持（大小写不敏感）：
/// - 可打印字符："]"、"["、"="、"." 等——经 VkKeyScanW 按当前键盘布局解析，
///   其高字节的 shift/ctrl/alt 状态会并入组合（如 "+" 在美式布局 = Shift + VK_OEM_PLUS）；
/// - 命名键："Up"/"Down"/"Left"/"Right"/"Space"/"Enter"/"Esc"/"Tab" 等与 "F1".."F24"；
/// - 单个字母/数字：按物理键位解析，"j" 与 "J" 等价、不附加 Shift；
/// - 修饰前缀："Ctrl+Up"、"Shift+."、"Ctrl+Alt+R"（Win 键不支持，播放器快捷键用不到）。
pub fn parse_key(s: &str) -> Option<KeyCombo> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split('+').collect();
    // 主键本身是 '+' 时（"+"、"Ctrl++"）split 会产生两个尾部空片段，在此还原
    let (main, modifiers) = if parts.len() >= 2
        && parts[parts.len() - 1].is_empty()
        && parts[parts.len() - 2].is_empty()
    {
        ("+", &parts[..parts.len() - 2])
    } else {
        (parts[parts.len() - 1], &parts[..parts.len() - 1])
    };

    let mut combo = KeyCombo {
        ctrl: false,
        alt: false,
        shift: false,
        vk: 0,
    };
    for m in modifiers {
        match m.trim().to_ascii_lowercase().as_str() {
            "ctrl" | "control" => combo.ctrl = true,
            "alt" => combo.alt = true,
            "shift" => combo.shift = true,
            _ => return None,
        }
    }

    let (vk, shift, ctrl, alt) = resolve_main_key(main.trim())?;
    combo.vk = vk;
    combo.shift |= shift;
    combo.ctrl |= ctrl;
    combo.alt |= alt;
    Some(combo)
}

/// 主键 → (vk, shift, ctrl, alt)。后三者来自 VkKeyScanW 的上档状态（仅可打印字符路径）
fn resolve_main_key(main: &str) -> Option<(u16, bool, bool, bool)> {
    if main.is_empty() {
        return None;
    }
    let lower = main.to_ascii_lowercase();
    let named = match lower.as_str() {
        "up" => Some(VK_UP),
        "down" => Some(VK_DOWN),
        "left" => Some(VK_LEFT),
        "right" => Some(VK_RIGHT),
        "space" => Some(VK_SPACE),
        "enter" | "return" => Some(VK_RETURN),
        "tab" => Some(VK_TAB),
        "esc" | "escape" => Some(VK_ESCAPE),
        "backspace" => Some(VK_BACK),
        "home" => Some(VK_HOME),
        "end" => Some(VK_END),
        "pageup" => Some(VK_PRIOR),
        "pagedown" => Some(VK_NEXT),
        "insert" => Some(VK_INSERT),
        "delete" => Some(VK_DELETE),
        _ => None,
    };
    if let Some(vk) = named {
        return Some((vk.0, false, false, false));
    }
    // F1..F24（VK 连续分布，从 VK_F1 偏移）
    if let Some(num) = lower.strip_prefix('f') {
        if let Ok(n) = num.parse::<u16>() {
            if (1..=24).contains(&n) {
                return Some((VK_F1.0 + n - 1, false, false, false));
            }
        }
        // "f99"、"foo" 等以 f 开头的多字符串到此为无效；单字符 "f" 落到下面的字母分支
        if main.chars().count() > 1 {
            return None;
        }
    }
    // 单字符
    let mut chars = main.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if c.is_ascii_alphanumeric() {
        // 字母/数字按物理键位：VK 值即大写 ASCII（"j"/"J" 都是 0x4A），不附加 Shift——
        // 规则里的 "Z" 指 Z 键本身而非 Shift+Z
        return Some((c.to_ascii_uppercase() as u16, false, false, false));
    }
    // 其余可打印字符交给 VkKeyScanW（依赖当前键盘布局；美式/简中默认布局下
    // "]"→VK_OEM_6、"="→VK_OEM_PLUS）。返回值低字节 = VK，高字节 = 上档状态：
    // 1=Shift、2=Ctrl、4=Alt——必须并入组合，否则 "?"、"+" 这类上档字符发出去就变成了
    // "/"、"="（§7.3 要求发出播放器实际配置的那个键）
    let unit = u16::try_from(u32::from(c)).ok()?; // BMP 之外 VkKeyScanW 无法表达
                                                  // SAFETY: 纯查表调用，无前置条件
    let ret = unsafe { VkKeyScanW(unit) };
    if ret == -1 {
        return None;
    }
    let vk = (ret & 0xFF) as u16;
    let state = (ret >> 8) & 0xFF;
    if vk == 0 || vk == 0xFF {
        return None;
    }
    Some((vk, state & 1 != 0, state & 2 != 0, state & 4 != 0))
}

/// 发送前要临时抬起的物理修饰键，左右分开列出——只放开真正按着的那一侧，
/// 免得把没按的那侧也“按回去”。顺序即抬起顺序：Ctrl 垫在最后（见 [`build_sequence`]）。
const NEUTRALIZE_ORDER: [VIRTUAL_KEY; 8] = [
    VK_LSHIFT,
    VK_RSHIFT,
    VK_LWIN,
    VK_RWIN,
    VK_LMENU,
    VK_RMENU,
    VK_LCONTROL,
    VK_RCONTROL,
];

/// 此刻物理按着的修饰键（GetAsyncKeyState 高位 = 按下）
fn held_modifiers() -> Vec<u16> {
    NEUTRALIZE_ORDER
        .iter()
        // SAFETY: 纯状态查询，无前置条件
        .filter(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
        .map(|vk| vk.0)
        .collect()
}

/// 组装一次发送的完整键序 `(vk, 是否抬起)`：中和物理修饰键 → 目标组合键 → 复原物理修饰键。
///
/// 为什么要中和：热键是 Ctrl+Alt+↑ 这类带修饰键的组合，触发的瞬间用户十有八九还按着
/// Ctrl+Alt，此时直接发播放器的 `.`／`,` 会被系统合成 `Ctrl+Alt+.` 递给播放器，播放器
/// 不认这个组合就整个丢掉——真机上表现为 OSD 跳到目标值、画面倍速纹丝不动。
///
/// 为什么发完还要按回去：用户按住 Ctrl+Alt 连点 ↑ 是常规用法，修饰键若停在抬起状态，
/// 后续几下 RegisterHotKey 就不再匹配，连点会从第二下开始失灵。
fn build_sequence(held: &[u16], combo: KeyCombo) -> Vec<(u16, bool)> {
    let mut seq: Vec<(u16, bool)> = Vec::with_capacity(held.len() * 2 + 10);

    // Alt／Win 单独抬起会被系统读成「呼出窗口菜单／开始菜单」；而 Ctrl 按着时抬 Alt 只发
    // WM_KEYUP（见 WM_SYSKEYUP 文档），Win 同理。held 里本来就有 Ctrl 时天然满足——
    // NEUTRALIZE_ORDER 已把 Ctrl 排在 Alt/Win 之后；没有 Ctrl 才临时垫一个。
    let has_ctrl = held
        .iter()
        .any(|&vk| vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0);
    let needs_mask = !has_ctrl
        && held.iter().any(|&vk| {
            vk == VK_LMENU.0 || vk == VK_RMENU.0 || vk == VK_LWIN.0 || vk == VK_RWIN.0
        });

    if needs_mask {
        seq.push((VK_LCONTROL.0, false));
    }
    seq.extend(held.iter().map(|&vk| (vk, true)));
    if needs_mask {
        seq.push((VK_LCONTROL.0, true));
    }

    let mut order: Vec<u16> = Vec::with_capacity(4);
    if combo.ctrl {
        order.push(VK_CONTROL.0);
    }
    if combo.alt {
        order.push(VK_MENU.0);
    }
    if combo.shift {
        order.push(VK_SHIFT.0);
    }
    order.push(combo.vk);
    seq.extend(order.iter().map(|&vk| (vk, false)));
    seq.extend(order.iter().rev().map(|&vk| (vk, true)));

    seq.extend(held.iter().rev().map(|&vk| (vk, false)));
    seq
}

/// 用 SendInput 发送组合键：中和用户按着的修饰键 → 按下修饰键 → 主键按下/抬起 →
/// 逆序释放修饰键 → 复原用户的修饰键（详见 [`build_sequence`]）。
///
/// 整个序列一次性批量提交，SendInput 保证其间不会被用户的真实输入插队——中和与复原之间
/// 不存在“修饰键半开”的时间窗，用户按住 ↑ 的自动重复也挤不进来。
pub fn send_key_combo(combo: KeyCombo) -> Result<(), Error> {
    let inputs: Vec<INPUT> = build_sequence(&held_modifiers(), combo)
        .into_iter()
        .map(|(vk, key_up)| key_input(vk, key_up))
        .collect();

    // SAFETY: inputs 数组在调用期间有效，cbsize 与元素类型一致
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        // 常见失败：目标进程完整性级别更高（管理员身份运行的播放器），被 UIPI 拦截
        Err(Error::SendInput {
            expected: inputs.len() as u32,
            sent,
        })
    }
}

/// 构造单个键盘事件。方向键等扩展键必须带 KEYEVENTF_EXTENDEDKEY，
/// 否则部分程序会把 VK_UP 当成小键盘 8（NumLock 语义）处理
fn key_input(vk: u16, key_up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    if is_extended_key(vk) {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                // 补齐扫描码：部分程序（低层钩子/游戏引擎）只认扫描码不认 VK
                // SAFETY: 纯查表调用，无前置条件
                wScan: unsafe { MapVirtualKeyW(u32::from(vk), MAPVK_VK_TO_VSC) } as u16,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Win32 扩展键表（KEYEVENTF_EXTENDEDKEY 的适用范围）
fn is_extended_key(vk: u16) -> bool {
    matches!(
        VIRTUAL_KEY(vk),
        VK_UP
            | VK_DOWN
            | VK_LEFT
            | VK_RIGHT
            | VK_HOME
            | VK_END
            | VK_PRIOR
            | VK_NEXT
            | VK_INSERT
            | VK_DELETE
            | VK_DIVIDE
            | VK_NUMLOCK
            | VK_RCONTROL
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
            | VK_APPS
            | VK_SNAPSHOT
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        VK_F12, VK_F5, VK_OEM_4, VK_OEM_6, VK_OEM_COMMA, VK_OEM_PERIOD, VK_OEM_PLUS,
    };

    fn combo(ctrl: bool, alt: bool, shift: bool, vk: u16) -> KeyCombo {
        KeyCombo {
            ctrl,
            alt,
            shift,
            vk,
        }
    }

    // 注意：涉及 VkKeyScanW 的断言依赖美式物理布局的 OEM 键位
    // （简中等 IME 布局同样基于美式键位，与开发/CI 环境一致）

    /// 单个可打印字符：VLC 默认调速键 "]"/"["/"="（§7.3 内置规则）
    #[test]
    fn parses_printable_chars() {
        assert_eq!(parse_key("]"), Some(combo(false, false, false, VK_OEM_6.0)));
        assert_eq!(parse_key("["), Some(combo(false, false, false, VK_OEM_4.0)));
        assert_eq!(
            parse_key("="),
            Some(combo(false, false, false, VK_OEM_PLUS.0))
        );
        // '+' 是 '=' 的上档字符：VkKeyScanW 的 shift 状态必须并入组合
        assert_eq!(
            parse_key("+"),
            Some(combo(false, false, true, VK_OEM_PLUS.0))
        );
    }

    /// 字母/数字按物理键位解析，大小写等价、不附加 Shift
    #[test]
    fn parses_letters_and_digits() {
        assert_eq!(
            parse_key("j"),
            Some(combo(false, false, false, u16::from(b'J')))
        );
        assert_eq!(
            parse_key("J"),
            Some(combo(false, false, false, u16::from(b'J')))
        );
        assert_eq!(
            parse_key("5"),
            Some(combo(false, false, false, u16::from(b'5')))
        );
        assert_eq!(
            parse_key("0"),
            Some(combo(false, false, false, u16::from(b'0')))
        );
    }

    /// 命名键大小写不敏感
    #[test]
    fn parses_named_keys() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_RETURN, VK_SPACE, VK_UP};
        assert_eq!(
            parse_key("Space"),
            Some(combo(false, false, false, VK_SPACE.0))
        );
        assert_eq!(
            parse_key("space"),
            Some(combo(false, false, false, VK_SPACE.0))
        );
        assert_eq!(parse_key("Up"), Some(combo(false, false, false, VK_UP.0)));
        assert_eq!(
            parse_key("Enter"),
            Some(combo(false, false, false, VK_RETURN.0))
        );
    }

    /// F1..F24 从 VK_F1 连续偏移
    #[test]
    fn parses_function_keys() {
        assert_eq!(parse_key("F5"), Some(combo(false, false, false, VK_F5.0)));
        assert_eq!(parse_key("f12"), Some(combo(false, false, false, VK_F12.0)));
        assert_eq!(parse_key("F1"), Some(combo(false, false, false, VK_F1.0)));
    }

    /// 修饰前缀与 VkKeyScanW 上档状态的合并
    #[test]
    fn parses_modifier_prefixes() {
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_UP;
        assert_eq!(
            parse_key("Ctrl+Up"),
            Some(combo(true, false, false, VK_UP.0))
        );
        assert_eq!(
            parse_key("ctrl+alt+z"),
            Some(combo(true, true, false, u16::from(b'Z')))
        );
        // "Shift+."：修饰键来自前缀，主键经 VkKeyScanW
        assert_eq!(
            parse_key("Shift+."),
            Some(combo(false, false, true, VK_OEM_PERIOD.0))
        );
        // "Ctrl++"：主键是字面 '+'（上档），shift 来自 VkKeyScanW
        assert_eq!(
            parse_key("Ctrl++"),
            Some(combo(true, false, true, VK_OEM_PLUS.0))
        );
    }

    /// 非法输入一律返回 None，不 panic
    #[test]
    fn rejects_invalid_input() {
        for bad in [
            "", "   ", "Foo", "F99", "F0", "Ctrl+", "Ctrl", "Win+X", "Meta+Up", "①",
        ] {
            assert_eq!(parse_key(bad), None, "应拒绝的输入：{bad:?}");
        }
    }

    /// 百度网盘的步进键 ","（无修饰键的裸键）
    fn comma() -> KeyCombo {
        combo(false, false, false, VK_OEM_COMMA.0)
    }

    /// 没人按着修饰键时，键序与「按下主键、抬起主键」完全一致，不夹带任何多余事件
    #[test]
    fn sends_plain_combo_when_nothing_held() {
        assert_eq!(
            build_sequence(&[], comma()),
            vec![(VK_OEM_COMMA.0, false), (VK_OEM_COMMA.0, true)]
        );
    }

    /// 真机复现的那一幕：热键 Ctrl+Alt+↓ 触发时用户还按着 Ctrl+Alt，
    /// 步进键必须先把这两个键抬掉再发，发完原样按回去（否则连点第二下起热键就不匹配了）
    #[test]
    fn neutralizes_and_restores_held_ctrl_alt() {
        // held 由 NEUTRALIZE_ORDER 过滤而来，Alt 必定排在 Ctrl 前面
        let held = [VK_LMENU.0, VK_LCONTROL.0];
        assert_eq!(
            build_sequence(&held, comma()),
            vec![
                (VK_LMENU.0, true),
                (VK_LCONTROL.0, true),
                (VK_OEM_COMMA.0, false),
                (VK_OEM_COMMA.0, true),
                (VK_LCONTROL.0, false),
                (VK_LMENU.0, false),
            ]
        );
    }

    /// 组合键自带的修饰键照常发送，中和只针对用户物理按着的那些
    #[test]
    fn keeps_combo_own_modifiers() {
        let seq = build_sequence(&[VK_LCONTROL.0], combo(true, false, false, VK_UP.0));
        assert_eq!(
            seq,
            vec![
                (VK_LCONTROL.0, true),
                (VK_CONTROL.0, false),
                (VK_UP.0, false),
                (VK_UP.0, true),
                (VK_CONTROL.0, true),
                (VK_LCONTROL.0, false),
            ]
        );
    }

    /// 只按着 Alt（或 Win）时，抬起动作要垫一个临时 Ctrl 遮住，
    /// 否则系统会把这次抬起当成「呼出窗口菜单／开始菜单」
    #[test]
    fn masks_lone_alt_or_win_release_with_ctrl() {
        for lone in [VK_LMENU.0, VK_RMENU.0, VK_LWIN.0, VK_RWIN.0] {
            let seq = build_sequence(&[lone], comma());
            assert_eq!(
                &seq[..3],
                &[
                    (VK_LCONTROL.0, false),
                    (lone, true),
                    (VK_LCONTROL.0, true)
                ],
                "vk={lone:#x} 的抬起未被 Ctrl 遮住"
            );
            assert_eq!(seq.last(), Some(&(lone, false)), "vk={lone:#x} 未复原");
        }
    }

    /// 已经按着 Ctrl 就不必再垫：NEUTRALIZE_ORDER 把 Ctrl 排在末位，抬 Alt 时它还按着
    #[test]
    fn skips_mask_when_ctrl_already_held() {
        let seq = build_sequence(&[VK_LMENU.0, VK_RCONTROL.0], comma());
        assert_eq!(seq[0], (VK_LMENU.0, true));
        assert_eq!(seq[1], (VK_RCONTROL.0, true));
    }

    /// Shift 单独抬起没有副作用，不需要垫 Ctrl
    #[test]
    fn skips_mask_for_lone_shift() {
        let seq = build_sequence(&[VK_RSHIFT.0], comma());
        assert!(
            !seq.iter().any(|&(vk, _)| vk == VK_LCONTROL.0),
            "Shift 不该触发 Ctrl 遮罩：{seq:?}"
        );
        assert_eq!(seq[0], (VK_RSHIFT.0, true));
    }
}
