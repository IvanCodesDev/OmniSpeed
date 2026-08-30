/**
 * OmniSpeed 八站网页端真机回归驱动（M4.7）。
 *
 * 做什么：拉起一个隔离的 Chrome 实例（独立 user-data-dir + 静音 + 独立调试口 9444），
 * 经 CDP `Extensions.loadUnpacked` 装入 dist/ 解包扩展（Chrome 137 起品牌版移除了
 * --load-extension，只能走 CDP + --enable-unsafe-extension-debugging），随后逐站导航
 * 真实页面并检查：
 *   1. rate-guard 注入标记（MAIN world / document_start / all_frames）
 *   2. adapterFor(host) 是否命中预期适配器
 *   3. videoSelector 是否定位到真实 <video>（缺省选择器的站点走通用兜底规则）
 *   4. isLive / isAd 在点播页应为 false
 *   5. 倍速链路：postMessage setRate 2.5 → 回读；config 锁定后站点无手势写 1 被拦；
 *      setRate 16 突破站点上限；终了恢复 1×
 * 2–4 全部调用**注入页面的真实站点注册表**（esbuild 打包 src/sites，见 bundleSitesIife），
 * 跑的就是发布版同一份适配器代码，零镜像漂移。
 * 视频不在顶层 frame 时：先递归同源 iframe，再枚举 OOPIF（type=iframe 目标）逐个探测。
 *
 * 找不到视频时的候选来源（SPA 站点卡片常常不是 <a>，只认锚点会一无所获）：
 *   ① 页面锚点  ② 页面 HTML 正则（SSR 数据块里的视频 id）——每访问一页都会重新采集。
 *
 * 用法：node apps/extension/test/site-regression.mjs [--sites=bilibili,qq] [--entry=<url>] [--keep]
 *   --sites  只跑指定站点（逗号分隔 id）
 *   --entry  指定入口页，只在 --sites 选中单站时生效。站点把自动化挡在门外时
 *            （见下「封堵」），维护者可以手工丢一个真实视频链接进来复测。
 *   --keep   跑完保留 Chrome 实例与临时 profile（人工复查用）
 * 结果：控制台摘要 + 仓库根 .shots/site-regression-<时间戳>.json
 *
 * 「封堵」：站点把全新 profile 强制导去下载页/登录墙时（西瓜 www.ixigua.com → /app/），
 * 网页端根本没有可测页面，这跟「适配器写错了」是两回事，记为 blocked 并在汇总里单列，
 * 不混进失败数——否则每次回归都挂着一条永远修不好的红叉，久了就没人看结果了。
 */

import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  attachFirstPage,
  bundleSitesIife,
  connectBrowser,
  EXPECTED_EXT_ID,
  launchChrome,
  loadUnpackedExtension,
  makeSitesInjector,
  sleep,
} from "./cdp-harness.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const extRoot = join(here, "..");
const repoRoot = join(extRoot, "..", "..");
const DIST = join(extRoot, "dist");
const PORT = 9444;
const PROBE_TIMEOUT_MS = 15_000; // 冷加载的 SPA 播放器就绪可能要 10s+，轮询等待
const HARVEST_TIMEOUT_MS = 8_000; // 首页 feed 客户端渲染，链接可能晚于 load 事件出现
const SITE_BUDGET_MS = 210_000;
const MAX_CANDIDATES = 5; // 每站最多访问几个页面（入口 + 采集来的候选）

// ---------------------------------------------------------------------------
// 站点表
//
// entries：按顺序尝试的入口页。优先给**链接富集的列表页**而不是写死某个视频 id
// （上一轮 bilibili 就栽在写死的 BV1GJ411x7h7 被删上）。
// anchors：锚点采集，sel 选锚点、re 匹配 pathname+search。
// html：HTML 正则采集，对付卡片不是 <a> 的 SPA；tpl 走 String.replace 语义（$& / $1）。
// ---------------------------------------------------------------------------
const SITES = [
  {
    id: "bilibili",
    entries: ["https://www.bilibili.com/v/popular/all", "https://www.bilibili.com"],
    anchors: { sel: 'a[href*="/video/BV"]', re: "/video/BV[0-9A-Za-z]+" },
    html: { re: "/video/BV[0-9A-Za-z]{8,12}", tpl: "https://www.bilibili.com$&" },
  },
  {
    id: "douyin",
    entries: ["https://www.douyin.com/discover", "https://www.douyin.com"],
    anchors: { sel: 'a[href*="/video/"]', re: "/video/\\d{15,}" },
    html: { re: "/video/\\d{15,}", tpl: "https://www.douyin.com$&" },
  },
  { id: "youtube", entries: ["https://www.youtube.com/watch?v=jNQXAC9IVRw"] },
  {
    id: "qq",
    entries: ["https://v.qq.com/channel/movie", "https://v.qq.com"],
    anchors: { sel: 'a[href*="/x/cover/"], a[href*="/x/page/"]', re: "/x/(cover|page)/" },
    html: { re: "/x/cover/[A-Za-z0-9]+/[A-Za-z0-9]+\\.html", tpl: "https://v.qq.com$&" },
  },
  {
    id: "iqiyi",
    entries: ["https://www.iqiyi.com"],
    anchors: { sel: 'a[href*="/v_"]', re: "/v_[0-9a-z]+" },
    html: { re: "/v_[0-9a-z]{8,}\\.html", tpl: "https://www.iqiyi.com$&" },
  },
  {
    id: "youku",
    entries: ["https://www.youku.com"],
    anchors: { sel: 'a[href*="v_show"], a[href*="video?vid="]', re: "(v_show/id_|video\\?vid=)" },
    html: { re: "/v_show/id_[A-Za-z0-9=]+\\.html", tpl: "https://v.youku.com$&" },
  },
  {
    id: "ixigua",
    entries: ["https://www.ixigua.com/channel/hot", "https://www.ixigua.com"],
    anchors: { sel: "a[href]", re: "^/\\d{15,}" },
    html: { re: 'href="/(\\d{15,})"', tpl: "https://www.ixigua.com/$1" },
    // 全新 profile 访问任意路径都被换成 App 下载页（23:3x 实测：页面只有安装包和备案
    // 信息，没有任何回到网页版的链接）→ 网页端无从自动化，记 blocked 而非 FAIL。
    // 手上有真实视频链接时：--sites=ixigua --entry=https://www.ixigua.com/<id>
    gate: { re: "^https://www\\.ixigua\\.com/app/", why: "被换成 App 下载页" },
  },
  {
    id: "kuaishou",
    entries: ["https://www.kuaishou.com"],
    anchors: { sel: 'a[href*="/short-video/"]', re: "/short-video/" },
  },
];

// ---------------------------------------------------------------------------
// 页面内脚本模板
// ---------------------------------------------------------------------------

/** 基础探测：guard 标记 / 顶层 + 同源 iframe 的 video 清单 / 页面身份 */
function probeExpr() {
  return `(() => {
    const info = (v) => {
      const r = v.getBoundingClientRect();
      return { w: Math.round(r.width), h: Math.round(r.height), rate: v.playbackRate,
        paused: v.paused, ready: v.readyState, t: Math.round(v.currentTime * 10) / 10,
        src: (v.currentSrc || "").slice(0, 80) };
    };
    const collect = (win, acc, depth) => {
      let doc = null;
      try { doc = win.document; } catch { return acc; }
      if (!doc) return acc;
      for (const v of doc.querySelectorAll("video")) acc.push(v);
      if (depth < 3) for (let i = 0; i < win.frames.length; i++) collect(win.frames[i], acc, depth + 1);
      return acc;
    };
    const all = collect(window, [], 0);
    const infos = all.map(info);
    return {
      href: location.href.slice(0, 140),
      host: location.host,
      title: document.title.slice(0, 60),
      guard: window.__omnispeed_rate_guard_installed__ === true,
      videoCount: all.length,
      // 0×0 的隐藏 <video> 占位不算「这页能测」：爱奇艺首页就挂着一个，
      // 只看 videoCount 会让驱动把首页当成播放页，然后理直气壮地报「选择器没命中」。
      usableCount: infos.filter((v) => v.w > 0 && v.h > 0).length,
      videos: infos.slice(0, 4),
      hasBwp: !!document.querySelector("bwp-video"),
    };
  })()`;
}

/** 适配器身份 + videoSelector 命中 + 真实 isLive/isAd（全部调注入页面的真实注册表） */
function adapterExpr() {
  return `(() => {
    const a = __omniSites.adapterFor(location.host);
    const out = { id: a.id, videoSelector: a.videoSelector ?? null, selector: null,
      live: null, ad: null, err: null };
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

/**
 * 倍速链路测试：setRate 回读 / 锁定拦截 / 16× / 恢复 1×。
 *
 * 回读一律走**有界轮询**而不是「睡固定毫秒再读一次」：媒体还在加载时
 * （readyState 低）浏览器会在 load 过程中把 playbackRate 复位成 defaultPlaybackRate，
 * 未开锁定的 setRate 本来就不保证当场就稳（那正是 rateLock 存在的理由，
 * 而锁定行为由下一步单独验）。固定睡眠只会把这变成随机红叉。
 * 轮询到超时仍不对 = 真的没生效，照样判失败；同时记录实际等到第几毫秒，
 * 「慢但正确」和「压根没生效」在结果里分得开。
 */
function rateExpr(videoSelector) {
  return `(async () => {
    const sel = ${JSON.stringify(videoSelector ?? null)};
    const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
    const post = (m) => window.postMessage(Object.assign({ ns: "__omnispeed__", dir: "content->guard" }, m), "*");
    const pick = () => {
      const hit = sel ? document.querySelector(sel) : null;
      if (hit instanceof HTMLVideoElement) return hit;
      let best = null, bestScore = -1;
      for (const v of document.querySelectorAll("video")) {
        const r = v.getBoundingClientRect();
        const s = (!v.paused && !v.ended ? 1e12 : 0) + r.width * r.height;
        if (s > bestScore) { bestScore = s; best = v; }
      }
      return best;
    };
    const v = pick();
    if (!v) return { ok: false, reason: "no-video" };
    const read = () => { try { return v.playbackRate; } catch { return null; } };
    const waited = {};
    const settle = async (key, want, budgetMs) => {
      const t0 = Date.now();
      let last = read();
      while (Date.now() - t0 < budgetMs) {
        if (last !== null && Math.abs(last - want) < 1e-6) break;
        await sleep(120);
        last = read();
      }
      waited[key] = Date.now() - t0;
      return last;
    };

    post({ type: "setRate", rate: 2.5 });
    const r25 = await settle("r25", 2.5, 3000);
    post({ type: "config", config: { targetRate: 2.5, rateLock: true, maxRate: 16 } });
    await sleep(150);
    try { v.playbackRate = 1; } catch {}   // 模拟站点脚本无手势复位
    const rLock = await settle("rLock", 2.5, 2000);
    post({ type: "setRate", rate: 16 });
    const r16 = await settle("r16", 16, 2000);
    post({ type: "config", config: { targetRate: null, rateLock: false, maxRate: 16 } });
    post({ type: "setRate", rate: 1 });
    const r1 = await settle("r1", 1, 1500);
    return { ok: true, r25, rLock, r16, r1, waited, pickedPaused: v.paused, ready: v.readyState };
  })()`;
}

/** 候选采集：锚点 + HTML 正则（SPA 卡片常不是 <a>，只认锚点会一无所获） */
function harvestExpr(anchors, html) {
  return `(() => {
    const seen = new Set();
    const out = [];
    const push = (raw) => {
      let u;
      try { u = new URL(raw, location.href); } catch { return; }
      if (!/^https?:$/.test(u.protocol)) return;
      const key = u.origin + u.pathname + u.search;
      if (seen.has(key)) return;
      seen.add(key);
      out.push(u.href);
    };
    const anchors = ${JSON.stringify(anchors ?? null)};
    if (anchors) {
      const re = new RegExp(anchors.re);
      for (const a of document.querySelectorAll(anchors.sel)) {
        const href = a.getAttribute("href");
        if (!href) continue;
        let u;
        try { u = new URL(href, location.href); } catch { continue; }
        if (!re.test(u.pathname + u.search)) continue;
        push(u.href);
        if (out.length >= 8) break;
      }
    }
    const html = ${JSON.stringify(html ?? null)};
    if (html && out.length < 8) {
      const src = document.documentElement.innerHTML;
      const re = new RegExp(html.re, "g");
      const one = new RegExp(html.re);
      let m;
      while ((m = re.exec(src)) !== null && out.length < 8) {
        push(m[0].replace(one, html.tpl));
        if (re.lastIndex === m.index) re.lastIndex++;
      }
    }
    return out;
  })()`;
}

// ---------------------------------------------------------------------------
// 主流程
// ---------------------------------------------------------------------------
async function main() {
  const args = process.argv.slice(2);
  const onlyArg = args.find((a) => a.startsWith("--sites="));
  const only = onlyArg ? onlyArg.slice("--sites=".length).split(",").filter(Boolean) : null;
  const entryArg = args.find((a) => a.startsWith("--entry="));
  const entryOverride = entryArg ? entryArg.slice("--entry=".length) : null;
  if (entryOverride && only?.length !== 1) throw new Error("--entry 需要配合 --sites=<单个站点> 使用");
  const keep = args.includes("--keep");

  const sitesSource = await bundleSitesIife(extRoot);
  const { chrome, profile, cleanup } = await launchChrome({ port: PORT, keep });
  console.log(`[驱动] Chrome 已拉起 PID=${chrome.pid}，profile=${profile}`);
  process.on("SIGINT", async () => {
    await cleanup();
    process.exit(130);
  });

  let results = [];
  try {
    const cdp = await connectBrowser(PORT);
    const { extId, swOk } = await loadUnpackedExtension(cdp, DIST);
    console.log(
      `[驱动] 扩展已装载 id=${extId}` +
        `${extId === EXPECTED_EXT_ID ? "（与 NM 清单一致）" : "（⚠️ 与 NM 清单不一致，NM 链路将不通）"}` +
        ` SW：${swOk ? "在线" : "未见（继续，content 脚本不依赖 SW 存活）"}`,
    );

    const { sessionId, evalIn, navigate } = await attachFirstPage(cdp);
    const ensureSites = makeSitesInjector(evalIn, sitesSource);

    /** 在指定会话上跑全部检查（顶层或 OOPIF）；适配器由该 frame 自己的 host 决定 */
    const runChecks = async (sid, probe, rec) => {
      if (!(await ensureSites(sid))) {
        rec.notes.push("站点注册表注入失败，跳过适配器检查");
        rec.pass = false;
        return;
      }
      const a = await evalIn(sid, adapterExpr());
      rec.host = probe.host;
      rec.adapterActual = a.id;
      rec.adapterOk = a.id === rec.site;
      rec.guard = probe.guard;
      rec.videoCount = probe.videoCount;
      rec.videos = probe.videos;
      rec.usableCount = probe.usableCount;
      rec.selector = a.selector;
      // 无 videoSelector 的站点走通用兜底：页面上有可见 video 即可被主媒体规则选中
      rec.selectorOk = a.videoSelector ? !!a.selector?.hit : probe.usableCount > 0;
      rec.predicates = { live: a.live, ad: a.ad, err: a.err };

      try {
        rec.rate = await evalIn(sid, rateExpr(a.videoSelector), { awaitPromise: true, timeoutMs: 30_000 });
      } catch (e) {
        rec.rate = { ok: false, reason: e.message };
      }

      const r = rec.rate ?? {};
      rec.rateOk = r.ok === true && r.r25 === 2.5 && r.rLock === 2.5 && r.r16 === 16;
      rec.restored = r.ok === true && r.r1 === 1;
      rec.pass =
        rec.guard && rec.adapterOk && rec.selectorOk && rec.rateOk &&
        rec.predicates.live !== true && rec.predicates.ad !== true;
    };

    /** OOPIF 扫描：顶层与同源 frame 都没视频时，逐个附着跨进程 iframe 目标探测 */
    const scanOopifs = async (rec) => {
      const { targetInfos } = await cdp.send("Target.getTargets");
      const frames = targetInfos.filter((t) => t.type === "iframe" && /^https?:/.test(t.url));
      for (const f of frames.slice(0, 6)) {
        try {
          const { sessionId: fs } = await cdp.send("Target.attachToTarget", { targetId: f.targetId, flatten: true });
          const probe = await evalIn(fs, probeExpr());
          if (probe?.usableCount > 0) return { sid: fs, probe, frameUrl: f.url };
          await cdp.send("Target.detachFromTarget", { sessionId: fs }).catch(() => {});
        } catch (e) {
          rec.notes.push(`OOPIF 探测失败(${f.url.slice(0, 60)}): ${e.message.slice(0, 80)}`);
        }
      }
      return null;
    };

    const runSite = async (site) => {
      const rec = { site: site.id, notes: [], tried: [] };
      const queue = entryOverride ? [entryOverride] : [...site.entries];
      const gate = site.gate ? new RegExp(site.gate.re) : null;
      let gated = 0;
      const budget = Date.now() + SITE_BUDGET_MS;
      let done = false;

      // 站点预算走协作式检查而非 Promise.race：race 不会真的停下循环，
      // 超时站点会继续在共享的 sessionId 上导航，把下一站的页面掀翻。
      while (queue.length > 0 && !done && rec.tried.length < MAX_CANDIDATES) {
        if (Date.now() > budget) {
          rec.notes.push(`站点预算超时（${SITE_BUDGET_MS / 1000}s），停止尝试`);
          break;
        }
        const url = queue.shift();
        if (rec.tried.includes(url)) continue;
        rec.tried.push(url);

        const nav = await navigate(url);
        if (!nav.ok) {
          rec.notes.push(`导航失败(${nav.errorText}): ${url.slice(0, 70)}`);
          continue;
        }
        if (!nav.loaded) rec.notes.push(`load 事件超时（SPA/慢页，继续探测）: ${url.slice(0, 70)}`);

        // 轮询等视频出现（冷加载播放器就绪可能远晚于 load 事件）
        let probe = null;
        const deadline = Date.now() + PROBE_TIMEOUT_MS;
        while (Date.now() < deadline) {
          await sleep(1200);
          try {
            probe = await evalIn(sessionId, probeExpr());
          } catch {
            continue;
          }
          if (probe.usableCount > 0) break;
        }
        if (!probe) {
          rec.notes.push(`探测失败（页面脚本一直异常）: ${url.slice(0, 70)}`);
          continue;
        }
        rec.finalUrl = probe.href;
        rec.title = probe.title;
        rec.notes.push(
          `试 ${url.slice(0, 70)} → 「${probe.title}」videos=${probe.videoCount}` +
            `（可见 ${probe.usableCount}）guard=${probe.guard}`,
        );

        if (gate && gate.test(probe.href) && !entryOverride) {
          gated++;
          rec.notes.push(`被站点封堵（${site.gate.why}）：${probe.href.slice(0, 70)}`);
          continue; // 门外的页面既没视频也没链接，采集它没有意义
        }

        if (probe.usableCount > 0) {
          await runChecks(sessionId, probe, rec);
          rec.context = "top";
          done = true;
          break;
        }

        const oopif = await scanOopifs(rec);
        if (oopif) {
          rec.notes.push(`视频位于 OOPIF: ${oopif.frameUrl.slice(0, 80)}`);
          await runChecks(oopif.sid, oopif.probe, rec);
          rec.context = "oopif";
          done = true;
          break;
        }

        // 这一页没视频 → 就从这一页采集候选。
        // 每访问一页都采一次：上一轮把采集做成整站一次性，B 站入口恰好是已删视频的
        // 错误页，采集扑空后 fallback 首页（链接最富集的页面）就再也没被采集过。
        if (site.anchors || site.html) {
          let links = [];
          const hDeadline = Date.now() + HARVEST_TIMEOUT_MS;
          while (Date.now() < hDeadline && links.length === 0) {
            try {
              links = await evalIn(sessionId, harvestExpr(site.anchors, site.html));
            } catch (e) {
              rec.notes.push(`采集失败: ${e.message.slice(0, 100)}`);
              break;
            }
            if (links.length === 0) await sleep(1500);
          }
          const fresh = links.filter((u) => !rec.tried.includes(u) && !queue.includes(u));
          rec.notes.push(`从本页采集到 ${links.length} 条候选（新增 ${Math.min(fresh.length, 3)} 条入队）`);
          queue.push(...fresh.slice(0, 3));
        }
      }

      rec.pass = rec.pass ?? false;
      rec.reachable = rec.tried.length > 0 && !!rec.finalUrl;
      if (!done) {
        // 每一页都被挡在门外 = 站点不给网页端，不是我们的适配器坏了
        rec.blocked = gated > 0 && gated === rec.tried.length;
        rec.notes.push(rec.blocked ? "全部入口都被站点封堵，网页端无可测页面" : "未找到任何可测视频");
      }
      return rec;
    };

    const list = only ? SITES.filter((s) => only.includes(s.id)) : SITES;
    for (const site of list) {
      console.log(`\n[站点] ${site.id} 开始…`);
      const rec = await runSite(site);
      results.push(rec);
      console.log(
        `[站点] ${site.id} ${rec.pass ? "PASS" : rec.blocked ? "BLOCKED" : "FAIL"} ` +
          `guard=${rec.guard ?? "-"} adapter=${rec.adapterActual ?? "-"} ` +
          `sel=${rec.selectorOk ?? "-"} videos=${rec.usableCount ?? 0}/${rec.videoCount ?? 0} ` +
          `rate=${rec.rate ? JSON.stringify({ r25: rec.rate.r25, lock: rec.rate.rLock, r16: rec.rate.r16, r1: rec.rate.r1 }) : "-"} ` +
          `live=${rec.predicates?.live ?? "-"} ad=${rec.predicates?.ad ?? "-"}`,
      );
      for (const n of rec.notes) console.log(`        · ${n}`);
    }
  } finally {
    const shots = join(repoRoot, ".shots");
    await mkdir(shots, { recursive: true });
    const stamp = new Date().toISOString().replace(/[:.]/g, "-");
    const outFile = join(shots, `site-regression-${stamp}.json`);
    await writeFile(outFile, JSON.stringify(results, null, 2), "utf8");
    console.log(`\n[驱动] 结果已写入 ${outFile}`);
    const passed = results.filter((r) => r.pass).length;
    const blocked = results.filter((r) => r.blocked).length;
    console.log(
      `[驱动] 汇总：${passed}/${results.length} 通过` +
        (blocked > 0 ? `，${blocked} 站被站点封堵（网页端无可测页面，不计为失败）` : "") +
        `，失败 ${results.length - passed - blocked}`,
    );
    await cleanup();
  }
}

main().catch((e) => {
  console.error(`[驱动] 致命错误: ${e.message}`);
  process.exit(1);
});
