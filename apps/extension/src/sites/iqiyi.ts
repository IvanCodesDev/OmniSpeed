import type { SiteAdapter } from "./types";
import { adLayerActive } from "./dom";

/**
 * 爱奇艺（开发文档 §5.2.1 表格）：
 * - 自研播放器 iqp，标准 <video>；`.iqp-player` 为播放器容器。
 * - 广告层 `.iqp-player-videoad` 真的挂出广告内容时暂停干预；空壳常驻不算
 *   （判据见 dom.ts::adLayerActive，优酷是该误判的实测样本，本站同构写法一并收敛）。
 * - 直播走 live.iqiyi.com 子域。DOM 钩子随站点改版可能变动，需真机标定后维护。
 */
export const iqiyi: SiteAdapter = {
  id: "iqiyi",
  match: /(^|\.)iqiyi\.com$/,
  videoSelector: ".iqp-player video",
  isLive: () => /^live\./.test(location.host),
  isAd: () => adLayerActive(".iqp-player-videoad"),
};
