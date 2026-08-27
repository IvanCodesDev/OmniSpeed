//! 手动验证 ForegroundWatcher（开发文档 §7.1）：监听 5 秒并打印前台窗口变化。
//!
//! 运行：`cargo run --example watch_foreground`，期间切换任意窗口即可看到输出
//! （本进程自身的窗口被 WINEVENT_SKIPOWNPROCESS 过滤，不会出现）。

use std::time::{Duration, Instant};

fn main() {
    let t0 = Instant::now();
    let watcher = platform_win::ForegroundWatcher::start(move |info| {
        println!(
            "[{:>6.3}s] foreground: pid={} process={} hwnd=0x{:X} title={:?}",
            t0.elapsed().as_secs_f32(),
            info.pid,
            info.process_name,
            info.hwnd,
            info.title,
        );
    })
    .expect("ForegroundWatcher 启动失败");

    println!("watching foreground changes for 5s ...");
    std::thread::sleep(Duration::from_secs(5));
    drop(watcher);
    println!("watcher stopped, bye");
}
