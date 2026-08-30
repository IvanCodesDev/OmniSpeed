/**
 * `src/sites/dom.ts::adLayerActive` 单测（Node 内置 test runner + 类型剥离，零依赖）。
 *
 * 跑法：npm run test -w @omnispeed/extension
 *
 * 锁死的是八站回归实测到的真 bug：优酷点播页 `.advertise-layer` 无广告时是个
 * **看不见的空壳**，而且分两级形态——
 *   ① 播放器未加载：满播放器尺寸、display/visibility/opacity 全正常、0 子节点；
 *   ② 播放器加载后：多出唯一一个 `visibility:hidden; z-index:-1` 的占位 iframe。
 * 旧判据 `offsetParent !== null` 在 ① 就恒为真；只补「有子节点」在 ② 仍然误判。
 * 误判会让 content.ts::shouldEnforce() 全程停止干预（PRD §7.6 的「广告不干预」
 * 被放大成「整站不干预」），所以两级形态都要有测试钉住。
 */

import test from "node:test";
import assert from "node:assert/strict";
import { adLayerActive } from "../src/sites/dom.ts";

const SEL = ".ad-layer";

/** 造一个够用的假元素：只实现 adLayerActive 真正读到的那几个面 */
function el({ w = 853, h = 488, visibility = "visible", opacity = "1", text = "", kids = [] } = {}) {
  return {
    getBoundingClientRect: () => ({ width: w, height: h }),
    querySelectorAll: () => kids,
    textContent: text,
    __style: { visibility, opacity },
  };
}

/** 用假 document/getComputedStyle 跑一段，跑完还原全局 */
function withDom(matches, fn) {
  const prevDoc = globalThis.document;
  const prevGcs = globalThis.getComputedStyle;
  globalThis.document = {
    querySelectorAll: (sel) => (typeof matches === "function" ? matches(sel) : matches),
  };
  globalThis.getComputedStyle = (e) => e.__style;
  try {
    return fn();
  } finally {
    globalThis.document = prevDoc;
    globalThis.getComputedStyle = prevGcs;
  }
}

test("空壳广告层不算广告时段 · 形态①：满尺寸可见但没有内容", () => {
  const shell = el({ w: 853, h: 488, text: "", kids: [] });
  assert.equal(withDom([shell], () => adLayerActive(SEL)), false);
});

test("空壳广告层不算广告时段 · 形态②：只挂着 visibility:hidden 的占位 iframe", () => {
  // 优酷实测原样：<iframe style="...z-index:-1;width:100%;height:100%;visibility:hidden">
  const placeholder = el({ w: 853, h: 488, visibility: "hidden" });
  const shell = el({ kids: [placeholder] });
  assert.equal(withDom([shell], () => adLayerActive(SEL)), false);
});

test("广告层挂出真的画得出来的内容才算广告时段", () => {
  assert.equal(withDom([el({ kids: [el({ w: 853, h: 488 })] })], () => adLayerActive(SEL)), true);
  assert.equal(withDom([el({ text: "广告 15 秒" })], () => adLayerActive(SEL)), true);
});

test("0 尺寸 / opacity:0 的子孙不算内容（埋点、预留节点）", () => {
  assert.equal(withDom([el({ kids: [el({ w: 0, h: 0 })] })], () => adLayerActive(SEL)), false);
  assert.equal(withDom([el({ kids: [el({ opacity: "0" })] })], () => adLayerActive(SEL)), false);
});

test("空壳与真广告层并存时命中真的那个（不能只看 querySelector 的第一个）", () => {
  const shell = el({ kids: [] });
  const real = el({ kids: [el()] });
  assert.equal(withDom([shell, real], () => adLayerActive(SEL)), true);
});

test("广告层自身未渲染 / 被隐藏时不算，哪怕里面有内容", () => {
  const content = [el()];
  assert.equal(withDom([el({ w: 0, h: 0, kids: content })], () => adLayerActive(SEL)), false);
  assert.equal(withDom([el({ visibility: "hidden", kids: content })], () => adLayerActive(SEL)), false);
  assert.equal(withDom([el({ opacity: "0", kids: content })], () => adLayerActive(SEL)), false);
});

test("position:fixed 的真广告层不漏判（offsetParent 恒 null，故不用它做判据）", () => {
  const fixedAd = el({ kids: [el()] });
  assert.equal("offsetParent" in fixedAd, false);
  assert.equal(withDom([fixedAd], () => adLayerActive(SEL)), true);
});

test("子孙很多时只看前 50 个，不在 1.5s 心跳里遍历整棵树", () => {
  const many = Array.from({ length: 80 }, (_, i) =>
    // 只有第 60 个是可见的：超出扫描窗口，应判 false
    el({ w: i === 59 ? 100 : 0, h: i === 59 ? 100 : 0 }),
  );
  assert.equal(withDom([el({ kids: many })], () => adLayerActive(SEL)), false);
  many[10] = el({ w: 100, h: 100 });
  assert.equal(withDom([el({ kids: many })], () => adLayerActive(SEL)), true);
});

test("无匹配元素 / 选择器非法都返回 false，不抛异常", () => {
  assert.equal(withDom([], () => adLayerActive(SEL)), false);
  assert.equal(
    withDom(
      () => {
        throw new SyntaxError("非法选择器");
      },
      () => adLayerActive("!!!"),
    ),
    false,
  );
});
