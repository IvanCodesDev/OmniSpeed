/**
 * 站点适配器共用的 DOM 判定助手。
 *
 * 只放「多个站点都要用、且判错会影响功能」的通用判据，规则本身仍保持薄、可社区贡献。
 */

/** 元素是否真的画在屏幕上：有非零尺寸的布局盒，且没被 visibility/opacity 抹掉 */
function rendered(el: Element): boolean {
  const r = el.getBoundingClientRect();
  if (r.width <= 0 || r.height <= 0) return false; // display:none 时全为 0，一并挡掉
  const cs = getComputedStyle(el);
  return cs.visibility !== "hidden" && cs.opacity !== "0";
}

/**
 * 广告层是否真的在放广告。
 *
 * 不能只看「广告容器存在且可见」，也不能退一步看「容器里有没有子节点」——
 * 各家播放器都把广告层常驻在 DOM 里，无广告时留下的是**看不见的空壳**。
 * 优酷 v.youku.com 点播页两级实测：
 *   - 播放器未加载时：`.advertise-layer` 为 853×488 满尺寸、`display:block` /
 *     `visibility:visible` / `opacity:1`、`offsetParent` 非空，但 0 子节点；
 *   - 播放器加载后：层里多出唯一一个子节点，是
 *     `<iframe style="...z-index:-1;visibility:hidden">` 的隐形占位（层上标着 `data-spm="adfree"`）。
 * 于是 `offsetParent !== null` 恒为真，`childElementCount > 0` 也只是把误判从
 * 「永远误判」缩小成「播放器一加载就误判」，两者都不成立。
 *
 * 误判的代价不是少显示一个角标：content.ts 的 `shouldEnforce()` 在广告时段一律
 * 停止干预，于是倍速跟随、锁定恢复、导航恢复全部失效（PRD §7.6 的「广告不干预」
 * 被放大成「整站不干预」），同时持续向桌面端上报 adPlaying=true。
 *
 * 判据：广告层自身要**画出来**，并且**装着真的画出来的东西**（非空文本，或任一
 * 渲染可见的子孙元素——真广告的播放器/倒计时/跳过按钮/可见 iframe 都满足）。
 * 遍历**全部**匹配项：空壳与真广告层可能并存，命中任意一个真的即算广告时段。
 *
 * 已知边界：1×1 的可见埋点像素理论上能骗过判据。实测各站埋点都是
 * `display:none` / 0 尺寸 / `visibility:hidden`，因此不额外加尺寸阈值——
 * 与其塞一个拍脑袋的magic number，不如把边界如实写在这里。
 */
export function adLayerActive(selector: string): boolean {
  let els: NodeListOf<HTMLElement>;
  try {
    els = document.querySelectorAll<HTMLElement>(selector);
  } catch {
    return false;
  }
  for (const el of els) {
    if (!rendered(el)) continue;
    if ((el.textContent ?? "").trim() !== "") return true;
    // 只看前 50 个子孙：广告层本身很小，够用又不会在 1.5s 心跳里拖慢页面
    const kids = el.querySelectorAll<HTMLElement>("*");
    const limit = Math.min(kids.length, 50);
    for (let i = 0; i < limit; i++) {
      if (rendered(kids[i])) return true;
    }
  }
  return false;
}
