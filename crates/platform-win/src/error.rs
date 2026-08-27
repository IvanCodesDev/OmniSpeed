//! 统一错误类型：本 crate 对外的所有可失败 API 均返回 [`Error`]（thiserror，§4.4 技术栈约定）。

/// platform-win 的错误类型
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// [`crate::ForegroundWatcher`] 仅支持单实例（WINEVENTPROC 回调经全局槽转发，见 watcher 模块头）
    #[error("前台监听器已在运行：ForegroundWatcher 仅支持单实例")]
    WatcherAlreadyRunning,

    /// 监听线程创建或握手失败（理论上仅 OOM 等极端情况）
    #[error("前台监听线程启动失败：{0}")]
    WatcherStart(String),

    /// SetWinEventHook 返回空句柄
    #[error("SetWinEventHook 注册失败：{0}")]
    HookInstall(#[source] windows::core::Error),

    /// SendInput 未能注入全部键盘事件（常见原因：目标进程完整性级别更高，被 UIPI 拦截）
    #[error("SendInput 仅注入 {sent}/{expected} 个键盘事件")]
    SendInput { expected: u32, sent: u32 },

    /// 其余 Win32 调用失败
    #[error("{api} 调用失败：{source}")]
    Win32 {
        api: &'static str,
        #[source]
        source: windows::core::Error,
    },
}
