import type { SiteAdapter } from "./types";

/**
 * 哔哩哔哩（开发文档 §5.2.1 表格）：
 * - 播放器 bpx-player，标准 <video>；切 P / 自动连播 / 拖进度后站点复位 → 由 Rate-Guard
 *   锁定恢复（通用机制，无需特判）。
 * - 预加载会产生多个 video 实例 → videoSelector 定位 bpx 播放区取主实例。
 * - 部分场景使用自研 WASM 播放器 <bwp-video>（开发文档 Spike #4）：playbackRate 兼容性
 *   未知，写入侧（rate-guard nativeWrite）一律 try/catch 吞异常，这里只负责把它纳入定位。
 */
export const bilibili: SiteAdapter = {
  id: "bilibili",
  match: /(^|\.)bilibili\.com$/,
  videoSelector: ".bpx-player-video-area video, .bpx-player-video-wrap video, bwp-video",
  // B 站直播为独立子域 live.bilibili.com（PRD §7.6：直播识别后置灰倍速并提示）
  isLive: () => /(^|\.)live\.bilibili\.com$/.test(location.host),
};
