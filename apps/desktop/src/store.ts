import { create } from "zustand";
import {
  getCoreState,
  onHotkeyTriggered,
  pushHotkeysEnabled,
  pushRate,
  pushRateConfig,
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
  selectedAppId: string;
  settings: Settings;
  siteRules: SiteRule[];
  shortcuts: Record<ShortcutId, string[]>;
  /** 快捷键注册冲突（Rust 侧 RegisterHotKey 失败反馈，快捷键页行内标红） */
  conflicts: ShortcutConflicts;
  /** 已在「应用页」保存过规则的应用（保存后状态显示为「已适配」） */
  ruleSavedIds: string[];
  setPage: (page: Page) => void;
  toggleListening: () => void;
  setHotkeysEnabled: (enabled: boolean) => void;
  setRate: (rate: number) => void;
  applyRate: (rate: number) => void;
  stepRate: (dir: 1 | -1) => void;
  setSelectedApp: (id: string) => void;
  updateSettings: (patch: Partial<Settings>) => void;
  updateSiteRule: (host: string, patch: Partial<SiteRule>) => void;
  setShortcut: (id: ShortcutId, combo: string[]) => void;
  resetShortcuts: () => void;
  /** 保存并注册全部快捷键；返回是否全部注册成功 */
  saveShortcuts: () => Promise<boolean>;
  markRuleSaved: (id: string) => void;
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
  ruleSavedIds: [],
  setPage: (page) => set({ page }),
  toggleListening: () => set((s) => ({ listening: !s.listening })),
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
    if (sliderMax !== s.settings.sliderMax) pushRateConfig(s.settings.step, sliderMax);
    pushRate(target);
    set({ rate: target, settings: { ...s.settings, sliderMax } });
  },
  stepRate: (dir) => {
    const s = get();
    const next = clampRate(s.rate + dir * s.settings.step, s.settings.sliderMax);
    pushRate(next);
    set({ rate: next });
  },
  setSelectedApp: (selectedAppId) => set({ selectedAppId }),
  updateSettings: (patch) => {
    const s = get();
    const settings = { ...s.settings, ...patch };
    if (settings.step !== s.settings.step || settings.sliderMax !== s.settings.sliderMax) {
      pushRateConfig(settings.step, settings.sliderMax);
    }
    // 调低上限时同步收拢当前倍速（Rust 侧 sync_rate_config 做同样的收口）
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
  markRuleSaved: (id) =>
    set((s) => (s.ruleSavedIds.includes(id) ? s : { ruleSavedIds: [...s.ruleSavedIds, id] })),
}));

/** 倍速显示格式：2 → "2.0"，1.25 → "1.25" */
export const formatRate = (rate: number) =>
  Number.isInteger(rate * 10) ? rate.toFixed(1) : rate.toFixed(2);

/**
 * 主窗口启动时与 Rust 核心同步（浏览器预览时为空操作）：
 * 1. 拉取权威状态（倍速、快捷键、总开关、注册冲突）；
 * 2. 把前端持有的步长/滑块上限推给 Rust（热键步进依赖）；
 * 3. 订阅全局热键事件，实时刷新倍速显示。
 */
export async function initCoreSync() {
  const snap = await getCoreState();
  if (snap) {
    useAppStore.setState({
      rate: snap.rate,
      hotkeysEnabled: snap.hotkeysEnabled,
      shortcuts: snap.shortcuts,
      conflicts: snap.conflicts,
    });
    const { step, sliderMax } = useAppStore.getState().settings;
    pushRateConfig(step, sliderMax);
  }
  void onHotkeyTriggered((payload) => {
    if (payload.action === "speedUp" || payload.action === "speedDown" || payload.action === "reset") {
      useAppStore.setState({ rate: payload.rate });
    }
  });
}

// 调试辅助：?page=… 直达指定页面（截图回归 / 深链用），
// 在模块加载阶段生效，避免首帧渲染后再切换。
{
  const p = new URLSearchParams(window.location.search).get("page");
  if (p === "home" || p === "apps" || p === "shortcuts" || p === "settings") {
    useAppStore.setState({ page: p });
  }
}
