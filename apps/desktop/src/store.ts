import { create } from "zustand";
import {
  managedApps,
  methodText,
  previewMedia,
  type AppInfo,
  type AppRulePatch,
  type MediaSession,
} from "./data";
import {
  applyToCurrent,
  getCoreState,
  getCurrentMedia,
  installUpdate,
  listApps,
  onAppsStatusChanged,
  onHotkeyTriggered,
  onMediaChanged,
  onUpdateAvailable,
  pushAppRule,
  pushHotkeysEnabled,
  pushListening,
  pushRate,
  pushSettings,
  pushShortcuts,
  type ShortcutConflicts,
} from "./lib/ipc";

export type Page = "home" | "apps" | "shortcuts" | "settings";
export type StepSize = 0.1 | 0.25 | 0.5;

export const SLIDER_MIN = 0.25;
/** 浏览器内核倍速硬上限（Chromium 16×，见开发文档 §2.1） */
export const MAX_RATE = 16;
/** 滑块显示上限的可选档（PRD §7.4，默认 6×） */
export const SLIDER_MAX_OPTIONS = [6, 8, 16] as const;

export interface Settings {
  defaultRate: number;
  step: StepSize;
  /** 滑块显示上限（默认 6×，最高 16×） */
  sliderMax: number;
  /** 控制页预设档位 */
  presets: number[];
  /** >4× 时提示浏览器将静音音频 */
  highSpeedWarning: boolean;
  /** 高倍速缓冲不足时自动回落 */
  smartSlowdown: boolean;
  /** 变速不变调 */
  preservesPitch: boolean;
  rememberPerApp: boolean;
  startOnBoot: boolean;
  minimizeToTray: boolean;
  autoUpdate: boolean;
}

/** 站点适配规则（PRD §7.2 / §7.6，M3 接入扩展后由 Rust 侧持久化） */
export interface SiteRule {
  host: string;
  name: string;
  /** null = 跟随全局默认倍速 */
  defaultRate: number | null;
  maxRate: number;
  /** 倍速锁定（拦截站点复位） */
  rateLock: boolean;
  /** 短视频流新视频跟随当前倍速 */
  follow: boolean;
}

const defaultSiteRules: SiteRule[] = [
  { host: "bilibili.com", name: "哔哩哔哩", defaultRate: null, maxRate: 16, rateLock: true, follow: true },
  { host: "douyin.com", name: "抖音", defaultRate: null, maxRate: 16, rateLock: true, follow: true },
  { host: "youtube.com", name: "YouTube", defaultRate: null, maxRate: 16, rateLock: true, follow: true },
  { host: "v.qq.com", name: "腾讯视频", defaultRate: null, maxRate: 16, rateLock: true, follow: false },
  { host: "iqiyi.com", name: "爱奇艺", defaultRate: null, maxRate: 16, rateLock: true, follow: false },
  { host: "youku.com", name: "优酷", defaultRate: null, maxRate: 16, rateLock: true, follow: false },
  { host: "ixigua.com", name: "西瓜视频", defaultRate: null, maxRate: 16, rateLock: true, follow: true },
  { host: "kuaishou.com", name: "快手", defaultRate: null, maxRate: 16, rateLock: true, follow: true },
];

export type ShortcutId = "speedUp" | "speedDown" | "reset" | "playPause" | "togglePanel";

export const defaultShortcuts: Record<ShortcutId, string[]> = {
  speedUp: ["Ctrl", "Alt", "↑"],
  speedDown: ["Ctrl", "Alt", "↓"],
  reset: ["Ctrl", "Alt", "0"],
  playPause: ["Ctrl", "Alt", "Space"],
  togglePanel: ["Ctrl", "Alt", "S"],
};

interface AppStore {
  page: Page;
  listening: boolean;
  hotkeysEnabled: boolean;
  rate: number;
  /** 当前被接管的媒体（Rust get_current_media / media:changed；浏览器预览回退 mock） */
  currentMedia: MediaSession | null;
  /** 应用适配列表（Rust list_apps / apps:status-changed；浏览器预览回退 mock） */
  apps: AppInfo[];
  selectedAppId: string;
  settings: Settings;
  siteRules: SiteRule[];
  shortcuts: Record<ShortcutId, string[]>;
  /** 快捷键注册冲突（Rust 侧 RegisterHotKey 失败反馈，快捷键页行内标红） */
  conflicts: ShortcutConflicts;
  /** 检查到的新版本号（null = 无更新），设置页页脚展示 */
  updateAvailable: string | null;
  /** 更新下载安装进行中（按钮防重复点击） */
  updating: boolean;
  setPage: (page: Page) => void;
  toggleListening: () => void;
  setHotkeysEnabled: (enabled: boolean) => void;
  setRate: (rate: number) => void;
  applyRate: (rate: number) => void;
  stepRate: (dir: 1 | -1) => void;
  /** 「应用到当前媒体」：把当前倍速强制下发（PRD §7.1），以 Rust 返回的实际生效值为准 */
  applyToCurrentMedia: () => Promise<void>;
  setSelectedApp: (id: string) => void;
  /** 保存应用规则并用 Rust 返回的完整列表刷新（浏览器预览本地模拟） */
  saveAppRule: (patch: AppRulePatch) => Promise<void>;
  updateSettings: (patch: Partial<Settings>) => void;
  updateSiteRule: (host: string, patch: Partial<SiteRule>) => void;
  setShortcut: (id: ShortcutId, combo: string[]) => void;
  resetShortcuts: () => void;
  /** 保存并注册全部快捷键；返回是否全部注册成功 */
  saveShortcuts: () => Promise<boolean>;
  /** 设置页「更新并重启」：下载安装新版本（成功后应用自动重启） */
  installUpdateNow: () => Promise<void>;
}

const clampRate = (rate: number, max: number) =>
  Math.min(max, Math.max(SLIDER_MIN, Math.round(rate * 100) / 100));

/** 能容纳该倍速的最小显示上限档 */
const ceilingFor = (rate: number) => SLIDER_MAX_OPTIONS.find((m) => m >= rate) ?? MAX_RATE;

export const useAppStore = create<AppStore>()((set, get) => ({
  page: "home",
  listening: true,
  hotkeysEnabled: true,
  rate: 1,
  // initCoreSync 拉取真实数据（浏览器预览回退 mock），初值为空避免 Tauri 下闪现占位数据
  currentMedia: null,
  apps: [],
  selectedAppId: "unknown",
  settings: {
    defaultRate: 1,
    step: 0.25,
    sliderMax: 6,
    presets: [1, 1.5, 2, 3, 4, 5],
    highSpeedWarning: true,
    smartSlowdown: false,
    preservesPitch: true,
    rememberPerApp: true,
    startOnBoot: true,
    minimizeToTray: true,
    autoUpdate: true,
  },
  siteRules: defaultSiteRules,
  shortcuts: { ...defaultShortcuts },
  conflicts: {},
  updateAvailable: null,
  updating: false,
  setPage: (page) => set({ page }),
  toggleListening: () => {
    const listening = !get().listening;
    set({ listening });
    // 同步 Rust 侧暂停 / 恢复全局监听
    pushListening(listening);
  },
  setHotkeysEnabled: (hotkeysEnabled) => {
    set({ hotkeysEnabled });
    // 开启时 Rust 立即注册并返回最新冲突表；关闭时注销全部热键
    void pushHotkeysEnabled(hotkeysEnabled).then((conflicts) => set({ conflicts }));
  },
  setRate: (rate) => {
    const next = clampRate(rate, get().settings.sliderMax);
    pushRate(next);
    set({ rate: next });
  },
  // 预设档位 / 最近媒体是「明确指定某个倍速」的意图，不该被显示上限静默夹掉，
  // 超出时把上限抬到能容纳它的档（滑块拖动与快捷键步进仍受上限约束）
  applyRate: (rate) => {
    const s = get();
    const target = clampRate(rate, MAX_RATE);
    const sliderMax = Math.max(s.settings.sliderMax, ceilingFor(target));
    const settings = sliderMax !== s.settings.sliderMax ? { ...s.settings, sliderMax } : s.settings;
    if (settings !== s.settings) pushSettings(settings);
    pushRate(target);
    set({ rate: target, settings });
  },
  stepRate: (dir) => {
    const s = get();
    const next = clampRate(s.rate + dir * s.settings.step, s.settings.sliderMax);
    pushRate(next);
    set({ rate: next });
  },
  applyToCurrentMedia: async () => {
    const actual = await applyToCurrent(get().rate);
    if (actual !== null) set({ rate: actual });
  },
  setSelectedApp: (selectedAppId) => set({ selectedAppId }),
  saveAppRule: async (patch) => {
    const apps = await pushAppRule(patch);
    if (apps) {
      set({ apps });
      return;
    }
    // 浏览器预览：本地模拟 Rust 侧保存行为（保存后状态即「已适配」，PRD §7.2）
    set((s) => ({
      apps: s.apps.map((a) =>
        a.id === patch.id
          ? {
              ...a,
              method: patch.method,
              methodLabel: methodText[patch.method],
              keys: patch.keys,
              ipcConfig: patch.ipcConfig,
              status: a.status === "connected" ? a.status : "adapted",
            }
          : a,
      ),
    }));
  },
  updateSettings: (patch) => {
    const s = get();
    const settings = { ...s.settings, ...patch };
    pushSettings(settings);
    // 调低上限时同步收拢当前倍速（Rust 侧 save_settings 做同样的收口）
    set({ settings, rate: clampRate(s.rate, settings.sliderMax) });
  },
  updateSiteRule: (host, patch) =>
    set((s) => ({
      siteRules: s.siteRules.map((r) => (r.host === host ? { ...r, ...patch } : r)),
    })),
  setShortcut: (id, combo) => set((s) => ({ shortcuts: { ...s.shortcuts, [id]: combo } })),
  resetShortcuts: () => set({ shortcuts: { ...defaultShortcuts } }),
  saveShortcuts: async () => {
    const conflicts = await pushShortcuts(get().shortcuts);
    set({ conflicts });
    return Object.keys(conflicts).length === 0;
  },
  installUpdateNow: async () => {
    if (get().updating) return;
    set({ updating: true });
    try {
      await installUpdate(); // 成功后应用重启，走不到下面
    } catch (err) {
      console.error("更新失败", err);
      set({ updating: false });
    }
  },
}));

/** 倍速显示格式：2 → "2.0"，1.25 → "1.25" */
export const formatRate = (rate: number) =>
  Number.isInteger(rate * 10) ? rate.toFixed(1) : rate.toFixed(2);

/**
 * 主窗口启动时与 Rust 核心同步（浏览器预览时回退占位数据）：
 * 1. 拉取权威状态（倍速、快捷键、总开关、注册冲突）与 M2 状态（当前媒体、应用列表）；
 * 2. 把前端持有的步长/滑块上限推给 Rust（热键步进依赖）；
 * 3. 订阅全局热键 / 前台媒体 / 应用状态事件，实时刷新。
 */
export async function initCoreSync() {
  const [snap, media, apps] = await Promise.all([getCoreState(), getCurrentMedia(), listApps()]);
  if (snap) {
    useAppStore.setState({
      // 当前媒体可回读真实倍速时以它为准（优先于 Rust 侧记忆的全局倍速）
      rate: media?.rate ?? snap.rate,
      hotkeysEnabled: snap.hotkeysEnabled,
      shortcuts: snap.shortcuts,
      conflicts: snap.conflicts,
      currentMedia: media,
      apps: apps ?? [],
      // 设置以 Rust 持久化值为权威（M4）
      settings: snap.settings,
      listening: snap.listening,
    });
  } else {
    // 纯浏览器预览（npm run dev）：无 Rust 后端，应用列表与当前媒体回退到占位数据
    useAppStore.setState({ apps: managedApps, currentMedia: previewMedia });
  }
  void onHotkeyTriggered((payload) => {
    if (payload.action === "speedUp" || payload.action === "speedDown" || payload.action === "reset") {
      useAppStore.setState({ rate: payload.rate });
    }
  });
  void onMediaChanged((media) => {
    // 接管对象带真实倍速时同步 store（当前媒体的真实倍速优先）
    useAppStore.setState(
      media?.rate != null ? { currentMedia: media, rate: media.rate } : { currentMedia: media },
    );
  });
  void onAppsStatusChanged((apps) => useAppStore.setState({ apps }));
  void onUpdateAvailable((info) => useAppStore.setState({ updateAvailable: info.version }));
}

// 调试辅助：?page=… 直达指定页面（截图回归 / 深链用），
// 在模块加载阶段生效，避免首帧渲染后再切换。
{
  const p = new URLSearchParams(window.location.search).get("page");
  if (p === "home" || p === "apps" || p === "shortcuts" || p === "settings") {
    useAppStore.setState({ page: p });
  }
}
