/**
 * OmniSpeed Rate-Guard —— 倍速锁定核心。
 * MAIN world / document_start / all_frames（manifest 固定，勿改）。
 *
 * 原理（开发文档 §5.2 / §7.4 双保险）：
 *  ① 抢在任何站点脚本之前（document_start 保证）用 Object.defineProperty 代理
 *     HTMLMediaElement.prototype.playbackRate ——站点的复位写入在 setter 里被拦截，
 *     无闪速；这是「5× 不被 B 站/抖音改回 1×」的根本保障。
 *  ② ratechange 事后校正兜底——覆盖换源后内核默认值等不经过 setter 的路径。
 *
 * world 边界：站点脚本跑在 MAIN world，只有在 MAIN world 打补丁才拦得住站点写入；
 * isolated 侧（content.ts）通过 window.postMessage 请求写入（协议见 guard-protocol.ts）。
 */

import {
  DEFAULT_MAX_RATE,
  GESTURE_WINDOW_MS,
  GUARD_NS,
  clampRate,
  type ContentToGuard,
  type GuardConfig,
  type GuardToContent,
} from "./guard-protocol";

const INSTALL_FLAG = "__omnispeed_rate_guard_installed__";

function install(): void {
  const proto = HTMLMediaElement.prototype;
  const desc = Object.getOwnPropertyDescriptor(proto, "playbackRate");
  if (!desc || !desc.get || !desc.set || !desc.configurable) return;
  const nativeGet = desc.get;
  const nativeSet = desc.set;

  const state: GuardConfig = {
    // targetRate=null 的安全默认：收到配置前不拦截任何写入
    targetRate: null,
    rateLock: false,
    maxRate: DEFAULT_MAX_RATE,
  };

  // ---- 用户手势识别（开发文档 §5.2）----
  // 捕获阶段监听，站点 stopPropagation 也拦不住时间戳记录。
  // 桌面全局快捷键由 OS 层捕获、不产生页面 keydown，不会误入手势窗口。
  let lastGestureAt = -Infinity;
  const markGesture = (): void => {
    lastGestureAt = Date.now();
  };
  window.addEventListener("pointerdown", markGesture, true);
  window.addEventListener("keydown", markGesture, true);
  const hasRecentGesture = (): boolean => Date.now() - lastGestureAt <= GESTURE_WINDOW_MS;

  // ---- 内部写入（带内部标记，直接走原生 setter，绕过代理防循环）----
  let internalWrite = false;
  function nativeWrite(el: HTMLMediaElement, rate: number): void {
    internalWrite = true;
    try {
      nativeSet.call(el, clampRate(rate, state.maxRate));
    } catch {
      // 越界抛 NotSupportedError；bwp-video（B 站自研 WASM 播放器）对 playbackRate
      // 的兼容性未知（开发文档 Spike #4）——统一吞掉，失败不影响其他元素。
    } finally {
      internalWrite = false;
    }
  }

  function postToContent(msg: GuardToContent): void {
    window.postMessage(msg, "*");
  }

  // ---- ① prototype setter 代理：写入决策 ----
  Object.defineProperty(proto, "playbackRate", {
    configurable: true,
    enumerable: desc.enumerable,
    get(this: HTMLMediaElement): number {
      // getter 透传原生
      return nativeGet.call(this) as number;
    },
    set(this: HTMLMediaElement, value: number) {
      // 决策 ①：自己的写入（内部标记）直接放行。
      // 常规内部写入走 nativeWrite 不经过这里；此分支兜底子类 super 赋值等间接路径。
      if (internalWrite) {
        try {
          nativeSet.call(this, value);
        } catch {
          /* 同 nativeWrite */
        }
        return;
      }

      const requested = Number(value);

      // 决策 ②：站点写入 + 500ms 内有用户手势 → 用户在站点 UI 主动调速：
      // 放行 + 同步为新目标 + 通知 isolated（→ 上报 media:userRate）。
      if (hasRecentGesture()) {
        const applied = clampRate(requested, state.maxRate);
        try {
          nativeSet.call(this, applied);
        } catch {
          return;
        }
        state.targetRate = applied;
        postToContent({ ns: GUARD_NS, dir: "guard->content", type: "userRate", rate: applied });
        return;
      }

      // 决策 ③：站点写入 + 无手势 + 锁定 + 有目标 → 复位行为，拦截。
      // 站点的值被丢弃；若真实值已漂移则改写回 targetRate。
      if (state.rateLock && state.targetRate !== null) {
        const target = clampRate(state.targetRate, state.maxRate);
        try {
          if ((nativeGet.call(this) as number) !== target) {
            nativeSet.call(this, target);
          }
        } catch {
          /* 同 nativeWrite */
        }
        return;
      }

      // 决策 ④：其余情况放行（仍 clamp 防越界抛错）。
      try {
        nativeSet.call(this, clampRate(requested, state.maxRate));
      } catch {
        /* 同 nativeWrite */
      }
    },
  });

  // ---- ② ratechange 事后校正兜底（开发文档 §7.4 双保险之二）----
  // 媒体事件不冒泡但有捕获阶段，window 捕获监听可收到全部后代元素的 ratechange。
  // 防循环：校正写入走 nativeWrite（内部标记），且只在真实值 ≠ clamp 后目标时动手——
  // 校正后二者相等，由校正自身触发的 ratechange 不会再次写入。
  window.addEventListener(
    "ratechange",
    (e: Event) => {
      const el = e.target;
      if (!(el instanceof HTMLMediaElement)) return;
      if (!state.rateLock || state.targetRate === null) return;
      if (hasRecentGesture()) return; // 手势窗口内的变化尊重用户
      const target = clampRate(state.targetRate, state.maxRate);
      let actual: number;
      try {
        actual = nativeGet.call(el) as number;
      } catch {
        return;
      }
      if (actual !== target) nativeWrite(el, target);
    },
    true,
  );

  // ---- isolated → guard 指令 ----
  window.addEventListener("message", (e: MessageEvent) => {
    // 基本校验：同窗口 + 命名空间 + 方向。同源站点脚本仍可伪造（见 guard-protocol.ts 头注）。
    if (e.source !== window) return;
    const data = e.data as ContentToGuard | null | undefined;
    if (!data || typeof data !== "object" || data.ns !== GUARD_NS || data.dir !== "content->guard") {
      return;
    }
    switch (data.type) {
      case "setRate": {
        if (typeof data.rate !== "number") return;
        state.targetRate = clampRate(data.rate, state.maxRate);
        applyToAllMedia(state.targetRate);
        break;
      }
      case "config": {
        const c = data.config;
        if (!c || typeof c !== "object") return;
        if (typeof c.maxRate === "number" && Number.isFinite(c.maxRate)) state.maxRate = c.maxRate;
        state.rateLock = c.rateLock === true;
        state.targetRate =
          typeof c.targetRate === "number" ? clampRate(c.targetRate, state.maxRate) : null;
        break;
      }
    }
  });

  /**
   * 应用到本 frame 内全部媒体元素（isolated 侧决定何时请求，如新媒体出现/切集恢复；
   * 已处于目标值的元素写同值为幂等 no-op）。
   * bwp-video 一并尝试：非 HTMLMediaElement 时原生 setter 抛 TypeError，由 try/catch
   * 吞掉（开发文档 Spike #4）。局限：不含 shadow DOM 与未挂载 DOM 的元素。
   */
  function applyToAllMedia(rate: number): void {
    const els = document.querySelectorAll("video, audio, bwp-video");
    for (const el of els) {
      nativeWrite(el as HTMLMediaElement, rate);
    }
  }
}

// 防重复注入（如 history 缓存恢复等边界场景）
const w = window as unknown as Record<string, unknown>;
if (!w[INSTALL_FLAG]) {
  w[INSTALL_FLAG] = true;
  try {
    install();
  } catch {
    // 补丁失败时保持页面原生行为，不影响站点脚本运行
  }
}
