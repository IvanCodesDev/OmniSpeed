// 阻止 Windows release 版本弹出额外的控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 浏览器按 NM 宿主清单拉起本程序时带 --nm-host：只做 stdio ⟷ 主程序管道的中继，
    // 不初始化 GUI（开发文档 §5.3，复用主程序可执行文件）
    if std::env::args().any(|a| a == "--nm-host") {
        omnispeed_lib::nm_host_main();
        return;
    }
    omnispeed_lib::run()
}
