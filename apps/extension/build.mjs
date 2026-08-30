/**
 * OmniSpeed 扩展构建脚本：node build.mjs
 *
 * 双产物（M3.5 起支持 Firefox）：
 *   dist/          Chrome / Edge（edge://extensions / chrome://extensions 加载已解压）
 *     background.js（esm，manifest 声明为 module 型 SW）
 *   dist-firefox/  Firefox（about:debugging「临时载入附加组件」选 manifest.json）
 *     background.js（iife，MV3 事件页 background.scripts——Firefox 不支持 SW 后台）
 *   两者共用 content.js / rate-guard.js（iife）、icons/128.png；
 *   manifest.json 以 Chrome 版为基底，Firefox 变体在 firefoxManifest() 中派生。
 */

import { build } from "esbuild";
import { existsSync } from "node:fs";
import { copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const dist = join(root, "dist");
const distFirefox = join(root, "dist-firefox");

/**
 * Firefox 的 Gecko 扩展 ID：Native Messaging 清单 allowed_extensions 与
 * browser_specific_settings.gecko.id 必须一致（桌面侧 nm_bridge.rs GECKO_EXTENSION_ID 同源）
 */
const GECKO_ID = "connector@omnispeed.app";

/** Chrome manifest → Firefox manifest（gecko 后台模型 / 标识差异，能力保持一致） */
function firefoxManifest(chrome) {
  const mf = structuredClone(chrome);
  // Firefox MV3 不支持 service_worker 后台，用事件页（scripts, iife 产物）
  mf.background = { scripts: ["background.js"] };
  // key / minimum_chrome_version 是 Chromium 专属字段
  delete mf.key;
  delete mf.minimum_chrome_version;
  mf.browser_specific_settings = {
    // content_scripts 的 world:"MAIN" 自 Firefox 128 起支持
    gecko: { id: GECKO_ID, strict_min_version: "128.0" },
  };
  return mf;
}

/** 入口清单：并行开发期间 content 侧文件可能尚未就绪，按存在性过滤 */
const entries = [
  { entry: "src/background/background.ts", out: "background.js", format: "esm", to: [dist] },
  // Firefox 事件页不认 esm 后台 → 同源码单独出一份 iife
  { entry: "src/background/background.ts", out: "background.js", format: "iife", to: [distFirefox] },
  { entry: "src/content/content.ts", out: "content.js", format: "iife", to: [dist, distFirefox] },
  { entry: "src/content/rate-guard.ts", out: "rate-guard.js", format: "iife", to: [dist, distFirefox] },
];

// 产物目录清空重建
for (const dir of [dist, distFirefox]) {
  await rm(dir, { recursive: true, force: true });
  await mkdir(join(dir, "icons"), { recursive: true });
}

const built = { [dist]: [], [distFirefox]: [] };
const skipped = [];

for (const { entry, out, format, to } of entries) {
  const entryPath = join(root, entry);
  if (!existsSync(entryPath)) {
    skipped.push(entry);
    console.warn(`[build] 警告：入口不存在，跳过 ${entry}`);
    continue;
  }
  const [first, ...rest] = to;
  await build({
    entryPoints: [entryPath],
    outfile: join(first, out),
    bundle: true,
    format,
    // 双内核基线：Chromium 116（MV3 稳定）/ Firefox 128（world:MAIN）
    target: ["chrome116", "firefox128"],
    minify: false, // 开源可审计，保持产物可读
    sourcemap: false,
    charset: "utf8",
  });
  built[first].push(out);
  for (const dir of rest) {
    await copyFile(join(first, out), join(dir, out));
    built[dir].push(out);
  }
}

// manifest：Chrome 原样复制，Firefox 派生变体
const chromeManifest = JSON.parse(await readFile(join(root, "manifest.json"), "utf8"));
await copyFile(join(root, "manifest.json"), join(dist, "manifest.json"));
built[dist].push("manifest.json");
await writeFile(
  join(distFirefox, "manifest.json"),
  `${JSON.stringify(firefoxManifest(chromeManifest), null, 2)}\n`,
);
built[distFirefox].push("manifest.json");

// 图标
const iconSrc = join(root, "..", "desktop", "src-tauri", "icons", "128x128.png");
if (existsSync(iconSrc)) {
  for (const dir of [dist, distFirefox]) {
    await copyFile(iconSrc, join(dir, "icons", "128.png"));
    built[dir].push("icons/128.png");
  }
} else {
  console.warn(`[build] 警告：图标不存在，跳过 ${iconSrc}`);
}

// 构建结果清单
for (const dir of [dist, distFirefox]) {
  console.log(`\n[build] ${dir === dist ? "dist/（Chrome/Edge）" : "dist-firefox/（Firefox）"} 产物：`);
  for (const name of built[dir].sort()) {
    const { size } = await stat(join(dir, name));
    console.log(`  ${name.padEnd(20)} ${(size / 1024).toFixed(1)} KB`);
  }
  console.log(`[build] 共 ${(await readdir(dir)).length} 项（含 icons/ 目录）`);
}
if (skipped.length > 0) {
  console.log(`[build] 已跳过（入口缺失）：${skipped.join("、")}`);
}
