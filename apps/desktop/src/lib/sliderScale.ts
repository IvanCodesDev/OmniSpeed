import { SLIDER_MIN } from "../store";

/**
 * 滑块分段线性刻度：1×–2× 占据更长滑轨便于微调（PRD §7.1）。
 * 轨道位置 t ∈ [0,1]：前 18% 对应 0.25×–1×，中间 42% 对应 1×–2×，其余对应 2×–上限。
 */
export function makeSliderScale(max: number) {
  const anchors: Array<[number, number]> = [
    [0, SLIDER_MIN],
    [0.18, 1],
    [0.6, 2],
    [1, max],
  ];

  const rateToT = (rate: number) => {
    const r = Math.min(max, Math.max(SLIDER_MIN, rate));
    for (let i = 1; i < anchors.length; i++) {
      const [t0, r0] = anchors[i - 1];
      const [t1, r1] = anchors[i];
      if (r <= r1) return t0 + ((r - r0) / (r1 - r0)) * (t1 - t0);
    }
    return 1;
  };

  const tToRate = (t: number) => {
    const tt = Math.min(1, Math.max(0, t));
    for (let i = 1; i < anchors.length; i++) {
      const [t0, r0] = anchors[i - 1];
      const [t1, r1] = anchors[i];
      if (tt <= t1) return r0 + ((tt - t0) / (t1 - t0)) * (r1 - r0);
    }
    return max;
  };

  return { rateToT, tToRate };
}
