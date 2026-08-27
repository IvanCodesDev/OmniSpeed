/**
 * OmniSpeed 扩展消息协议（冻结契约，改动需同步 Rust 侧 nm_bridge.rs）。
 *
 * 两段链路（开发文档 §5.2 / §5.3）：
 *   内容脚本 ⟷ Service Worker：chrome.runtime 消息（本文件 ContentToSw / SwToContent）
 *   Service Worker ⟷ 桌面核心：Native Messaging JSON 帧（本文件 ExtToApp / AppToExt）
 *
 * Native Messaging 传输层为 Chrome 标准帧格式（4 字节小端长度前缀 + UTF-8 JSON），
 * 由浏览器与桌面侧中继（omnispeed.exe --nm-host）各自处理，SW 只面对 JSON 对象。
 * 中继会在首帧前向桌面核心注入 { type:"hostInfo", browser:"msedge.exe" }（由中继
 * 检测父进程得出，扩展侧不需要也不应该发送该帧）。
 */

/** 当前活动标签页的媒体状态（内容脚本采集，SW 汇总上报桌面核心） */
export interface MediaState {
  hasMedia: boolean;
  /** 主媒体元素的真实 playbackRate */
  rate: number;
  /** 页面标题（控制页「当前媒体」显示用） */
  title: string;
  /** 站点 host，如 "www.bilibili.com"（规则匹配 + 按站记忆的键） */
  host: string;
  /** 直播中 → 桌面侧禁用倍速并提示（PRD §7.6） */
  isLive: boolean;
  /** 平台广告时段 → 暂停干预，正片恢复（PRD §7.6） */
  adPlaying: boolean;
  /** 缓冲前沿余量（秒），高倍速警示用；未知为 null（开发文档 §7.8） */
  bufferedAhead: number | null;
  /** 命中的站点适配器 id："generic" | "bilibili" | "douyin" | "youtube" */
  adapter: string;
}

/** 桌面核心下发的运行配置（SW 缓存并广播给所有内容脚本） */
export interface RateConfig {
  /**
   * 会话目标倍速：Rate-Guard 锁定恢复与短视频流跟随的基准；
   * null = 尚无目标（不强制改写页面倍速）
   */
  targetRate: number | null;
  /** 倍速锁定：拦截站点脚本对 playbackRate 的复位（开发文档 §7.4） */
  rateLock: boolean;
  /** 内核硬上限收口（Chromium 16×，越界抛 NotSupportedError） */
  maxRate: number;
  /** 变速不变调（HTMLMediaElement.preservesPitch） */
  preservesPitch: boolean;
}

// ---------- 内容脚本 → Service Worker ----------

export type ContentToSw =
  /** 媒体状态变化（节流上报；hasMedia=false 表示本帧不再有可控媒体） */
  | { type: "media:state"; state: MediaState }
  /** 用户在站点 UI 主动调速（手势时间窗内的写入，开发文档 §5.2）：同步为新目标 */
  | { type: "media:userRate"; rate: number };

// ---------- Service Worker → 内容脚本 ----------

export type SwToContent =
  | { type: "media:setRate"; rate: number }
  | { type: "media:playPause" }
  | { type: "media:config"; config: RateConfig };

// ---------- Service Worker → 桌面核心（NM 帧） ----------

export type ExtToApp =
  /** 连接建立后的第一帧 */
  | { type: "hello"; version: string }
  /** 活动标签页媒体状态（null = 活动标签页无可控媒体）；用户主动调速也走这里 */
  | { type: "media"; state: MediaState | null };

// ---------- 桌面核心 → Service Worker（NM 帧） ----------

export type AppToExt =
  | { type: "setRate"; rate: number }
  | { type: "playPause" }
  | { type: "config"; config: RateConfig };
