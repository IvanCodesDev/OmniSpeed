import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import { OsdApp } from "./osd/OsdApp";
import { initCoreSync } from "./store";
import "./index.css";

// 同一前端入口服务两个窗口：主面板与全局 OSD（?window=osd，由 Rust 创建）
const isOsd = new URLSearchParams(window.location.search).get("window") === "osd";
if (!isOsd) void initCoreSync();

createRoot(document.getElementById("root")!).render(
  <StrictMode>{isOsd ? <OsdApp /> : <App />}</StrictMode>,
);
