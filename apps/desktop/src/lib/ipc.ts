import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppInfo, AppRulePatch, MediaSession } from "../data";
import type { ShortcutId } from "../store";

/** 浏览器预览（npm run dev）时所有 IPC 均为空操作，仅在 Tauri 环境生效 */

/** 快捷键冲突表：动作 → 用户可读原因（空对象 = 全部注册成功） */
export type ShortcutConflicts = Partial<Record<ShortcutId, string>>;

export interface CoreSnapshot {
  rate: number;
  step: number;
  sliderMax: number;
  hotkeysEnabled: boolean;
  shortcuts: Record<ShortcutId, string[]>;
  conflicts: ShortcutConflicts;
}

/** 全局热键触发事件（Rust `hotkey:triggered`） */
export interface HotkeyPayload {
  action: ShortcutId;
  rate: number;
  seq: number;
}

export async function getCoreState(): Promise<CoreSnapshot | null> {
  if (!isTauri()) return null;
  return invoke<CoreSnapshot>("get_core_state");
}

/** UI 调速后同步权威值到 Rust（返回值与前端 clamp 一致，无需回读） */
export function pushRate(rate: number) {
  if (isTauri()) void invoke("set_rate", { rate });
}

/** 步长 / 滑块上限变化时同步（热键步进依赖这两个值） */
export function pushRateConfig(step: number, sliderMax: number) {
  if (isTauri()) void invoke("sync_rate_config", { step, sliderMax });
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
