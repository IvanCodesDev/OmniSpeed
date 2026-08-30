/**
 * 站点选择器实测校准探针。
 *
 * 八站适配器里的 videoSelector / isLive / isAd 都是「最佳努力值」，站点一改版就可能失准。
 * 本工具把真实页面拉起来，直接回答三个问题：
 *   1. 适配器命中谁、videoSelector 有没有选中真 <video>
 *   2. isLive / isAd 在这一页返回什么（点播页应双 false）
 *   3. `--sel` 指定的选择器到底匹配到了哪些元素，各自的可见性指标是什么
 *      —— 排查「元素常驻 DOM 但其实是空壳」这类误判必需的证据
 *
 * 谓词跑的是注入页面的**真实站点注册表**（打包 src/sites），与发布版零漂移。
 *
 * 用法：
 *   node apps/extension/test/dom-probe.mjs <url> [--via=<先过一遍的页>] [--wait=20]
 *        [--watch=<秒>] [--sel="<css>"] [--js="<expr>"] [--keep]
 *   --via    先访问这一页再进目标页。部分站点直接深链进播放页不会起播（无来路/无会话），
 *            先过一遍首页才还原真实用户路径。
 *   --watch  进页后按 2s 一次持续采样谓词与视频状态，用来抓「前贴片广告 → 正片」的跃迁
 *            （PRD §7.6 的干预时机就挂在这个跃迁上，单次快照看不出来）。
 * 例（复现优酷 isAd 误判的取证）：
 *   node apps/extension/test/dom-probe.mjs "https://v.youku.com/v_show/id_XXX.html" --sel="[class*='kui-advertise'], .advertise-layer"
 */

import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  attachFirstPage,
  bundleSitesIife,
  connectBrowser,
  launchChrome,
  loadUnpackedExtension,
  makeSitesInjector,
  sleep,
} from "./cdp-harness.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const extRoot = join(here, "..");
const PORT = 9445; // 与回归驱动的 9444 错开，两者可同时跑

/** 页面身份 + video 清单 + guard 标记 */
function identityExpr() {
  return `(() => {
    const info = (v) => {
      const r = v.getBoundingClientRect();
      return { w: Math.round(r.width), h: Math.round(r.height), rate: v.playbackRate,
        paused: v.paused, ready: v.readyState, src: (v.currentSrc || "").slice(0, 70) };
    };
    return {
      href: location.href.slice(0, 160), host: location.host, title: document.title.slice(0, 80),
      guard: window.__omnispeed_rate_guard_installed__ === true,
      videos: [...document.querySelectorAll("video")].map(info),
      frames: window.frames.length,
    };
  })()`;
}

/** 选择器命中详情：每个匹配元素的几何 + 计算样式，用于判定「是否真的可见、是否空壳」 */
function inspectExpr(sel) {
  return `(() => {
    let els;
    try { els = [...document.querySelectorAll(${JSON.stringify(sel)})]; }
    catch (e) { return { err: e.message }; }
    return {
      count: els.length,
      items: els.slice(0, 12).map((el) => {
        const r = el.getBoundingClientRect();
        const cs = getComputedStyle(el);
        return {
          tag: el.tagName.toLowerCase(),
          cls: (el.className && el.className.toString ? el.className.toString() : "").slice(0, 90),
          w: Math.round(r.width), h: Math.round(r.height),
          display: cs.display, visibility: cs.visibility, opacity: cs.opacity,
          offsetParent: el.offsetParent !== null,
          clientRects: el.getClientRects().length,
          childCount: el.childElementCount,
          text: (el.textContent || "").trim().replace(/\\s+/g, " ").slice(0, 50),
        };
      }),
    };
  })()`;
}

/** 适配器身份 + 选择器命中 + 真实谓词（全部调注入页面的真实注册表） */
function adapterExpr() {
  return `(() => {
    const a = __omniSites.adapterFor(location.host);
    const out = { id: a.id, videoSelector: a.videoSelector ?? null, selector: null, live: null, ad: null, err: null };
    if (a.videoSelector) {
      const hit = document.querySelector(a.videoSelector);
      out.selector = hit
        ? { hit: true, tag: hit.tagName.toLowerCase(), isVideo: hit instanceof HTMLVideoElement }
        : { hit: false };
    }
    try { out.live = a.isLive ? a.isLive() : null; } catch (e) { out.err = "isLive:" + e.message; }
    try { out.ad = a.isAd ? a.isAd() : null; } catch (e) { out.err = (out.err ? out.err + ";" : "") + "isAd:" + e.message; }
    return out;
  })()`;
}

async function main() {
  const args = process.argv.slice(2);
  const url = args.find((a) => !a.startsWith("--"));
  if (!url) throw new Error("用法：node dom-probe.mjs <url> [--wait=20] [--sel=<css>] [--js=<expr>] [--keep]");
  const arg = (name, dflt) => {
    const a = args.find((x) => x.startsWith(`--${name}=`));
    return a ? a.slice(name.length + 3) : dflt;
  };
  const waitSec = Number(arg("wait", "20"));
  const watchSec = Number(arg("watch", "0"));
  const via = arg("via", null);
  const sel = arg("sel", null);
  const js = arg("js", null);
  const keep = args.includes("--keep");

  const dist = join(extRoot, "dist");
  const sitesSource = await bundleSitesIife(extRoot);
  const { chrome, cleanup } = await launchChrome({ port: PORT, keep });
  console.log(`[探针] Chrome PID=${chrome.pid}`);
  try {
    const cdp = await connectBrowser(PORT);
    if (existsSync(join(dist, "manifest.json"))) {
      const { extId, swOk } = await loadUnpackedExtension(cdp, dist);
      console.log(`[探针] 扩展 ${extId} SW=${swOk ? "在线" : "未见"}`);
    } else {
      console.log("[探针] dist/ 未构建，跳过装扩展（纯 DOM 校准仍可用，guard 标记会是 false）");
    }

    const { sessionId, evalIn, navigate } = await attachFirstPage(cdp);
    const ensureSites = makeSitesInjector(evalIn, sitesSource);
    if (via) {
      const v = await navigate(via);
      console.log(`[探针] 先过 ${via} → ${v.ok ? (v.loaded ? "load 完成" : "load 超时") : `失败:${v.errorText}`}`);
      await sleep(5000);
    }
    const nav = await navigate(url);
    console.log(`[探针] 导航 ${nav.ok ? (nav.loaded ? "load 完成" : "load 超时（继续）") : `失败:${nav.errorText}`}`);

    // 轮询等播放器就绪（SPA 播放器远晚于 load 事件）
    let ident = null;
    const deadline = Date.now() + waitSec * 1000;
    while (Date.now() < deadline) {
      await sleep(1500);
      try {
        ident = await evalIn(sessionId, identityExpr());
      } catch {
        continue;
      }
      if (ident.videos.length > 0) break;
    }
    if (!ident) throw new Error("页面一直无法求值");

    console.log("\n=== 页面 ===");
    console.log(JSON.stringify(ident, null, 2));

    console.log("\n=== 适配器与谓词 ===");
    if (await ensureSites(sessionId)) {
      console.log(JSON.stringify(await evalIn(sessionId, adapterExpr()), null, 2));
    } else {
      console.log("站点注册表注入失败");
    }

    if (sel) {
      console.log(`\n=== 选择器命中详情 === ${sel}`);
      console.log(JSON.stringify(await evalIn(sessionId, inspectExpr(sel)), null, 2));
    }
    if (watchSec > 0) {
      console.log(`\n=== 持续采样 ${watchSec}s（2s 一次）===`);
      const until = Date.now() + watchSec * 1000;
      const t0 = Date.now();
      while (Date.now() < until) {
        await sleep(2000);
        try {
          await ensureSites(sessionId);
          const a = await evalIn(sessionId, adapterExpr());
          const v = await evalIn(sessionId, identityExpr());
          const shells = sel ? await evalIn(sessionId, inspectExpr(sel)) : null;
          const active = shells?.items?.filter((i) => i.childCount > 0 || i.text) ?? [];
          console.log(
            `  +${String(Math.round((Date.now() - t0) / 1000)).padStart(3)}s ad=${a.ad} live=${a.live} ` +
              `videos=${v.videos.length} ${v.videos.map((x) => `${x.w}x${x.h}/rate${x.rate}/${x.paused ? "暂停" : "播放"}/ready${x.ready}`).join(" ")}` +
              (shells ? ` | 广告层 ${shells.count} 个，其中有内容 ${active.length} 个` : ""),
          );
        } catch (e) {
          console.log(`  · 采样失败: ${e.message.slice(0, 80)}`);
        }
      }
    }
    if (js) {
      console.log(`\n=== 自定义求值 ===`);
      console.log(JSON.stringify(await evalIn(sessionId, `(() => (${js}))()`), null, 2));
    }
    if (keep) console.log(`\n[探针] --keep：Chrome 保留（PID=${chrome.pid}），人工复查后请自行关闭`);
  } finally {
    await cleanup();
  }
}

main().catch((e) => {
  console.error(`[探针] 致命错误: ${e.message}`);
  process.exit(1);
});
