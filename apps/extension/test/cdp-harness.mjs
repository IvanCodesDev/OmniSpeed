/**
 * 真机页面测试的共用 CDP 骨架：拉起隔离 Chrome、装解包扩展、附着页面、导航与求值。
 *
 * 被 site-regression.mjs（八站回归）与 dom-probe.mjs（选择器实测校准）共用。
 * Chrome 137 起品牌版移除了 `--load-extension`，装扩展只能走
 * `--enable-unsafe-extension-debugging` + CDP `Extensions.loadUnpacked`。
 */

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { build } from "esbuild";

export const CHROME = "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe";
export const EXPECTED_EXT_ID = "ejpnpjbhmgckjfdednjgfhdpobencmpb";

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** 浏览器级单 WS + flatten sessions 的轻客户端（Node ≥22 自带 WebSocket，零依赖） */
export class Cdp {
  constructor(ws) {
    this.ws = ws;
    this.seq = 0;
    this.pending = new Map();
    this.waiters = new Set();
    ws.addEventListener("message", (e) => {
      let msg;
      try {
        msg = JSON.parse(e.data);
      } catch {
        return;
      }
      if (msg.id !== undefined) {
        const p = this.pending.get(msg.id);
        if (!p) return;
        this.pending.delete(msg.id);
        if (msg.error) p.reject(new Error(`${p.method}: ${msg.error.message}`));
        else p.resolve(msg.result);
        return;
      }
      for (const w of [...this.waiters]) {
        if (w.method === msg.method && (w.sessionId === undefined || w.sessionId === msg.sessionId)) {
          this.waiters.delete(w);
          w.resolve(msg.params);
        }
      }
    });
    ws.addEventListener("close", () => {
      for (const [, p] of this.pending) p.reject(new Error("WS 已关闭"));
      this.pending.clear();
    });
  }

  static async connect(wsUrl) {
    const ws = new WebSocket(wsUrl);
    await new Promise((resolve, reject) => {
      ws.addEventListener("open", resolve, { once: true });
      ws.addEventListener("error", () => reject(new Error(`WS 连接失败: ${wsUrl}`)), { once: true });
    });
    return new Cdp(ws);
  }

  send(method, params = {}, sessionId, timeoutMs = 20_000) {
    const id = ++this.seq;
    const payload = { id, method, params };
    if (sessionId) payload.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} 超时（${timeoutMs}ms）`));
      }, timeoutMs);
      this.pending.set(id, {
        method,
        resolve: (v) => {
          clearTimeout(timer);
          resolve(v);
        },
        reject: (e) => {
          clearTimeout(timer);
          reject(e);
        },
      });
      this.ws.send(JSON.stringify(payload));
    });
  }

  waitEvent(method, { sessionId, timeoutMs = 30_000 } = {}) {
    return new Promise((resolve, reject) => {
      const w = { method, sessionId, resolve: undefined };
      const timer = setTimeout(() => {
        this.waiters.delete(w);
        reject(new Error(`等待 ${method} 超时（${timeoutMs}ms）`));
      }, timeoutMs);
      w.resolve = (params) => {
        clearTimeout(timer);
        resolve(params);
      };
      this.waiters.add(w);
    });
  }

  close() {
    try {
      this.ws.close();
    } catch {
      /* 忽略 */
    }
  }
}

/**
 * 拉起隔离 Chrome（独立 profile + 静音 + 指定调试口），返回句柄与 cleanup。
 * 不碰用户日常浏览器：profile 是 temp 目录，端口自选。
 */
export async function launchChrome({ port, keep = false, windowSize = "1360,900" } = {}) {
  if (!existsSync(CHROME)) throw new Error(`未找到 Chrome：${CHROME}`);
  const profile = await mkdtemp(join(tmpdir(), "omnispeed-cdp-"));
  const chrome = spawn(
    CHROME,
    [
      `--user-data-dir=${profile}`,
      `--remote-debugging-port=${port}`,
      "--enable-unsafe-extension-debugging",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-search-engine-choice-screen",
      "--mute-audio",
      "--autoplay-policy=no-user-gesture-required",
      `--window-size=${windowSize}`,
      "--lang=zh-CN",
      "about:blank",
    ],
    { stdio: "ignore" },
  );

  const cleanup = async () => {
    if (keep) return;
    try {
      spawn("taskkill", ["/PID", String(chrome.pid), "/T", "/F"], { stdio: "ignore" });
    } catch {
      /* 忽略 */
    }
    await sleep(1500);
    for (let i = 0; i < 3; i++) {
      try {
        await rm(profile, { recursive: true, force: true });
        break;
      } catch {
        await sleep(1000);
      }
    }
  };

  return { chrome, profile, cleanup };
}

/** 轮询调试口直到 /json/version 可用，返回浏览器级 CDP 连接 */
export async function connectBrowser(port, { attempts = 40 } = {}) {
  let version = null;
  for (let i = 0; i < attempts && !version; i++) {
    try {
      version = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
    } catch {
      await sleep(500);
    }
  }
  if (!version) throw new Error(`调试口 ${port} 未上线`);
  return Cdp.connect(version.webSocketDebuggerUrl);
}

/** 装载解包扩展并等其 service worker 起来；返回 { extId, swOk } */
export async function loadUnpackedExtension(cdp, distDir) {
  if (!existsSync(join(distDir, "manifest.json"))) throw new Error(`dist/ 未构建：先跑 node build.mjs`);
  let extId;
  try {
    ({ id: extId } = await cdp.send("Extensions.loadUnpacked", { path: distDir }));
  } catch (e) {
    throw new Error(
      `Extensions.loadUnpacked 失败（${e.message}）——确认 Chrome 启动带了 --enable-unsafe-extension-debugging`,
    );
  }
  let swOk = false;
  for (let i = 0; i < 20 && !swOk; i++) {
    const { targetInfos } = await cdp.send("Target.getTargets");
    swOk = targetInfos.some((t) => t.type === "service_worker" && t.url.includes(extId));
    if (!swOk) await sleep(500);
  }
  return { extId, swOk };
}

/** 附着到初始页面目标，开 Page/Runtime 域，返回 sessionId 与页面操作助手 */
export async function attachFirstPage(cdp) {
  const { targetInfos } = await cdp.send("Target.getTargets");
  const page = targetInfos.find((t) => t.type === "page");
  if (!page) throw new Error("找不到初始页面目标");
  const { sessionId } = await cdp.send("Target.attachToTarget", { targetId: page.targetId, flatten: true });
  await cdp.send("Page.enable", {}, sessionId);
  await cdp.send("Runtime.enable", {}, sessionId);
  return { sessionId, ...pageOps(cdp, sessionId) };
}

/** 绑定到某个会话的导航/求值助手（顶层页或 OOPIF 通用） */
export function pageOps(cdp, sessionId) {
  const evalIn = async (sid, expression, { awaitPromise = false, timeoutMs = 25_000 } = {}) => {
    const r = await cdp.send(
      "Runtime.evaluate",
      { expression, returnByValue: true, awaitPromise, userGesture: false },
      sid,
      timeoutMs,
    );
    if (r.exceptionDetails) {
      throw new Error(
        `页面脚本异常: ${r.exceptionDetails.exception?.description?.slice(0, 200) ?? r.exceptionDetails.text}`,
      );
    }
    return r.result?.value;
  };

  const navigate = async (url) => {
    const loaded = cdp.waitEvent("Page.loadEventFired", { sessionId, timeoutMs: 30_000 });
    const nav = await cdp.send("Page.navigate", { url }, sessionId, 30_000);
    if (nav.errorText) {
      loaded.catch(() => {});
      return { ok: false, errorText: nav.errorText };
    }
    const fired = await loaded.then(() => true).catch(() => false);
    return { ok: true, loaded: fired };
  };

  return { evalIn, navigate, evalHere: (expr, opts) => evalIn(sessionId, expr, opts) };
}

/**
 * 把真实的 `src/sites` 注册表打成浏览器 IIFE 源码，供注入页面后直接调用。
 *
 * 为什么不是把 `adapter.isAd.toString()` 序列化进页面：那样只搬得走函数字面量本身，
 * 适配器一旦引用共享助手（如 dom.ts::adLayerActive）注入进去就是 ReferenceError，
 * 于是「规则不许抽公共函数」这种莫名其妙的约束会反向绑架产品代码。
 * 整体打包注入则跑的就是发布版同一份代码，零漂移也零约束。
 */
export async function bundleSitesIife(extRoot) {
  const out = await build({
    entryPoints: [join(extRoot, "src", "sites", "index.ts")],
    bundle: true,
    format: "iife",
    globalName: "__omniSites",
    write: false,
    target: ["chrome120"],
    charset: "utf8",
    logLevel: "silent",
  });
  return out.outputFiles[0].text;
}

/** 返回 ensureSites(sid)：确保该会话当前文档里有 `__omniSites`（导航后会丢，故每次用前检查） */
export function makeSitesInjector(evalIn, source) {
  return async (sid) => {
    const present = await evalIn(sid, `typeof __omniSites !== "undefined"`);
    if (present) return true;
    await evalIn(sid, `${source}\n;typeof __omniSites !== "undefined"`);
    return evalIn(sid, `typeof __omniSites !== "undefined"`);
  };
}
