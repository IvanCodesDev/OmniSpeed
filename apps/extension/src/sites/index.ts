import type { SiteAdapter } from "./types";
import { bilibili } from "./bilibili";
import { douyin } from "./douyin";
import { generic } from "./generic";
import { iqiyi } from "./iqiyi";
import { ixigua } from "./ixigua";
import { kuaishou } from "./kuaishou";
import { qq } from "./qq";
import { youku } from "./youku";
import { youtube } from "./youtube";

export type { SiteAdapter } from "./types";

/** 注册表：v1.0 首发 8 站齐（开发文档 §5.2.1），新增站点适配器在此登记 */
const adapters: SiteAdapter[] = [bilibili, douyin, youtube, qq, iqiyi, youku, ixigua, kuaishou];

/** 按 location.host 取适配器，未命中回 generic 兜底 */
export function adapterFor(host: string): SiteAdapter {
  return adapters.find((a) => a.match.test(host)) ?? generic;
}
