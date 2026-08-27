/**
 * 站点适配器声明式接口（开发文档 §5.2.1）。
 * 规则保持薄、可社区贡献：只做「主播放器定位 + 直播/广告识别」，
 * 倍速锁定、短视频流跟随、SPA 导航恢复等由 content.ts / rate-guard.ts 通用机制承担。
 */
export interface SiteAdapter {
  /** 上报用 id（协议 MediaState.adapter）："generic" | "bilibili" | "douyin" | "youtube" */
  id: string;
  /** 对 location.host 匹配 */
  match: RegExp;
  /** 主播放器定位；缺省则按「正在播放 > 可见面积最大 > 最新出现」通用规则选主媒体 */
  videoSelector?: string;
  /** 直播识别 → 桌面侧禁用倍速并提示（PRD §7.6） */
  isLive?(): boolean;
  /** 平台广告时段 → 暂停干预，正片恢复后再应用目标倍速（PRD §7.6） */
  isAd?(): boolean;
}
