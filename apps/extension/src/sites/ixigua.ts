import type { SiteAdapter } from "./types";

/**
 * 西瓜视频（开发文档 §5.2.1 表格）：
 * - 播放器 xgplayer；沉浸流/推荐流滑动换视频产生新 <video> 元素 → 由 content.ts 的
 *   MutationObserver 通用机制秒级接管 + 会话倍速跟随（同抖音策略），这里无需特判。
 * - 不指定 videoSelector：信息流页面同屏多个播放器实例，按「正在播放 > 可见面积
 *   最大 > 最新出现」的通用规则选主媒体更稳。
 * - 直播：live 子域或 /live 路径。DOM 钩子随站点改版可能变动，需真机标定后维护。
 */
export const ixigua: SiteAdapter = {
  id: "ixigua",
  match: /(^|\.)ixigua\.com$/,
  isLive: () => /^live\./.test(location.host) || location.pathname.startsWith("/live"),
};
