/** M0 阶段的静态占位数据，后续由 Tauri IPC（get_current_media / list_apps 等）替换 */

export type AppStatus = "connected" | "adapted" | "needs-setup";
export type BrandId = "chrome" | "edge" | "vlc" | "potplayer" | "unknown";
export type AppKind = "browser" | "player" | "unknown";
/** 播放器 IPC 通道类型（开发文档 §7.3） */
export type IpcKind = "vlc-http" | "wm-command" | "none";

export interface ManagedApp {
  id: string;
  name: string;
  icon: BrandId;
  process: string;
  status: AppStatus;
  method: string;
  kind: AppKind;
  ipc: IpcKind;
}

export const managedApps: ManagedApp[] = [
  { id: "edge", name: "Microsoft Edge", icon: "edge", process: "msedge.exe", status: "connected", method: "浏览器扩展", kind: "browser", ipc: "none" },
  { id: "chrome", name: "Google Chrome", icon: "chrome", process: "chrome.exe", status: "connected", method: "浏览器扩展", kind: "browser", ipc: "none" },
  { id: "vlc", name: "VLC media player", icon: "vlc", process: "vlc.exe", status: "adapted", method: "IPC 接口", kind: "player", ipc: "vlc-http" },
  { id: "potplayer", name: "PotPlayer", icon: "potplayer", process: "potplayer64.exe", status: "adapted", method: "快捷键", kind: "player", ipc: "wm-command" },
  { id: "unknown", name: "未知播放器", icon: "unknown", process: "lesson-04.exe", status: "needs-setup", method: "lesson-04.exe", kind: "unknown", ipc: "none" },
];

export const statusText: Record<AppStatus, string> = {
  connected: "已连接",
  adapted: "已适配",
  "needs-setup": "需要设置",
};

/** 当前被接管的媒体（M0 占位；监听暂停时控制页显示空态） */
export const currentMedia = {
  name: "React 教程",
  source: "Google Chrome",
  icon: "chrome" as BrandId,
};

export interface RecentMedia {
  id: string;
  name: string;
  source: string;
  icon: BrandId;
  rate: number;
  time: string;
}

export const recentMedia: RecentMedia[] = [
  { id: "edge", name: "Microsoft Edge", source: "microsoftedge.exe", icon: "edge", rate: 1.0, time: "刚刚" },
  { id: "vlc", name: "VLC media player", source: "vlc.exe", icon: "vlc", rate: 1.5, time: "2分钟前" },
  { id: "potplayer", name: "PotPlayer", source: "potplayer64.exe", icon: "potplayer", rate: 2.0, time: "10分钟前" },
];

export interface ShortcutItem {
  id: "speedUp" | "speedDown" | "reset" | "playPause" | "togglePanel";
  label: string;
  kind: "up" | "down" | "reset" | "playpause" | "panel";
}

export const shortcutItems: ShortcutItem[] = [
  { id: "speedUp", label: "加速播放", kind: "up" },
  { id: "speedDown", label: "减速播放", kind: "down" },
  { id: "reset", label: "恢复 1.0×", kind: "reset" },
  { id: "playPause", label: "暂停 / 继续", kind: "playpause" },
  { id: "togglePanel", label: "显示控制面板", kind: "panel" },
];

/** 设置页「预设档位」可选值 */
export const presetCandidates = [0.5, 0.75, 1, 1.25, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 16];
