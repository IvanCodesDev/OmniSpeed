/**
 * OmniSpeed 扩展 Service Worker（MV3, module 型）。
 *
 * 两项职责（开发文档 §5.2 / §5.3）：
 *   1. Native Messaging 客户端：与桌面核心（host: com.omnispeed.host）保持长连接，
 *      断线时静默指数退避重连（chrome.alarms + 内容脚本消息惰性触发双保险）。
 *   2. 标签路由：汇总各 tab/frame 的媒体状态，把「当前活动标签」的状态上报核心；
 *      把核心下发的 setRate / playPause 转给活动标签，config 广播给所有媒体标签。
 *
 * MV3 生命周期说明：NM 端口存活期间 SW 不会被休眠；端口断开后 SW 可能随时被回收，
 * 因此所有事件监听器都在顶层同步注册，config 持久化在 chrome.storage.local，
 * tab 媒体缓存则依赖内容脚本的节流重报自然重建。
 */

import type {
  AppToExt,
  ContentToSw,
  ExtToApp,
  MediaState,
  RateConfig,
  SwToContent,
} from "../shared/protocol";

const NM_HOST = "com.omnispeed.host";
const STORAGE_KEY_CONFIG = "rateConfig";
const ALARM_RECONNECT = "omnispeed-nm-reconnect";
/** 活动标签状态上报的合并节流窗口（毫秒） */
const REPORT_THROTTLE_MS = 200;
/** 重连退避：起始 1s，指数翻倍，封顶 60s */
const RECONNECT_BASE_MS = 1_000;
const RECONNECT_CAP_MS = 60_000;

// ---------- 运行时状态（SW 被回收后丢失，见文件头说明） ----------

let port: chrome.runtime.Port | null = null;
let reconnectDelayMs = RECONNECT_BASE_MS;
let reconnectTimer: number | null = null;

/** 每个 tab 按 frameId 缓存媒体状态（一个 tab 可能有多个含媒体的 iframe） */
const tabMedia = new Map<number, Map<number, MediaState>>();
/** 已回发过缓存 config 的 frame（键 "tabId:frameId"），避免每帧状态都重发 */
const configSentFrames = new Set<string>();
let activeTabId: number | null = null;
let cachedConfig: RateConfig | null = null;
let reportTimer: number | null = null;

// ---------- Native Messaging 连接管理 ----------

function connectHost(): void {
  if (port !== null) return;
  try {
    port = chrome.runtime.connectNative(NM_HOST);
  } catch {
    // 同步抛错（如权限异常）也走静默退避
    port = null;
    scheduleReconnect();
    return;
  }

  port.onMessage.addListener((raw: unknown) => {
    // 收到任何入站帧即证明链路健康：复位退避、撤掉保底闹钟
    reconnectDelayMs = RECONNECT_BASE_MS;
    try {
      void chrome.alarms.clear(ALARM_RECONNECT);
    } catch {
      /* ignore */
    }
    handleAppMessage(raw as AppToExt);
  });

  port.onDisconnect.addListener(() => {
    // 必须读一次 lastError，否则控制台刷 "Unchecked runtime.lastError"。
    // 桌面核心未运行/未注册时 connectNative 会立即走到这里——静默重试，不刷日志。
    void chrome.runtime.lastError;
    port = null;
    scheduleReconnect();
  });

  sendToHost({
    type: "hello",
    version: chrome.runtime.getManifest().version,
  });
}

function scheduleReconnect(): void {
  if (reconnectTimer !== null) return;
  // 快路径：SW 仍存活时用 setTimeout 按退避间隔重试
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectHost();
  }, reconnectDelayMs) as unknown as number;
  reconnectDelayMs = Math.min(reconnectDelayMs * 2, RECONNECT_CAP_MS);
  // 慢路径保底：SW 若被回收，setTimeout 失效，由闹钟唤醒重试。
  // MV3 对 alarms 有最小间隔限制（约 30s–1min），periodInMinutes: 1 已是最细粒度。
  try {
    chrome.alarms.create(ALARM_RECONNECT, { periodInMinutes: 1 });
  } catch {
    /* ignore */
  }
}

function sendToHost(msg: ExtToApp): void {
  if (port === null) {
    connectHost();
    if (port === null) return;
  }
  try {
    port.postMessage(msg);
  } catch {
    // postMessage 抛错说明端口已死，onDisconnect 会接管重连
  }
}

// ---------- 核心 → 扩展（AppToExt） ----------

function handleAppMessage(msg: AppToExt): void {
  switch (msg.type) {
    case "setRate":
      void sendToActiveTab({ type: "media:setRate", rate: msg.rate });
      break;
    case "playPause":
      void sendToActiveTab({ type: "media:playPause" });
      break;
    case "config":
      applyConfig(msg.config);
      break;
    default:
      break;
  }
}

/** 缓存 + 持久化新 config，并广播给所有已知媒体标签 */
function applyConfig(config: RateConfig): void {
  cachedConfig = config;
  try {
    void chrome.storage.local.set({ [STORAGE_KEY_CONFIG]: config });
  } catch {
    /* ignore */
  }
  broadcastConfig();
}

function broadcastConfig(): void {
  if (cachedConfig === null) return;
  const msg: SwToContent = { type: "media:config", config: cachedConfig };
  for (const tabId of tabMedia.keys()) {
    // 没有内容脚本的标签（如 chrome:// 页）sendMessage 会 reject，逐个吞掉
    try {
      void chrome.tabs.sendMessage(tabId, msg).catch(() => {});
    } catch {
      /* ignore */
    }
  }
}

function tabHasMedia(tabId: number): boolean {
  const frames = tabMedia.get(tabId);
  if (frames === undefined) return false;
  for (const state of frames.values()) {
    if (state.hasMedia) return true;
  }
  return false;
}

function mediaTabIds(): number[] {
  const ids: number[] = [];
  for (const [tabId] of tabMedia) {
    if (tabHasMedia(tabId)) ids.push(tabId);
  }
  return ids;
}

/** 全局热键到达时浏览器窗口往往已经失焦：活动标签可能是 chrome:// 或尚未记录。
 *  投递顺序：已知活动标签 → 所有有媒体的标签 → lastFocusedWindow 兜底。 */
async function sendToActiveTab(msg: SwToContent): Promise<void> {
  const ids: number[] = [];
  if (activeTabId !== null) ids.push(activeTabId);
  for (const id of mediaTabIds()) {
    if (!ids.includes(id)) ids.push(id);
  }
  if (ids.length === 0) {
    try {
      const [tab] = await chrome.tabs.query({
        active: true,
        lastFocusedWindow: true,
      });
      if (tab?.id !== undefined) ids.push(tab.id);
    } catch {
      /* ignore */
    }
  }
  await Promise.all(
    ids.map((tabId) => chrome.tabs.sendMessage(tabId, msg).catch(() => {})),
  );
}

// ---------- 内容脚本 → SW（ContentToSw） ----------

chrome.runtime.onMessage.addListener((raw: unknown, sender) => {
  try {
    const tabId = sender.tab?.id;
    if (tabId === undefined) return;
    const frameId = sender.frameId ?? 0;
    const msg = raw as ContentToSw;

    // 惰性重连保险：任何内容脚本消息到达都是一次重连机会
    if (port === null) connectHost();

    switch (msg.type) {
      case "media:state":
        handleMediaState(tabId, frameId, msg.state, sender.tab?.title);
        break;
      case "media:userRate":
        handleUserRate(tabId, frameId, msg.rate);
        break;
      default:
        break;
    }
  } catch {
    /* ignore */
  }
});

function handleMediaState(
  tabId: number,
  frameId: number,
  state: MediaState,
  tabTitle: string | undefined,
): void {
  // iframe 的 document.title 没有意义，统一以 tab 标题覆盖；host 保留内容脚本上报值
  if (tabTitle !== undefined && tabTitle !== "") {
    state = { ...state, title: tabTitle };
  }
  let frames = tabMedia.get(tabId);
  if (frames === undefined) {
    frames = new Map();
    tabMedia.set(tabId, frames);
  }
  frames.set(frameId, state);

  // 首次见到该 frame：回发缓存 config，保证新页面拿到 targetRate/rateLock
  const frameKey = `${tabId}:${frameId}`;
  if (!configSentFrames.has(frameKey)) {
    configSentFrames.add(frameKey);
    if (cachedConfig !== null) {
      const msg: SwToContent = { type: "media:config", config: cachedConfig };
      try {
        void chrome.tabs.sendMessage(tabId, msg, { frameId }).catch(() => {});
      } catch {
        /* ignore */
      }
    }
  }

  if (tabId === activeTabId) scheduleReport();
}

/** 用户在站点 UI 主动调速：同步为新目标并全量广播，同时向核心补报一帧该 tab 状态 */
function handleUserRate(tabId: number, frameId: number, rate: number): void {
  cachedConfig =
    cachedConfig !== null
      ? { ...cachedConfig, targetRate: rate }
      : { targetRate: rate, rateLock: false, maxRate: 16, preservesPitch: true, siteRules: [] };
  try {
    void chrome.storage.local.set({ [STORAGE_KEY_CONFIG]: cachedConfig });
  } catch {
    /* ignore */
  }
  broadcastConfig();

  // 更新缓存中该 frame 的 rate，随后直接上报（用户调速语义明确，不走节流）
  const frameState = tabMedia.get(tabId)?.get(frameId);
  if (frameState !== undefined) frameState.rate = rate;
  const best = bestStateOf(tabId);
  if (best !== null) sendToHost({ type: "media", state: best });
}

// ---------- 活动标签跟踪与状态上报 ----------

/** 多 frame 取舍：优先 hasMedia 的 frame（frameId 最小者），其次主 frame，最后任一 */
function bestStateOf(tabId: number): MediaState | null {
  const frames = tabMedia.get(tabId);
  if (frames === undefined || frames.size === 0) return null;
  let withMedia: MediaState | null = null;
  let withMediaFrameId = Number.POSITIVE_INFINITY;
  let mainFrame: MediaState | null = null;
  let fallback: MediaState | null = null;
  for (const [frameId, state] of frames) {
    if (state.hasMedia && frameId < withMediaFrameId) {
      withMedia = state;
      withMediaFrameId = frameId;
    }
    if (frameId === 0) mainFrame = state;
    if (fallback === null) fallback = state;
  }
  return withMedia ?? mainFrame ?? fallback;
}

/** 200ms 合并节流：短时间内的连发（切标签+状态帧）只上报最终态，避免风暴 */
function scheduleReport(): void {
  if (reportTimer !== null) return;
  reportTimer = setTimeout(() => {
    reportTimer = null;
    reportActiveState();
  }, REPORT_THROTTLE_MS) as unknown as number;
}

function reportActiveState(): void {
  const state = activeTabId !== null ? bestStateOf(activeTabId) : null;
  sendToHost({ type: "media", state });
}

chrome.tabs.onActivated.addListener((info) => {
  try {
    activeTabId = info.tabId;
    scheduleReport();
  } catch {
    /* ignore */
  }
});

chrome.windows.onFocusChanged.addListener((windowId) => {
  try {
    // 失焦（WINDOW_ID_NONE）时保留最后的活动标签：全局快捷键仍应作用于它
    if (windowId === chrome.windows.WINDOW_ID_NONE) return;
    void chrome.tabs
      .query({ active: true, windowId })
      .then(([tab]) => {
        if (tab?.id !== undefined) {
          activeTabId = tab.id;
          scheduleReport();
        }
      })
      .catch(() => {});
  } catch {
    /* ignore */
  }
});

chrome.tabs.onUpdated.addListener((tabId, changeInfo) => {
  try {
    if (changeInfo.url !== undefined) {
      // 导航离开：旧页面的媒体状态与「已发 config」标记全部作废，
      // 新页面的内容脚本会重新上报并重新拿到 config
      tabMedia.delete(tabId);
      clearConfigSentForTab(tabId);
      if (tabId === activeTabId) scheduleReport();
      return;
    }
    if (changeInfo.title !== undefined) {
      const frames = tabMedia.get(tabId);
      if (frames !== undefined) {
        for (const state of frames.values()) state.title = changeInfo.title;
      }
      if (tabId === activeTabId) scheduleReport();
    }
  } catch {
    /* ignore */
  }
});

chrome.tabs.onRemoved.addListener((tabId) => {
  try {
    tabMedia.delete(tabId);
    clearConfigSentForTab(tabId);
    if (tabId === activeTabId) {
      activeTabId = null; // 随后 onActivated 会带来新的活动标签
      scheduleReport();
    }
  } catch {
    /* ignore */
  }
});

function clearConfigSentForTab(tabId: number): void {
  const prefix = `${tabId}:`;
  for (const key of configSentFrames) {
    if (key.startsWith(prefix)) configSentFrames.delete(key);
  }
}

// ---------- 闹钟：SW 被回收后的重连保底 ----------

chrome.alarms.onAlarm.addListener((alarm) => {
  try {
    if (alarm.name !== ALARM_RECONNECT) return;
    if (port === null) {
      connectHost();
    } else {
      void chrome.alarms.clear(ALARM_RECONNECT);
    }
  } catch {
    /* ignore */
  }
});

// ---------- SW 启动（安装、浏览器启动、休眠唤醒均会执行顶层代码） ----------

void (async () => {
  try {
    const stored = await chrome.storage.local.get(STORAGE_KEY_CONFIG);
    const restored = stored[STORAGE_KEY_CONFIG] as RateConfig | undefined;
    if (restored !== undefined && cachedConfig === null) cachedConfig = restored;
  } catch {
    /* ignore */
  }
})();

void (async () => {
  try {
    const [tab] = await chrome.tabs.query({
      active: true,
      lastFocusedWindow: true,
    });
    activeTabId = tab?.id ?? null;
  } catch {
    /* ignore */
  }
})();

connectHost();
