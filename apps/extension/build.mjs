/**
 * OmniSpeed 扩展构建脚本：node build.mjs
 *
 * 产出 dist/ 目录（可直接在 edge://extensions / chrome://extensions 加载已解压）：
 *   background.js（esm，manifest 声明为 module 型 SW）
 *   content.js / rate-guard.js（iife；并行开发期间入口缺失则警告跳过）
 *   manifest.json、icons/128.png
 */

import { build } from "esbuild";
import { existsSync } from "node:fs";
import { copyFile, mkdir, readdir, rm, stat } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const dist = join(root, "dist");

/** 入口清单：并行开发期间 content 侧文件可能尚未就绪，按存在性过滤 */
const entries = [
  { entry: "src/background/background.ts", out: "background.js", format: "esm" },
  { entry: "src/content/content.ts", out: "content.js", format: "iife" },
  { entry: "src/content/rate-guard.ts", out: "rate-guard.js", format: "iife" },
];

// dist/ 清空重建
await rm(dist, { recursive: true, force: true });
await mkdir(join(dist, "icons"), { recursive: true });

const built = [];
const skipped = [];

for (const { entry, out, format } of entries) {
  const entryPath = join(root, entry);
  if (!existsSync(entryPath)) {
    skipped.push(entry);
    console.warn(`[build] 警告：入口不存在，跳过 ${entry}`);
    continue;
  }
  await build({
    entryPoints: [entryPath],
    outfile: join(dist, out),
    bundle: true,
    format,
    target: "chrome116",
    minify: false, // 开源可审计，保持产物可读
    sourcemap: false,
    charset: "utf8",
  });
  built.push(out);
}

// 静态资源
await copyFile(join(root, "manifest.json"), join(dist, "manifest.json"));
built.push("manifest.json");

const iconSrc = join(root, "..", "desktop", "src-tauri", "icons", "128x128.png");
if (existsSync(iconSrc)) {
  await copyFile(iconSrc, join(dist, "icons", "128.png"));
  built.push("icons/128.png");
} else {
  console.warn(`[build] 警告：图标不存在，跳过 ${iconSrc}`);
}

// 构建结果清单
console.log("\n[build] dist/ 产物：");
for (const name of built.sort()) {
  const { size } = await stat(join(dist, name));
  console.log(`  ${name.padEnd(20)} ${(size / 1024).toFixed(1)} KB`);
}
if (skipped.length > 0) {
  console.log(`[build] 已跳过（入口缺失）：${skipped.join("、")}`);
}
console.log(`[build] 完成：${(await readdir(dist)).length} 项（含 icons/ 目录）`);
