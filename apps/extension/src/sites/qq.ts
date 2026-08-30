import type { SiteAdapter } from "./types";

/**
 * 腾讯视频（开发文档 §5.2.1 表格）：
 * - 自研播放器 txp，标准 <video>；`.txp_video_container` 为稳定的视频容器类名。
 * - 前贴片/中插广告期间播放器根节点挂出广告层 `.txp_ad_root`（隐藏时仍在 DOM，
 *   用 offsetParent 判断可见性）→ 广告时段暂停干预（PRD §7.6）。
 * - 直播走独立子域 live.v.qq.com。DOM 钩子随站点改版可能变动，需真机标定后维护。
 */
export const qq: SiteAdapter = {
  id: "qq",
  match: /(^|\.)v\.qq\.com$/,
  videoSelector: ".txp_video_container video",
  isLive: () => /^live\./.test(location.host),
  isAd: () => {
    const ad = document.querySelector<HTMLElement>(".txp_ad_root, .txp_ad");
    return !!ad && ad.offsetParent !== null;
  },
};
