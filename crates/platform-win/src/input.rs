//! 模拟按键（开发文档 §7.3 播放器控制通道·模拟按键兜底）。
//!
//! 把规则里配置的按键字符串（如 "]"、"Ctrl+Up"）解析为组合键，再用 SendInput 发送
//! **目标播放器自身**的调速快捷键。注意这与 OmniSpeed 全局热键（hotkey 模块，§7.2）
//! 方向相反：那边是"收用户的键"，这边是"给播放器发键"。按键通道要求目标窗口在前台，
//! 必要时先经 [`crate::bring_to_foreground`] 激活。

use windows::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, VkKeyScanW, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, VIRTUAL_KEY,
    VK_APPS, VK_BACK, VK_CONTROL, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_HOME,
    VK_INSERT, VK_LEFT, VK_LWIN, VK_MENU, VK_NEXT, VK_NUMLOCK, VK_PRIOR, VK_RCONTROL, VK_RETURN,
    VK_RIGHT, VK_RMENU, VK_RWIN, VK_SHIFT, VK_SNAPSHOT, VK_SPACE, VK_TAB, VK_UP,
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

/// 用 SendInput 发送组合键：按下修饰键 → 主键按下/抬起 → 逆序释放修饰键。
/// 整个序列一次性批量提交，期间不会被用户的真实输入插队。
pub fn send_key_combo(combo: KeyCombo) -> Result<(), Error> {
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

    let mut inputs: Vec<INPUT> = Vec::with_capacity(order.len() * 2);
    inputs.extend(order.iter().map(|&vk| key_input(vk, false)));
    inputs.extend(order.iter().rev().map(|&vk| key_input(vk, true)));

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
        VK_F12, VK_F5, VK_OEM_4, VK_OEM_6, VK_OEM_PERIOD, VK_OEM_PLUS,
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
}
