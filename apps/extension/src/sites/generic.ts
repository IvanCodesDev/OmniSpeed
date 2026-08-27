import type { SiteAdapter } from "./types";

/**
 * 通用 HTML5 兜底（开发文档 §5.2.1 表格「其他站点」行）：
 * 接管全部 HTMLMediaElement，仍享受 Rate-Guard 锁定；无直播/广告检测。
 */
export const generic: SiteAdapter = {
  id: "generic",
  match: /(?:)/,
};
