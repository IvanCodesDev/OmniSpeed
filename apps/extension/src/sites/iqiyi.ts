import type { SiteAdapter } from "./types";

/**
 * 爱奇艺（开发文档 §5.2.1 表格）：
 * - 自研播放器 iqp，标准 <video>；`.iqp-player` 为播放器容器。
 * - 广告层 `.iqp-player-videoad`（前贴片倒计时期间可见）→ 广告时段暂停干预。
 * - 直播走 live.iqiyi.com 子域。DOM 钩子随站点改版可能变动，需真机标定后维护。
 */
export const iqiyi: SiteAdapter = {
  id: "iqiyi",
  match: /(^|\.)iqiyi\.com$/,
  videoSelector: ".iqp-player video",
  isLive: () => /^live\./.test(location.host),
  isAd: () => {
    const ad = document.querySelector<HTMLElement>(".iqp-player-videoad");
    return !!ad && ad.offsetParent !== null;
  },
};
