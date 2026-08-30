/**
 * 领域类型（与 Rust 侧 serde camelCase 契约一致）与浏览器预览 fallback 数据。
 * Tauri 环境下应用列表 / 当前媒体由 IPC（list_apps / get_current_media / 事件）提供，
 * 此处 mock 仅用于 npm run dev 纯浏览器预览。
 */

export type AppStatus = "connected" | "adapted" | "needs-setup";
export type BrandId = "chrome" | "edge" | "vlc" | "potplayer" | "unknown";
export type AppKind = "browser" | "player" | "client" | "unknown";
/** 播放器 IPC 通道类型（开发文档 §7.3；cdp = Chromium 套壳客户端调试口接管） */
export type IpcKind = "mpv-ipc" | "vlc-http" | "wm-command" | "cdp" | "none";
export type AppMethod = "auto" | "ipc" | "hotkey" | "extension";

/** 目标软件自身的调速快捷键（PRD §7.2：OmniSpeed 模拟按键「替用户按」） */
export interface AppRuleKeys {
  up: string;
  down: string;
  reset: string;
}

export interface AppIpcConfig {
  pipe: string | null;
  port: number | null;
  password: string | null;
}

/**
 * 播放器自身快捷键构成的倍速网格（Rust `KeyRateGrid`）。
 * 有它说明该播放器的某些按键给的是**绝对倍速**（如百度网盘桌面端的数字键 1–5），
 * 于是按键通道也能精确设速：先按最近的档位键钉住，再用步进键补足差额。
 */
export interface AppKeyRate {
  anchors: { key: string; rate: number }[];
  /** up/down 每按一次的增减量 */
  step: number;
  min: number;
  max: number;
  /** 连发步进键的最小间隔，贴着发会被播放器吞掉步进（各播放器实测值） */
  stepGapMs: number;
}

/** 应用适配条目（Rust `list_apps` / `apps:status-changed`） */
export interface AppInfo {
  id: string;
  name: string;
  /** 小写 exe 名，如 "vlc.exe" */
  process: string;
  kind: AppKind;
  status: AppStatus;
  method: AppMethod;
  /** 控制方式展示文案，如「IPC 接口」/「快捷键」/「浏览器扩展」 */
  methodLabel: string;
  ipc: IpcKind;
  /** 是否检测到进程运行中 */
  running: boolean;
  /** 内置规则不可删除 */
  builtin: boolean;
  keys: AppRuleKeys | null;
  ipcConfig: AppIpcConfig | null;
  /** 非空 = 按键通道可精确设速，应用页据此说明可用区间与精度 */
  keyRate: AppKeyRate | null;
}

/** 「应用页」保存规则的提交体（Rust `save_app_rule`，返回更新后的完整列表） */
export interface AppRulePatch {
  id: string;
  process: string;
  name: string;
  method: Exclude<AppMethod, "extension">;
  keys: AppRuleKeys | null;
  ipcConfig: AppIpcConfig | null;
}

/** 当前被接管的媒体（Rust `get_current_media` / `media:changed`） */
export interface MediaSession {
  appId: string;
  /** 应用显示名（网页标题等 M3 起提供） */
  name: string;
  /** 来源进程名 */
  source: string;
  kind: AppKind;
  status: AppStatus;
  /** 适配器可回读时为真实倍速，否则 null */
  rate: number | null;
  canReadBack: boolean;
}

export const statusText: Record<AppStatus, string> = {
  connected: "已连接",
  adapted: "已适配",
  "needs-setup": "需要设置",
};

/** 控制方式展示文案（浏览器预览本地模拟保存时用；Tauri 下以 Rust 返回的 methodLabel 为准） */
export const methodText: Record<AppMethod, string> = {
  auto: "自动识别",
  ipc: "IPC 接口",
  hotkey: "快捷键",
  extension: "浏览器扩展",
};

const brandIds = ["chrome", "edge", "vlc", "potplayer"] as const;

/** 图标不进 IPC 契约：按 id / 进程名推导品牌图标，未知一律 "unknown" */
export function brandOf(...hints: (string | null | undefined)[]): BrandId {
  for (const hint of hints) {
    const s = hint?.toLowerCase() ?? "";
    const hit = brandIds.find((b) => s.includes(b));
    if (hit) return hit;
  }
  return "unknown";
}

/** 浏览器预览 fallback：与 Rust 侧内置规则同构的应用列表 */
export const managedApps: AppInfo[] = [
  {
    id: "edge",
    name: "Microsoft Edge",
    process: "msedge.exe",
    kind: "browser",
    status: "connected",
    method: "extension",
    methodLabel: "浏览器扩展",
    ipc: "none",
    running: true,
    builtin: true,
    keys: null,
    ipcConfig: null,
    keyRate: null,
  },
  {
    id: "chrome",
    name: "Google Chrome",
    process: "chrome.exe",
    kind: "browser",
    status: "connected",
    method: "extension",
    methodLabel: "浏览器扩展",
    ipc: "none",
    running: true,
    builtin: true,
    keys: null,
    ipcConfig: null,
    keyRate: null,
  },
  {
    id: "vlc",
    name: "VLC media player",
    process: "vlc.exe",
    kind: "player",
    status: "adapted",
    method: "ipc",
    methodLabel: "IPC 接口",
    ipc: "vlc-http",
    running: true,
    builtin: true,
    keys: { up: "]", down: "[", reset: "=" },
    ipcConfig: { pipe: null, port: 8080, password: null },
    keyRate: null,
  },
  {
    id: "potplayer",
    name: "PotPlayer",
    process: "potplayer64.exe",
    kind: "player",
    status: "adapted",
    method: "hotkey",
    methodLabel: "快捷键",
    ipc: "wm-command",
    running: true,
    builtin: true,
    keys: { up: "C", down: "X", reset: "Z" },
    ipcConfig: null,
    keyRate: null,
  },
  {
    id: "mpv",
    name: "mpv",
    process: "mpv.exe",
    kind: "player",
    status: "adapted",
    method: "ipc",
    methodLabel: "IPC 接口",
    ipc: "mpv-ipc",
    running: false,
    builtin: true,
    keys: { up: "]", down: "[", reset: "Backspace" },
    ipcConfig: { pipe: "\\\\.\\pipe\\mpvsocket", port: null, password: null },
    keyRate: null,
  },
  {
    id: "bilibili-client",
    name: "哔哩哔哩桌面端",
    process: "哔哩哔哩.exe",
    kind: "client",
    status: "adapted",
    method: "ipc",
    methodLabel: "CDP 接管",
    ipc: "cdp",
    running: false,
    builtin: true,
    keys: null,
    ipcConfig: { pipe: null, port: 9333, password: null },
    keyRate: null,
  },
  {
    id: "baidu-netdisk",
    name: "百度网盘",
    process: "baidunetdiskunite.exe",
    kind: "client",
    status: "adapted",
    method: "hotkey",
    methodLabel: "播放器快捷键 · 精确设速",
    ipc: "none",
    running: false,
    builtin: true,
    keys: { up: ".", down: ",", reset: "1" },
    ipcConfig: null,
    keyRate: {
      anchors: [1, 2, 3, 4, 5].map((rate) => ({ key: String(rate), rate })),
      step: 0.1,
      min: 0.5,
      max: 5,
      stepGapMs: 150,
    },
  },
  {
    id: "unknown",
    name: "未知播放器",
    process: "lesson-04.exe",
    kind: "unknown",
    status: "needs-setup",
    method: "auto",
    methodLabel: "未配置",
    ipc: "none",
    running: true,
    builtin: false,
    keys: null,
    ipcConfig: null,
    keyRate: null,
  },
];

/** 浏览器预览 fallback：当前被接管的媒体（Tauri 下由 get_current_media / media:changed 提供） */
export const previewMedia: MediaSession = {
  appId: "chrome",
  name: "React 教程",
  source: "chrome.exe",
  kind: "browser",
  status: "connected",
  rate: null,
  canReadBack: false,
};

export interface RecentMedia {
  id: string;
  name: string;
  source: string;
  icon: BrandId;
  rate: number;
  time: string;
}

/** 最近媒体占位（真实历史数据 M4 接入） */
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
