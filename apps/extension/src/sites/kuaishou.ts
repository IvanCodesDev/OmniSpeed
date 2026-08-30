import type { SiteAdapter } from "./types";

/**
 * 快手网页版（开发文档 §5.2.1 表格）：
 * - 自研播放器壳 + 标准 <video>；瀑布流/短视频滑动换视频走 content.ts 通用接管
 *   与会话倍速跟随（同抖音策略），不指定 videoSelector（同屏多实例场景通用规则更稳）。
 * - 直播为独立子域 live.kuaishou.com；主站直播间路径 /live 兜底。
 *   站点大量使用 CSS Modules 哈希类名，不依赖具体类名钩子，需真机标定后维护。
 */
export const kuaishou: SiteAdapter = {
  id: "kuaishou",
  match: /(^|\.)kuaishou\.com$/,
  isLive: () => /^live\./.test(location.host) || location.pathname.startsWith("/live"),
};
