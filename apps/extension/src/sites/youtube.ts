import type { SiteAdapter } from "./types";

/**
 * YouTube（开发文档 §5.2.1 表格）：
 * - 自研播放器；官方 UI 上限 2×、底层不限。
 * - SPA 导航（yt-navigate-finish）后的倍速恢复由 content.ts 通用导航机制处理。
 * - Shorts 与普通视频共用 .html5-main-video，走通用短视频流跟随策略。
 */
export const youtube: SiteAdapter = {
  id: "youtube",
  match: /(^|\.)youtube\.com$/,
  videoSelector: "video.html5-main-video",
  // 直播徽标 .ytp-live-badge 在点播页也存在但不可见/带 disabled 属性 → 双重判断
  isLive: () => {
    const badge = document.querySelector<HTMLElement>(".ytp-live-badge");
    return !!badge && badge.offsetParent !== null && !badge.hasAttribute("disabled");
  },
  // 广告时段：播放器容器带 ad-showing（PRD §7.6：广告期间不干预，正片恢复）
  isAd: () => !!document.querySelector(".html5-video-player.ad-showing"),
};
