/**
 * OmniSpeed content script —— 接管与上报。
 * ISOLATED world / document_start / all_frames（manifest 固定，勿改）。
 *
 * 职责（开发文档 §5.2）：
 *  - 媒体注册表：初扫 + MutationObserver 动态接管——抖音等虚拟列表滑动换视频
 *    「新元素秒级接管」的关键。
 *  - 主媒体选择与状态上报（chrome.runtime → SW，类型严格用 ../shared/protocol.ts）。
 *  - 收 SW 指令并执行；其中倍速写入统一交给 MAIN world 的 rate-guard（world 隔离：
 *    isolated 直接写 playbackRate 走本 world 原生 setter，不经过站点可见的代理，
 *    也享受不到 Rate-Guard 的锁定语义；读 playbackRate 则不受 MAIN 补丁影响，可直接读）。
 *  - SPA 导航兜底重扫；广告时段暂停干预（不强写倍速）。
 *
 * iframe：all_frames 注入，每个 frame 管自己的媒体、独立上报；SW 按 sender.frameId
 * 汇总，标题会被 SW 用 tab.title 覆盖，这里不用操心。
 */

import type { ContentToSw, MediaState, RateConfig, SwToContent } from "../shared/protocol";
import { adapterFor } from "../sites";
import {
  DEFAULT_MAX_RATE,
  GUARD_NS,
  clampRate,
  type ContentToGuard,
  type GuardToContent,
} from "./guard-protocol";

const REPORT_THROTTLE_MS = 300;
const HEARTBEAT_MS = 1500;
const NAV_POLL_MS = 500;

const adapter = adapterFor(location.host);

/** SW 下发前的安全默认：无目标、不锁定（与 rate-guard 侧默认一致） */
const config: RateConfig = {
  targetRate: null,
  rateLock: false,
  maxRate: DEFAULT_MAX_RATE,
  preservesPitch: true,
};

// ============ 与 rate-guard（MAIN world）通信 ============

function postToGuard(msg: ContentToGuard): void {
  window.postMessage(msg, "*");
}

function syncGuardConfig(): void {
  postToGuard({
    ns: GUARD_NS,
    dir: "content->guard",
    type: "config",
    config: { targetRate: config.targetRate, rateLock: config.rateLock, maxRate: config.maxRate },
  });
}

/** 请求 rate-guard 把 targetRate 写到本 frame 全部媒体（新元素跟随/切集恢复共用此入口） */
function applyTarget(): void {
  if (config.targetRate === null) return;
  postToGuard({ ns: GUARD_NS, dir: "content->guard", type: "setRate", rate: config.targetRate });
}

/** 自动干预（跟随/锁定恢复）的前提：有目标 + 锁定开启 + 非广告时段 */
function shouldEnforce(): boolean {
  return config.targetRate !== null && config.rateLock && !safeAdCheck();
}

function safeAdCheck(): boolean {
  try {
    return adapter.isAd?.() ?? false;
  } catch {
    return false;
  }
}

function safeLiveCheck(): boolean {
  try {
    return adapter.isLive?.() ?? false;
  } catch {
    return false;
  }
}

// ============ 媒体注册表 ============

let seqCounter = 0;
/** value = 注册顺序号（「最新出现」判据） */
const registry = new Map<HTMLMediaElement, number>();

/**
 * 是否按媒体元素接管。bwp-video（B 站 WASM 播放器，Spike #4）等非 HTMLMediaElement
 * 但暴露 playbackRate 的元素做鸭子类型接纳，后续读写全部 try/catch。
 */
function isMediaLike(el: Element): el is HTMLMediaElement {
  if (el instanceof HTMLMediaElement) return true;
  return typeof (el as { playbackRate?: unknown }).playbackRate === "number";
}

function register(el: HTMLMediaElement): void {
  if (registry.has(el)) return;
  registry.set(el, ++seqCounter);
  startNavPoll();
  try {
    el.preservesPitch = config.preservesPitch;
  } catch {
    /* bwp-video 等鸭子类型可能不支持 */
  }
  // 短视频流滑动跟随 + 切集恢复：新出现的媒体立即应用目标倍速（广告时段暂停干预）
  if (shouldEnforce()) applyTarget();
  reportSoon();
}

function prune(): void {
  for (const el of [...registry.keys()]) {
    if (!el.isConnected) registry.delete(el);
  }
}

function scan(root: ParentNode = document): void {
  const selector = adapter.videoSelector
    ? `video, audio, ${adapter.videoSelector}`
    : "video, audio";
  let els: NodeListOf<Element>;
  try {
    els = root.querySelectorAll(selector);
  } catch {
    els = root.querySelectorAll("video, audio");
  }
  for (const el of els) {
    if (isMediaLike(el)) register(el);
  }
}

// 动态接管：childList + subtree 覆盖虚拟列表换视频、懒加载播放器等一切 DOM 注入路径
const observer = new MutationObserver((mutations) => {
  let touched = false;
  for (const m of mutations) {
    for (const node of m.addedNodes) {
      if (!(node instanceof Element)) continue;
      if (isMediaLike(node)) {
        register(node);
        touched = true;
      } else if (node.firstElementChild) {
        scan(node);
      }
    }
    if (m.removedNodes.length > 0 && registry.size > 0) touched = true;
  }
  if (touched) reportSoon();
});
observer.observe(document, { childList: true, subtree: true });

// ============ 主媒体选择 ============

function visibleArea(el: Element): number {
  try {
    const r = el.getBoundingClientRect();
    const w = Math.min(r.right, innerWidth) - Math.max(r.left, 0);
    const h = Math.min(r.bottom, innerHeight) - Math.max(r.top, 0);
    return w > 0 && h > 0 ? w * h : 0;
  } catch {
    return 0;
  }
}

/** 站点适配器 videoSelector 命中优先；否则「正在播放 > 可见面积最大 > 最新出现」 */
function mainMedia(): HTMLMediaElement | null {
  if (adapter.videoSelector) {
    try {
      const el = document.querySelector(adapter.videoSelector);
      if (el && isMediaLike(el) && el.isConnected) {
        register(el);
        return el;
      }
    } catch {
      /* 适配器 selector 异常时回落到通用规则 */
    }
  }
  prune();
  let best: HTMLMediaElement | null = null;
  let bestPlaying = -1;
  let bestArea = -1;
  let bestSeq = -1;
  for (const [el, seq] of registry) {
    // 鸭子类型元素 paused 可能为 undefined，用显式布尔比较避免误判
    const playing = el.paused === false && el.ended !== true ? 1 : 0;
    const area = visibleArea(el);
    if (
      playing > bestPlaying ||
      (playing === bestPlaying && area > bestArea) ||
      (playing === bestPlaying && area === bestArea && seq > bestSeq)
    ) {
      best = el;
      bestPlaying = playing;
      bestArea = area;
      bestSeq = seq;
    }
  }
  return best;
}

// ============ 状态上报（变化即报 300ms 节流 + 有媒体时 1.5s 心跳） ============

function bufferedAhead(m: HTMLMediaElement): number | null {
  // 缓冲前沿余量：高倍速警示用（开发文档 §7.8 缓冲监控）
  try {
    const t = m.currentTime;
    const b = m.buffered;
    for (let i = 0; i < b.length; i++) {
      if (b.start(i) <= t && t <= b.end(i)) return b.end(i) - t;
    }
  } catch {
    /* 鸭子类型/跨源等异常 → 未知 */
  }
  return null;
}

function buildState(): MediaState {
  const m = mainMedia();
  if (!m) {
    // hasMedia=false 也要报：SW 据此清空本 frame 状态
    return {
      hasMedia: false,
      rate: 1,
      title: document.title,
      host: location.host,
      isLive: false,
      adPlaying: false,
      bufferedAhead: null,
      adapter: adapter.id,
    };
  }
  let rate = 1;
  try {
    // isolated 的原生 getter 不受 MAIN world 补丁影响，读到的即真实值
    rate = m.playbackRate;
  } catch {
    /* 鸭子类型异常 */
  }
  return {
    hasMedia: true,
    rate,
    title: document.title,
    host: location.host,
    isLive: safeLiveCheck(),
    adPlaying: safeAdCheck(),
    bufferedAhead: bufferedAhead(m),
    adapter: adapter.id,
  };
}

function sendToSw(msg: ContentToSw): void {
  // 扩展更新 / SW 重启时 sendMessage 会抛 Extension context invalidated，
  // 无接收端时返回被拒的 Promise —— 全部吞掉，页面侧不受影响。
  try {
    if (!chrome.runtime?.id) return;
    void chrome.runtime.sendMessage(msg).catch(() => {});
  } catch {
    /* context invalidated */
  }
}

let lastSentAt = 0;
let pendingTimer: number | null = null;
let lastSentJson = "";

/** force：心跳/导航等场景绕过「与上次相同则不发」的去重，但仍更新时间基准 */
function sendState(force: boolean): void {
  lastSentAt = Date.now();
  const state = buildState();
  const json = JSON.stringify(state);
  if (!force && json === lastSentJson) return;
  lastSentJson = json;
  sendToSw({ type: "media:state", state });
}

/**
 * 变化触发的节流上报：窗口内多次变化只发最后一帧（发送时才 buildState，
 * 保证不丢最终态）。
 */
function reportSoon(): void {
  const wait = lastSentAt + REPORT_THROTTLE_MS - Date.now();
  if (wait <= 0) {
    sendState(false);
  } else if (pendingTimer === null) {
    pendingTimer = window.setTimeout(() => {
      pendingTimer = null;
      sendState(false);
    }, wait);
  }
}

// 心跳 + 广告结束恢复：有媒体时 1.5s 强制上报（title/缓冲/直播态等无事件字段靠它刷新）
let lastAdPlaying = false;
window.setInterval(() => {
  prune();
  const ad = safeAdCheck();
  if (lastAdPlaying && !ad && config.rateLock && config.targetRate !== null) {
    // 广告 → 正片：恢复目标倍速（PRD §7.6）
    applyTarget();
  }
  lastAdPlaying = ad;
  if (registry.size > 0) sendState(true);
}, HEARTBEAT_MS);

// 媒体事件驱动的即时上报；capture 监听可收到不冒泡的媒体事件
for (const type of ["play", "pause", "ratechange", "ended", "loadstart", "durationchange"]) {
  document.addEventListener(
    type,
    (e: Event) => {
      const t = e.target;
      if (t instanceof Element && isMediaLike(t)) {
        register(t);
        reportSoon();
      }
    },
    true,
  );
}

// ============ 收 rate-guard 通知（用户在站点 UI 主动调速） ============

window.addEventListener("message", (e: MessageEvent) => {
  if (e.source !== window) return;
  const data = e.data as GuardToContent | null | undefined;
  if (!data || typeof data !== "object" || data.ns !== GUARD_NS || data.dir !== "guard->content") {
    return;
  }
  if (data.type === "userRate" && typeof data.rate === "number") {
    // 同步为新目标（guard 侧已同步自己的 targetRate），并上报 media:userRate
    config.targetRate = data.rate;
    sendToSw({ type: "media:userRate", rate: data.rate });
    reportSoon();
  }
});

// ============ 收 SW 指令（SwToContent） ============

function applyConfig(next: RateConfig): void {
  config.maxRate = next.maxRate;
  config.rateLock = next.rateLock;
  config.targetRate = next.targetRate === null ? null : clampRate(next.targetRate, next.maxRate);
  config.preservesPitch = next.preservesPitch;
  syncGuardConfig();
  // preservesPitch 是实例属性，isolated 直接设即可（不受 world 隔离影响）
  prune();
  for (const el of registry.keys()) {
    try {
      el.preservesPitch = config.preservesPitch;
    } catch {
      /* 鸭子类型 */
    }
  }
  if (shouldEnforce()) applyTarget();
  reportSoon();
}

try {
  chrome.runtime.onMessage.addListener((raw: unknown) => {
    const msg = raw as SwToContent;
    if (!msg || typeof msg !== "object") return;
    switch (msg.type) {
      case "media:setRate": {
        if (typeof msg.rate !== "number") break;
        // 显式指令（全局快捷键/控制页）：更新本地目标并转发 rate-guard 执行
        config.targetRate = clampRate(msg.rate, config.maxRate);
        postToGuard({
          ns: GUARD_NS,
          dir: "content->guard",
          type: "setRate",
          rate: config.targetRate,
        });
        reportSoon();
        break;
      }
      case "media:playPause": {
        const m = mainMedia();
        if (!m) break;
        try {
          // isolated 调实例方法不受 world 隔离影响
          if (m.paused) {
            const p = m.play();
            if (p) void p.catch(() => {});
          } else {
            m.pause();
          }
        } catch {
          /* 鸭子类型/自动播放策略异常 */
        }
        reportSoon();
        break;
      }
      case "media:config": {
        if (msg.config && typeof msg.config === "object") applyConfig(msg.config);
        break;
      }
    }
  });
} catch {
  /* context invalidated */
}

// ============ SPA 导航（开发文档 §5.2.1 onNavigate） ============

let lastHref = location.href;
let navPollTimer: number | null = null;

function onNavigate(): void {
  lastHref = location.href;
  prune();
  scan();
  if (shouldEnforce()) applyTarget();
  sendState(true);
}

window.addEventListener("popstate", () => onNavigate());
// YouTube SPA 导航完成事件是普通 DOM 事件，ISOLATED world 能收到；document/window 双挂保险
window.addEventListener("yt-navigate-finish", () => onNavigate());
document.addEventListener("yt-navigate-finish", () => onNavigate());

/** 500ms 轮询 location.href 兜底（pushState 无事件可听）；仅曾有媒体的页面开启 */
function startNavPoll(): void {
  if (navPollTimer !== null) return;
  navPollTimer = window.setInterval(() => {
    if (location.href !== lastHref) onNavigate();
  }, NAV_POLL_MS);
}

// ============ 启动 ============

// document_start 时 DOM 可能尚空：先扫一遍（SSR 首屏），MutationObserver 接力，
// DOMContentLoaded 再兜底全扫 + 上报初始基准（无媒体也报一帧 hasMedia=false）
scan();
document.addEventListener("DOMContentLoaded", () => {
  scan();
  sendState(true);
});
