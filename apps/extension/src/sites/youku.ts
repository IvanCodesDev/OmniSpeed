import type { SiteAdapter } from "./types";

/**
 * 优酷（开发文档 §5.2.1 表格）：
 * - 自研 H5 播放器（kui 系），标准 <video>；`#ykPlayer` 为经典播放器容器，
 *   新版页面容器为 `.kui-player`，两者并列定位取主实例。
 * - 前贴片广告层 `.kui-advertise-*`（旧版 `.advertise-layer`）可见时暂停干预（PRD §7.6）。
 * - 直播走 live.youku.com / vku.youku.com 子域。DOM 钩子随站点改版可能变动，
 *   需真机标定后维护，规则保持薄、可社区贡献。
 */
export const youku: SiteAdapter = {
  id: "youku",
  match: /(^|\.)youku\.com$/,
  videoSelector: "#ykPlayer video, .kui-player video",
  isLive: () => /^(live|vku)\./.test(location.host),
  isAd: () => {
    const ad = document.querySelector<HTMLElement>(
      "[class*='kui-advertise'], .advertise-layer",
    );
    return !!ad && ad.offsetParent !== null;
  },
};
