import type { SiteAdapter } from "./types";
import { adLayerActive } from "./dom";

/**
 * 腾讯视频（开发文档 §5.2.1 表格）：
 * - 自研播放器 txp，标准 <video>；视频容器类名是 **`.txp_videos_container`（复数）**
 *   —— 真机实测（23:2x，`/x/cover/mzc002005etgzma/q410283pzms.html`）祖先链为
 *   `video < .txp_videos_container < ._mod_thumbplayer_container_#player-component`；
 *   旧规则写的单数 `.txp_video_container` 从来没命中过，只是一直被通用兜底掩盖。
 *   单数形式保留在后面，兼容可能仍在用旧标记的页面。
 * - 前贴片/中插广告期间播放器根节点挂出广告层 `.txp_ad_root`（无广告时空壳仍留在
 *   DOM 里）→ 只有真的挂出广告内容才暂停干预（判据见 dom.ts::adLayerActive，
 *   PRD §7.6）。
 * - 直播走独立子域 live.v.qq.com。DOM 钩子随站点改版可能变动，需真机标定后维护。
 */
export const qq: SiteAdapter = {
  id: "qq",
  match: /(^|\.)v\.qq\.com$/,
  videoSelector: ".txp_videos_container video, .txp_video_container video",
  isLive: () => /^live\./.test(location.host),
  isAd: () => adLayerActive(".txp_ad_root, .txp_ad"),
};
