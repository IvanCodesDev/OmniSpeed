import type { SiteAdapter } from "./types";
import { bilibili } from "./bilibili";
import { douyin } from "./douyin";
import { generic } from "./generic";
import { youtube } from "./youtube";

export type { SiteAdapter } from "./types";

/** 注册表：新增站点适配器在此登记（v1.0 首发 8 站，其余见开发文档 §5.2.1，逐站补充） */
const adapters: SiteAdapter[] = [bilibili, douyin, youtube];

/** 按 location.host 取适配器，未命中回 generic 兜底 */
export function adapterFor(host: string): SiteAdapter {
  return adapters.find((a) => a.match.test(host)) ?? generic;
}
