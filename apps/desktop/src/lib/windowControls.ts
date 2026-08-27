import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** 浏览器预览时窗口控制按钮为空操作，仅在 Tauri 环境生效 */
export async function minimizeWindow() {
  if (isTauri()) await getCurrentWindow().minimize();
}

export async function toggleMaximizeWindow() {
  if (isTauri()) await getCurrentWindow().toggleMaximize();
}

export async function closeWindow() {
  if (isTauri()) await getCurrentWindow().close();
}
