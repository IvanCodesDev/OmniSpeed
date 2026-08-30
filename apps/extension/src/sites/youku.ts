import type { SiteAdapter } from "./types";
import { adLayerActive } from "./dom";

/**
 * 优酷（开发文档 §5.2.1 表格）：
 * - 自研 H5 播放器（kui 系），标准 <video>；`#ykPlayer` 为经典播放器容器，
 *   新版页面容器为 `.kui-player`，两者并列定位取主实例。
 * - 前贴片广告层 `.kui-advertise-*`（旧版 `.advertise-layer`）真的挂出广告内容时
 *   暂停干预（PRD §7.6）；空壳常驻不算——判据见 dom.ts::adLayerActive，
 *   本站正是该误判的真机样本（22:5x 八站回归实测）。
 * - 直播走 live.youku.com / vku.youku.com 子域。DOM 钩子随站点改版可能变动，
 *   需真机标定后维护，规则保持薄、可社区贡献。
 */
export const youku: SiteAdapter = {
  id: "youku",
  match: /(^|\.)youku\.com$/,
  videoSelector: "#ykPlayer video, .kui-player video",
  isLive: () => /^(live|vku)\./.test(location.host),
  isAd: () => adLayerActive("[class*='kui-advertise'], .advertise-layer"),
};
