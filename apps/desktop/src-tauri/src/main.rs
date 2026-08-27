// 阻止 Windows release 版本弹出额外的控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    omnispeed_lib::run()
}
