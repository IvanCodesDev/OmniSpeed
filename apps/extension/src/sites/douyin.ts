import type { SiteAdapter } from "./types";

/**
 * 抖音网页版（开发文档 §5.2.1 表格）：
 * - 播放器 xgplayer；虚拟列表滑动换视频会产生新 <video> 元素 → 由 content.ts 的
 *   MutationObserver 通用机制秒级接管 + 会话倍速跟随，这里无需特判。
 * - 直播：live.douyin.com 子域或 /live 路径；页内直播容器的 DOM 钩子（data-e2e 标记）
 *   随站点改版可能变动，需真机标定后维护，规则保持薄、可社区贡献。
 */
export const douyin: SiteAdapter = {
  id: "douyin",
  match: /(^|\.)douyin\.com$/,
  isLive: () =>
    /^live\./.test(location.host) ||
    location.pathname.startsWith("/live") ||
    !!document.querySelector("[data-e2e='live-room'], [data-e2e='living-container']"),
};
