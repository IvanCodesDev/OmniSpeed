# OmniSpeed Connector（浏览器扩展，MV3）

OmniSpeed 桌面应用的浏览器伴侣：通过 Native Messaging 与桌面核心互通，让全局快捷键统一控制网页视频倍速（0.25×–16×），并提供 Rate-Guard 倍速锁定防站点复位。

## 目录结构

```
extension/
├─ manifest.json          # MV3 清单（key 固定 → 确定性扩展 ID）
├─ build.mjs              # esbuild 构建脚本
├─ src/
│  ├─ background/         # Service Worker：NM 长连接 + 标签路由
│  ├─ content/            # ISOLATED world 常规接管 + MAIN world Rate-Guard
│  ├─ sites/              # 站点适配器（八站：B站/抖音/YouTube/腾讯/爱奇艺/优酷/西瓜/快手 + generic）
│  └─ shared/protocol.ts  # 消息契约（冻结，改动需同步 Rust 侧 nm_bridge.rs）
├─ dist/                  # Chrome / Edge 构建产物（加载已解压扩展指向这里）
└─ dist-firefox/          # Firefox 构建产物（about:debugging 临时载入）
```

## 构建

```bash
# 仓库根目录
npm run ext:build    # 等价于 apps/extension 下的 node build.mjs
npm run check -w @omnispeed/extension   # TypeScript 类型检查
```

一次构建输出两套产物：

- `apps/extension/dist/`（Chrome / Edge）：`background.js`（esm module SW）、`content.js` / `rate-guard.js`（iife）、`manifest.json`、`icons/128.png`。
- `apps/extension/dist-firefox/`（Firefox ≥ 128）：`background.js` 改为 iife **事件页**（Firefox MV3 不支持 Service Worker 后台），manifest 由 Chrome 版派生——去掉 `key` / `minimum_chrome_version`，加 `browser_specific_settings.gecko`（id + `strict_min_version: 128.0`，MAIN world 内容脚本的最低要求）。

## 加载扩展

### Chrome / Edge

1. 打开 `edge://extensions`（或 `chrome://extensions`）。
2. 打开右上角「开发者模式」。
3. 点「加载已解压的扩展程序」，选择 `apps/extension/dist/` 目录。

### Firefox（≥ 128）

1. 地址栏打开 `about:debugging#/runtime/this-firefox`。
2. 点「临时载入附加组件…」，选择 `apps/extension/dist-firefox/manifest.json`。
3. 临时附加组件在 Firefox 重启后失效，需重新载入；长期安装需 AMO 签名（或在 ESR / Developer 版中关闭 `xpinstall.signatures.required`），发布版将提供签名产物。

### 确定性扩展 ID

`manifest.json` 的 `key` 字段固定了公钥，因此无论在哪台机器加载，扩展 ID 恒为：

```
ejpnpjbhmgckjfdednjgfhdpobencmpb
```

桌面应用按此 ID 注册 Native Messaging host 清单的 `allowed_origins`，二者必须一致，请勿改动 `key`。

Firefox 侧则使用固定 Gecko ID `connector@omnispeed.app`（build.mjs 写入 `browser_specific_settings.gecko.id`），
桌面侧 Firefox NM 清单的 `allowed_extensions` 与之对应（`nm_bridge.rs` 的 `GECKO_EXTENSION_ID` 同源）。

### Native Messaging 宿主

- host 名称：`com.omnispeed.host`（桌面核心以 `omnispeed.exe --nm-host` 作为中继被浏览器拉起）。
- 注册方式：**桌面应用启动时自动写入注册表**（用户级，无需管理员权限），无需手动配置：
  - Chrome / Edge：`HKCU\Software\Google\Chrome（Microsoft\Edge）\NativeMessagingHosts\com.omnispeed.host`，清单 `allowed_origins` 指向扩展 ID；
  - Firefox：`HKCU\Software\Mozilla\NativeMessagingHosts\com.omnispeed.host`，单独一份 `com.omnispeed.host.firefox.json`，`allowed_extensions` 指向 Gecko ID。

## 常见问题

**扩展连不上桌面核心？**

1. 确认 OmniSpeed 桌面应用已在运行（NM host 清单由它启动时注册）。
2. 桌面应用启动后无需重载扩展——Service Worker 会自动重连（指数退避，封顶 60s；切换标签页或页面有媒体活动时也会立即触发重连）。
3. 若刚更换过 `manifest.json` 的 `key` 或扩展是从商店以外的其他 ID 加载的，NM 的 `allowed_origins` 校验会拒绝连接——请用本仓库构建的 `dist/` 加载以保持 ID 一致。

**修改代码后不生效？**

重新 `npm run ext:build`，然后在扩展管理页点击该扩展的「重新加载」。
