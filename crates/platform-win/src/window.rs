//! 窗口查询与消息通道：前台窗口信息、窗口查找、SendMessage/PostMessage 封装。
//!
//! 对应开发文档 §7.1（HWND → pid → 进程名的识别链路，供适配器注册表匹配规则）与
//! §7.3（PotPlayer/MPC-HC 的 WM_COMMAND / WM_USER 控制消息、模拟按键兜底所需的
//! 前台判定/置前台）。

use std::collections::HashMap;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::core::{BOOL, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowW, GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostMessageW, SendMessageW,
    SetForegroundWindow, ShowWindow, SW_RESTORE,
};

use crate::Error;

/// 前台窗口信息（§7.1：hwnd → pid → 进程名，交给适配器注册表匹配内置/用户规则）
#[derive(Debug, Clone)]
pub struct ForegroundInfo {
    pub hwnd: isize,
    pub pid: u32,
    /// 小写进程 exe 名，如 "vlc.exe"（查不到路径时为空字符串，由上层规则忽略）
    pub process_name: String,
    pub exe_path: Option<std::path::PathBuf>,
    pub title: String,
}

/// isize → HWND 的内部统一转换点（对外 API 一律用 isize 表示句柄，隔离 windows 类型）
pub(crate) fn to_hwnd(hwnd: isize) -> HWND {
    HWND(hwnd as *mut core::ffi::c_void)
}

/// 查询当前前台窗口
pub fn foreground_info() -> Option<ForegroundInfo> {
    // SAFETY: 只读查询；桌面暂无前台窗口时返回空句柄，由 info_from_hwnd 过滤
    let hwnd = unsafe { GetForegroundWindow() };
    info_from_hwnd(hwnd)
}

/// HWND → [`ForegroundInfo`]（watcher 的事件回调与 foreground_info 复用同一条识别链路）
pub(crate) fn info_from_hwnd(hwnd: HWND) -> Option<ForegroundInfo> {
    if hwnd.is_invalid() {
        return None;
    }
    let pid = raw_window_pid(hwnd);
    if pid == 0 {
        return None;
    }
    let exe_path = process_path_of(pid);
    // 进程名从完整路径提取；拿不到（受保护进程等）则置空，不因此丢弃整条信息——标题仍有展示价值
    let process_name = exe_path
        .as_deref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    Some(ForegroundInfo {
        hwnd: hwnd.0 as isize,
        pid,
        process_name,
        exe_path,
        title: window_text(hwnd),
    })
}

/// hwnd → 进程 pid（hwnd 无效时返回 0）
pub fn window_pid(hwnd: isize) -> u32 {
    raw_window_pid(to_hwnd(hwnd))
}

fn raw_window_pid(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    // SAFETY: pid 指针在调用期间有效；hwnd 无效时函数返回 0 且不写 pid
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid
}

/// pid → 小写进程 exe 名（如 "vlc.exe"）
pub fn process_name_of(pid: u32) -> Option<String> {
    let path = process_path_of(pid)?;
    Some(path.file_name()?.to_string_lossy().to_lowercase())
}

/// pid → exe 完整路径。
/// 用 PROCESS_QUERY_LIMITED_INFORMATION：权限需求最低，对提权进程通常也拿得到（§7.6 最小权限原则）。
fn process_path_of(pid: u32) -> Option<PathBuf> {
    /// OpenProcess 句柄的 RAII 守卫，任何提前返回都不泄漏句柄
    struct Guard(HANDLE);
    impl Drop for Guard {
        fn drop(&mut self) {
            // SAFETY: 句柄由 OpenProcess 返回且仅关闭一次
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    // SAFETY: 缓冲区与长度指针在调用期间有效
    unsafe {
        let guard = Guard(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?);
        // 32767 是 NT 路径上限，一次取够，省去 ERROR_INSUFFICIENT_BUFFER 重试循环
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        QueryFullProcessImageNameW(
            guard.0,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .ok()?;
        Some(PathBuf::from(OsString::from_wide(&buf[..len as usize])))
    }
}

fn window_text(hwnd: HWND) -> String {
    // SAFETY: hwnd 无效时两个调用都安全地返回 0
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buf);
        String::from_utf16_lossy(&buf[..copied.max(0) as usize])
    }
}

/// UTF-16 + NUL 结尾，Win32 宽字符串参数用
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 查找窗口（类名/标题任一可选）；返回 hwnd。
/// 两者都不给时直接返回 None——FindWindowW(NULL, NULL) 会匹配任意窗口，没有意义。
pub fn find_window(class: Option<&str>, title: Option<&str>) -> Option<isize> {
    if class.is_none() && title.is_none() {
        return None;
    }
    let class_w = class.map(to_wide);
    let title_w = title.map(to_wide);
    // SAFETY: 宽字符串缓冲由 class_w/title_w 持有，调用期间存活
    let hwnd = unsafe {
        FindWindowW(
            class_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
            title_w
                .as_ref()
                .map_or(PCWSTR::null(), |v| PCWSTR(v.as_ptr())),
        )
    }
    .ok()?;
    (!hwnd.is_invalid()).then_some(hwnd.0 as isize)
}

/// 枚举查找：返回第一个属于指定进程名（小写 exe 名）的顶层可见窗口。
/// 场景：§7.3 播放器控制通道——先定位播放器主窗口，再走控制消息或模拟按键。
pub fn find_window_by_process(process_name: &str) -> Option<isize> {
    struct Search {
        target: String,
        found: Option<isize>,
        /// 同一进程往往有多个顶层窗口，缓存 pid → 进程名避免重复 OpenProcess
        names: HashMap<u32, Option<String>>,
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: lparam 是 EnumWindows 调用栈上 Search 的裸指针，回调只在该调用内同步执行
        let search = &mut *(lparam.0 as *mut Search);
        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        let pid = raw_window_pid(hwnd);
        if pid == 0 {
            return true.into();
        }
        let name = search
            .names
            .entry(pid)
            .or_insert_with(|| process_name_of(pid));
        if name.as_deref() == Some(search.target.as_str()) {
            search.found = Some(hwnd.0 as isize);
            return false.into(); // 找到即中断枚举
        }
        true.into()
    }

    let mut search = Search {
        target: process_name.to_lowercase(),
        found: None,
        names: HashMap::new(),
    };
    // 回调返回 FALSE 中断枚举时 EnumWindows 本身报"失败"，因此忽略返回值、只看 found
    let _ = unsafe { EnumWindows(Some(enum_proc), LPARAM(&mut search as *mut Search as isize)) };
    search.found
}

/// SendMessageW 封装（同步等待窗口过程返回）。
/// §7.3：PotPlayer/MPC-HC 的 WM_COMMAND / WM_USER 控制消息不要求前台焦点，是按键兜底之前的首选通道。
pub fn send_message(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize {
    // SAFETY: 目标窗口无效时 SendMessageW 返回 0，无未定义行为
    unsafe {
        SendMessageW(
            to_hwnd(hwnd),
            msg,
            Some(WPARAM(wparam)),
            Some(LPARAM(lparam)),
        )
        .0
    }
}

/// PostMessageW 封装（投递后立即返回，不等待目标处理）
pub fn post_message(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> Result<(), Error> {
    // SAFETY: 同 send_message
    unsafe { PostMessageW(Some(to_hwnd(hwnd)), msg, WPARAM(wparam), LPARAM(lparam)) }.map_err(
        |source| Error::Win32 {
            api: "PostMessageW",
            source,
        },
    )
}

/// 目标窗口当前是否前台
pub fn is_foreground(hwnd: isize) -> bool {
    // SAFETY: 只读查询
    unsafe { GetForegroundWindow() == to_hwnd(hwnd) }
}

/// 尝试把目标窗口置前台（§7.3：模拟按键兜底要求目标窗口持有焦点）。
///
/// Windows 有前台锁保护：非前台进程直接调 SetForegroundWindow 常被拒绝。
/// 失败后用经典绕行——把本线程与当前前台线程的输入队列临时挂接（AttachThreadInput）
/// 再试一次。返回最终是否置前成功。
pub fn bring_to_foreground(hwnd: isize) -> bool {
    let target = to_hwnd(hwnd);
    // SAFETY: 各调用对无效 hwnd 都以失败返回，无未定义行为
    unsafe {
        if IsIconic(target).as_bool() {
            // 最小化状态即便置前成功也收不到按键，先还原
            let _ = ShowWindow(target, SW_RESTORE);
        }
        if SetForegroundWindow(target).as_bool() {
            return true;
        }
        let fg = GetForegroundWindow();
        let fg_thread = if fg.is_invalid() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let cur = GetCurrentThreadId();
        if fg_thread == 0 || fg_thread == cur {
            return GetForegroundWindow() == target;
        }
        let attached = AttachThreadInput(cur, fg_thread, true).as_bool();
        let ok = SetForegroundWindow(target).as_bool();
        if attached {
            let _ = AttachThreadInput(cur, fg_thread, false);
        }
        ok || GetForegroundWindow() == target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 查询自身进程：OpenProcess + QueryFullProcessImageNameW 全链路应成功且统一小写
    #[test]
    fn process_name_of_self() {
        let name = process_name_of(std::process::id()).expect("查询自身进程名应成功");
        assert!(name.ends_with(".exe"), "进程名应含 .exe 后缀：{name}");
        assert_eq!(name, name.to_lowercase(), "进程名应统一小写");
    }

    /// 只做冒烟：测试宿主环境可能没有前台窗口（None 合法），有则字段应自洽
    #[test]
    fn foreground_info_smoke() {
        if let Some(info) = foreground_info() {
            assert_ne!(info.pid, 0);
            assert_eq!(info.process_name, info.process_name.to_lowercase());
        }
    }
}
