/**
 * rate-guard（MAIN world）与 content（ISOLATED world）之间的 window.postMessage 内部协议。
 *
 * 为什么需要它（开发文档 §5.2）：Chromium 中 MAIN / ISOLATED 两个 world 各有独立的
 * JS 原型链——isolated 侧直接写 playbackRate 走的是它自己 world 的原生 setter，
 * 不会经过 MAIN world 的代理；因此所有倍速写入统一由 rate-guard 执行，
 * isolated 侧只通过 postMessage 请求。
 *
 * 安全性说明：同源 window.postMessage 站点脚本可伪造，完全防冒用做不到；这里只做
 * 基本校验（e.source === window + ns/dir 匹配 + 字段类型检查）。伪造者的能力上限
 * 只是改本页自己的倍速（站点本来就能做到），风险可接受。
 */

export const GUARD_NS = "__omnispeed__";

/** Chromium 内核允许的 playbackRate 下限（1/16，越界抛 NotSupportedError，开发文档 §2.1） */
export const MIN_RATE = 0.0625;

/** Chromium 内核硬上限（开发文档 §7.8） */
export const KERNEL_MAX_RATE = 16;

/** 收到桌面核心配置前的默认上限 */
export const DEFAULT_MAX_RATE = 16;

/** 手势时间窗（ms）：写入前该窗口内有 pointerdown/keydown → 视为用户主动调速（开发文档 §5.2） */
export const GESTURE_WINDOW_MS = 500;

/** rate-guard 关心的配置子集（preservesPitch 由 isolated 直接设实例属性，不进 MAIN world） */
export interface GuardConfig {
  targetRate: number | null;
  rateLock: boolean;
  maxRate: number;
}

/** isolated → MAIN world */
export type ContentToGuard =
  /** 写入 rate 并同步 targetRate（应用到本 frame 内全部媒体元素） */
  | { ns: typeof GUARD_NS; dir: "content->guard"; type: "setRate"; rate: number }
  /** targetRate / rateLock / maxRate 更新 */
  | { ns: typeof GUARD_NS; dir: "content->guard"; type: "config"; config: GuardConfig };

/** MAIN world → isolated */
export type GuardToContent =
  /** 手势时间窗内的站点写入已放行 → isolated 上报 media:userRate 并同步为新目标 */
  { ns: typeof GUARD_NS; dir: "guard->content"; type: "userRate"; rate: number };

/**
 * 所有写入统一 clamp 到 [1/16, maxRate]；maxRate 本身收口到内核硬上限 16，
 * 避免配置异常时触发 NotSupportedError（开发文档 §7.8）。
 */
export function clampRate(rate: number, maxRate: number): number {
  if (!Number.isFinite(rate)) return 1;
  const upper = Math.min(Math.max(maxRate, MIN_RATE), KERNEL_MAX_RATE);
  return Math.min(Math.max(rate, MIN_RATE), upper);
}
