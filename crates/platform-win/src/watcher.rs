//! 前台窗口监听（开发文档 §7.1）：SetWinEventHook(EVENT_SYSTEM_FOREGROUND) 事件驱动，
//! 避免轮询。
//!
//! 钩子装在专用线程上并跑 GetMessageW 消息循环——WINEVENT_OUTOFCONTEXT 钩子的回调
//! 由安装线程的消息循环派发，没有消息循环就永远收不到事件。停止 = 向该线程
//! PostThreadMessageW(WM_QUIT) + join。

use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW, TranslateMessage,
    EVENT_SYSTEM_FOREGROUND, MSG, PM_NOREMOVE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_QUIT, WM_USER,
};

use crate::window::{foreground_info, info_from_hwnd, ForegroundInfo};
use crate::Error;

/// winuser.h 的 OBJID_WINDOW：只处理"窗口本体"的前台事件，忽略子对象/插入符等伴生事件
const OBJID_WINDOW: i32 = 0;

type Callback = Arc<dyn Fn(ForegroundInfo) + Send + Sync + 'static>;

/// 全局回调槽。WINEVENTPROC 是不带用户数据指针的 C 函数指针，只能经 static 转发；
/// 槽位同时充当单实例守卫（Some = 已有监听器在运行）。应用只需要一个监听器（§5.1 monitor）。
static CALLBACK: Mutex<Option<Callback>> = Mutex::new(None);

/// 取回调槽。临界区内只有赋值/克隆，不会 panic；即便极端情况被毒化也直接取回数据
fn callback_slot() -> MutexGuard<'static, Option<Callback>> {
    CALLBACK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 前台窗口监听器。
///
/// **单实例**：受 WINEVENTPROC 无用户数据参数所限，回调经全局槽转发，同一时刻只允许
/// 一个监听器存活；已有实例运行时再次 [`ForegroundWatcher::start`] 返回
/// [`Error::WatcherAlreadyRunning`]。实例 Drop 后可重新启动。
pub struct ForegroundWatcher {
    /// 监听线程 id：Drop 时 PostThreadMessageW(WM_QUIT) 用
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl ForegroundWatcher {
    /// 启动监听。回调在监听线程上执行；启动后立即用当前前台窗口回调一次，
    /// 调用方无需再自行查询初始状态。
    ///
    /// 注意：不要在回调内 drop 本实例——Drop 会 join 监听线程，自己等自己会死锁。
    pub fn start(callback: impl Fn(ForegroundInfo) + Send + Sync + 'static) -> Result<Self, Error> {
        {
            let mut slot = callback_slot();
            if slot.is_some() {
                return Err(Error::WatcherAlreadyRunning);
            }
            *slot = Some(Arc::new(callback));
        }

        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("foreground-watcher".into())
            .spawn(move || watcher_thread(tx));
        let thread = match spawned {
            Ok(t) => t,
            Err(e) => {
                *callback_slot() = None;
                return Err(Error::WatcherStart(e.to_string()));
            }
        };

        // 等监听线程完成「建消息队列 → 装钩子」的握手，把失败暴露给调用方
        match rx.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                *callback_slot() = None;
                Err(e)
            }
            // 线程未握手即退出（理论不可达，防御性兜底）
            Err(_) => {
                let _ = thread.join();
                *callback_slot() = None;
                Err(Error::WatcherStart("监听线程提前退出".into()))
            }
        }
    }
}

impl Drop for ForegroundWatcher {
    /// 反注册钩子并退出线程：WM_QUIT 结束消息循环 → 线程 UnhookWinEvent 后返回 → join
    fn drop(&mut self) {
        // SAFETY: thread_id 来自监听线程握手，其消息队列已在装钩子前显式创建，投递必达
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        match posted {
            Ok(()) => {
                if let Some(t) = self.thread.take() {
                    let _ = t.join();
                }
            }
            // 理论不可达；不 join，避免把调用方挂死
            Err(e) => eprintln!("[platform-win] 停止前台监听失败（线程可能滞留）：{e}"),
        }
        // 线程已退出，释放回调槽，允许之后重新 start
        *callback_slot() = None;
    }
}

/// 监听线程主体：建消息队列 → 装钩子 → 握手 → 初始回调 → 消息循环 → 反注册
fn watcher_thread(handshake: mpsc::Sender<Result<u32, Error>>) {
    // SAFETY: MSG 缓冲在本函数内存活；钩子句柄仅在本线程使用并成对反注册
    unsafe {
        let mut msg = MSG::default();
        // 先用 PeekMessageW 强制创建本线程消息队列，保证 Drop 里的 PostThreadMessageW 必达
        // （线程消息队列是惰性创建的，没有它 WM_QUIT 会投递失败）
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);

        // WINEVENT_OUTOFCONTEXT：回调在本线程异步派发，无需向目标进程注入 DLL；
        // WINEVENT_SKIPOWNPROCESS：OmniSpeed 自己的主面板/OSD 切前台不触发回调，
        // 否则"按热键 → OSD 弹出 → 前台变成自己"会形成干扰循环（§7.1）
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(on_win_event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        );
        if hook.is_invalid() {
            let _ = handshake.send(Err(Error::HookInstall(windows::core::Error::from_win32())));
            return;
        }
        let _ = handshake.send(Ok(GetCurrentThreadId()));
        drop(handshake);

        // 契约：启动后立即用当前前台窗口回调一次。初始快照忠实上报（不套用
        // SKIPOWNPROCESS 过滤），由上层规则决定是否忽略自身进程
        if let Some(info) = foreground_info() {
            invoke(info);
        }

        // 消息循环：out-of-context 钩子的回调由 GetMessageW 内部派发；
        // 返回 0 表示收到 WM_QUIT，-1 表示错误，二者都退出
        loop {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWinEvent(hook);
    }
}

/// WINEVENTPROC：系统在监听线程的消息循环里派发 EVENT_SYSTEM_FOREGROUND 时进入
unsafe extern "system" fn on_win_event(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _time_ms: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || id_object != OBJID_WINDOW {
        return;
    }
    // 事件携带的 hwnd 即新前台窗口；识别失败（窗口已瞬时销毁等）则丢弃本次事件
    if let Some(info) = info_from_hwnd(hwnd) {
        invoke(info);
    }
}

/// 把事件转发给用户回调；先把 Arc 克隆出锁外再调用，回调执行期间不占用全局槽
fn invoke(info: ForegroundInfo) {
    let cb = callback_slot().clone();
    if let Some(cb) = cb {
        cb(info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 单实例约束 + Drop 后可重启（同时覆盖 WM_QUIT 停机与线程 join 路径）
    #[test]
    fn single_instance_and_restart() {
        let w1 = ForegroundWatcher::start(|_| {}).expect("首次启动应成功");
        assert!(matches!(
            ForegroundWatcher::start(|_| {}),
            Err(Error::WatcherAlreadyRunning)
        ));
        drop(w1);
        let w2 = ForegroundWatcher::start(|_| {}).expect("Drop 后应可重新启动");
        drop(w2);
    }
}
