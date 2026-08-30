import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppInfo, AppRulePatch, MediaSession } from "../data";
import type { Settings, ShortcutId, SiteRule } from "../store";

/** 浏览器预览（npm run dev）时所有 IPC 均为空操作，仅在 Tauri 环境生效 */

/** 快捷键冲突表：动作 → 用户可读原因（空对象 = 全部注册成功） */
export type ShortcutConflicts = Partial<Record<ShortcutId, string>>;

export interface CoreSnapshot {
  rate: number;
  hotkeysEnabled: boolean;
  shortcuts: Record<ShortcutId, string[]>;
  conflicts: ShortcutConflicts;
  listening: boolean;
  /** 设置页全部选项（Rust 权威，含持久化值） */
  settings: Settings;
}

/** 全局热键触发事件（Rust `hotkey:triggered`） */
export interface HotkeyPayload {
  action: ShortcutId;
  rate: number;
  seq: number;
  /** OSD 副文案（>4× 浏览器静音提示 / 智能降速说明） */
  notice?: string | null;
}

export async function getCoreState(): Promise<CoreSnapshot | null> {
  if (!isTauri()) return null;
  return invoke<CoreSnapshot>("get_core_state");
}

/** UI 调速后同步权威值到 Rust（返回值与前端 clamp 一致，无需回读） */
export function pushRate(rate: number) {
  if (isTauri()) void invoke("set_rate", { rate });
}

/** 设置页保存：推送完整设置对象（Rust 侧为权威并落盘） */
export function pushSettings(settings: Settings) {
  if (isTauri()) void invoke("save_settings", { settings });
}

export async function pushShortcuts(
  shortcuts: Record<ShortcutId, string[]>,
): Promise<ShortcutConflicts> {
  if (!isTauri()) return {};
  return invoke<ShortcutConflicts>("save_shortcuts", { shortcuts });
}

export async function pushHotkeysEnabled(enabled: boolean): Promise<ShortcutConflicts> {
  if (!isTauri()) return {};
  return invoke<ShortcutConflicts>("set_hotkeys_enabled", { enabled });
}

export async function onHotkeyTriggered(
  handler: (payload: HotkeyPayload) => void,
): Promise<UnlistenFn | null> {
  if (!isTauri()) return null;
  return listen<HotkeyPayload>("hotkey:triggered", (event) => handler(event.payload));
}

/* ── M2：桌面播放器适配（前台应用识别 / 播放器控制） ── */

/** 应用适配列表（浏览器预览返回 null，由 store 回退 mock） */
export async function listApps(): Promise<AppInfo[] | null> {
  if (!isTauri()) return null;
  return invoke<AppInfo[]>("list_apps");
}

/** 保存应用规则，返回更新后的完整列表（浏览器预览返回 null，由 store 本地模拟） */
export async function pushAppRule(rule: AppRulePatch): Promise<AppInfo[] | null> {
  if (!isTauri()) return null;
  return invoke<AppInfo[]>("save_app_rule", { rule });
}

/** 当前被接管的媒体（null = 无接管对象） */
export async function getCurrentMedia(): Promise<MediaSession | null> {
  if (!isTauri()) return null;
  return invoke<MediaSession | null>("get_current_media");
}

/* ── M3.5：站点级规则（应用页「网站适配」） ── */

/** 站点规则列表（浏览器预览返回 null，由 store 回退默认表） */
export async function listSiteRules(): Promise<SiteRule[] | null> {
  if (!isTauri()) return null;
  return invoke<SiteRule[]>("list_site_rules");
}

/** 保存单条站点规则，返回更新后的完整列表（Rust 落盘并即时推送浏览器扩展） */
export async function pushSiteRule(rule: SiteRule): Promise<SiteRule[] | null> {
  if (!isTauri()) return null;
  return invoke<SiteRule[]>("save_site_rule", { rule });
}

/** 暂停 / 恢复全局监听 */
export function pushListening(enabled: boolean) {
  if (isTauri()) void invoke("set_listening", { enabled });
}

/** 「应用到当前媒体」：返回实际生效倍速（浏览器预览返回 null） */
export async function applyToCurrent(rate: number): Promise<number | null> {
  if (!isTauri()) return null;
  return invoke<number>("apply_to_current", { rate });
}

/** 前台接管对象变化（Rust `media:changed`） */
export async function onMediaChanged(
  handler: (media: MediaSession | null) => void,
): Promise<UnlistenFn | null> {
  if (!isTauri()) return null;
  return listen<MediaSession | null>("media:changed", (event) => handler(event.payload));
}

/** 应用运行状态 / 规则变化（Rust `apps:status-changed`） */
export async function onAppsStatusChanged(
  handler: (apps: AppInfo[]) => void,
): Promise<UnlistenFn | null> {
  if (!isTauri()) return null;
  return listen<AppInfo[]>("apps:status-changed", (event) => handler(event.payload));
}

/* ── M4：自动更新 ── */

export interface UpdateInfo {
  version: string;
}

/** 检查到新版本（Rust `update:available`，启动后与每 24h 检查一次） */
export async function onUpdateAvailable(
  handler: (info: UpdateInfo) => void,
): Promise<UnlistenFn | null> {
  if (!isTauri()) return null;
  return listen<UpdateInfo>("update:available", (event) => handler(event.payload));
}

/** 下载并安装更新，随后自动重启（设置页「更新并重启」） */
export async function installUpdate(): Promise<void> {
  if (!isTauri()) return;
  await invoke("install_update");
}
