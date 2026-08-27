import { Fragment, useEffect, useRef, useState } from "react";
import { ChevronRight, Pencil, Search } from "lucide-react";
import playerWindow from "../assets/clay/player-window.png";
import { AppIcon } from "../components/AppIcon";
import { Select } from "../components/Select";
import { Toggle } from "../components/Toggle";
import { managedApps, statusText, type AppStatus, type ManagedApp } from "../data";
import { cn } from "../lib/cn";
import { formatRate, useAppStore } from "../store";

const dotColor: Record<AppStatus, string> = {
  connected: "bg-emerald-500",
  adapted: "bg-emerald-500",
  "needs-setup": "bg-amber-400",
};

const keyRules = [
  { label: "加速", chip: "2×", chipCls: "bg-[#f0604d]/15 text-[#dd4531]", keys: ["Ctrl", "Up"] },
  { label: "减速", chip: "0.5×", chipCls: "bg-[#f4b62e]/20 text-[#a97a10]", keys: ["Ctrl", "Down"] },
  { label: "恢复 1.0×", chip: "1.0×", chipCls: "bg-emerald-500/14 text-emerald-600", keys: ["Ctrl", "R"] },
];

/** 浏览器行展开的「网站适配」列表（PRD §7.2 / §7.6） */
function SiteRulesPanel() {
  const siteRules = useAppStore((s) => s.siteRules);
  const updateSiteRule = useAppStore((s) => s.updateSiteRule);

  return (
    <div className="border-b border-line bg-card-2/40 px-5 pb-3 pt-2">
      <div className="grid grid-cols-[minmax(0,1fr)_88px_64px_54px_54px] items-center gap-2 py-1.5 text-[11px] text-mute">
        <span>网站适配（内置站点适配器）</span>
        <span>默认倍速</span>
        <span>上限</span>
        <span>倍速锁定</span>
        <span>新片跟随</span>
      </div>
      {siteRules.map((rule) => (
        <div
          key={rule.host}
          className="grid grid-cols-[minmax(0,1fr)_88px_64px_54px_54px] items-center gap-2 py-[7px]"
        >
          <span className="min-w-0">
            <span className="block truncate text-[13px] font-medium">{rule.name}</span>
            <span className="block truncate text-[11px] text-mute">{rule.host}</span>
          </span>
          <Select
            size="sm"
            className="w-full"
            aria-label={`${rule.name} 默认倍速`}
            value={rule.defaultRate ?? ""}
            onChange={(v) => updateSiteRule(rule.host, { defaultRate: v === "" ? null : Number(v) })}
            options={[
              { value: "" as const, label: "跟随全局" },
              ...[1, 1.25, 1.5, 2, 3, 5].map((v) => ({ value: v, label: `${formatRate(v)}×` })),
            ]}
          />
          <Select
            size="sm"
            className="w-full"
            aria-label={`${rule.name} 倍速上限`}
            value={rule.maxRate}
            onChange={(v) => updateSiteRule(rule.host, { maxRate: v })}
            options={[2, 3, 4, 6, 8, 16].map((v) => ({ value: v, label: `${v}×` }))}
          />

          <Toggle small checked={rule.rateLock} onChange={(v) => updateSiteRule(rule.host, { rateLock: v })} />
          <Toggle small checked={rule.follow} onChange={(v) => updateSiteRule(rule.host, { follow: v })} />
        </div>
      ))}
    </div>
  );
}

/** 规则编辑面板：随选中应用类型变化（浏览器 / 播放器 / 未知） */
function RuleEditor({ app }: { app: ManagedApp }) {
  const [tab, setTab] = useState<"ipc" | "hotkey" | "auto">("hotkey");
  const [justSaved, setJustSaved] = useState(false);
  const markRuleSaved = useAppStore((s) => s.markRuleSaved);
  const saveTimer = useRef<number>(undefined);

  // 切换应用时定位到该应用的首选 Tab（VLC 等优先 IPC，见开发文档 §7.3）
  useEffect(() => {
    setTab(app.ipc === "vlc-http" ? "ipc" : "hotkey");
    setJustSaved(false);
  }, [app.id, app.ipc]);

  useEffect(() => () => window.clearTimeout(saveTimer.current), []);

  const save = () => {
    markRuleSaved(app.id);
    setJustSaved(true);
    window.clearTimeout(saveTimer.current);
    saveTimer.current = window.setTimeout(() => setJustSaved(false), 1500);
  };

  if (app.kind === "browser") {
    return (
      <>
        <div className="mt-4 rounded-xl bg-emerald-500/8 px-4 py-3 text-[12.5px] leading-relaxed text-emerald-700">
          浏览器扩展已连接，网页媒体可精确设速（0.25×–16×），并带倍速锁定防复位。
        </div>
        <p className="mt-3 px-1 text-[12.5px] leading-relaxed text-mute">
          点击左侧该浏览器所在行可展开「网站适配」，对哔哩哔哩、抖音等平台单独设置默认倍速、上限、倍速锁定与新视频跟随。
        </p>
      </>
    );
  }

  return (
    <>
      <div className="mt-4 flex overflow-hidden rounded-xl border border-line">
        {(
          [
            { id: "ipc", label: "IPC" },
            { id: "hotkey", label: "快捷键" },
            { id: "auto", label: "自动识别" },
          ] as const
        ).map((t) => (
          <button
            key={t.id}
            onClick={() => setTab(t.id)}
            className={cn(
              "flex-1 py-2 text-[13px] font-semibold transition-colors",
              tab === t.id ? "bg-accent text-on-accent" : "bg-card text-ink-2 hover:bg-card-2",
            )}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === "ipc" && (
        <div className="mt-5">
          {app.ipc === "vlc-http" && (
            <div className="flex flex-col gap-3.5">
              <div>
                <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">HTTP 端口</div>
                <input
                  defaultValue="8080"
                  inputMode="numeric"
                  className="w-full rounded-xl border border-line bg-card-2/50 px-3 py-2 text-[13px] outline-none transition-colors focus:border-accent/60"
                />
              </div>
              <div>
                <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">访问密码</div>
                <input
                  placeholder="可选"
                  className="w-full rounded-xl border border-line bg-card-2/50 px-3 py-2 text-[13px] outline-none transition-colors focus:border-accent/60"
                />
              </div>
              <p className="rounded-xl bg-card-2/60 px-3 py-2.5 text-[12px] leading-relaxed text-mute">
                在 VLC 中开启：工具 → 首选项 → 显示所有设置 → 主界面 → 勾选 Web。IPC 可一步设为任意精确倍速，失败时自动回退快捷键模式。
              </p>
            </div>
          )}
          {app.ipc === "wm-command" && (
            <p className="rounded-xl bg-card-2/60 px-3 py-2.5 text-[12.5px] leading-relaxed text-mute">
              PotPlayer 通过窗口控制消息（WM_COMMAND）直接设速，无需配置、无需窗口前台。
            </p>
          )}
          {app.ipc === "none" && (
            <p className="rounded-xl border border-dashed border-line bg-card-2/40 px-4 py-6 text-center text-[12.5px] leading-relaxed text-mute">
              未发现该应用的已知控制接口，
              <br />
              请使用「快捷键」或「自动识别」。
            </p>
          )}
        </div>
      )}

      {tab === "hotkey" && (
        <div className="mt-5 flex flex-col gap-4">
          {keyRules.map((rule) => (
            <div key={rule.label}>
              <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">{rule.label}</div>
              <div className="flex items-center gap-2.5">
                <span className={cn("shrink-0 rounded-lg px-2.5 py-1.5 text-xs font-bold", rule.chipCls)}>
                  {rule.chip}
                </span>
                <div className="flex min-w-0 flex-1 items-center justify-between rounded-xl border border-line bg-card-2/50 px-3 py-2">
                  <span className="truncate text-[13px] font-medium text-ink-2">
                    {rule.keys.map((k, i) => (
                      <Fragment key={k}>
                        {i > 0 && <span className="mx-1.5 text-mute">+</span>}
                        {k}
                      </Fragment>
                    ))}
                  </span>
                  <button className="shrink-0 text-mute transition-colors hover:text-accent" aria-label="编辑按键">
                    <Pencil size={13.5} />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {tab === "auto" && (
        <p className="mt-5 rounded-xl border border-dashed border-line bg-card-2/40 px-4 py-6 text-center text-[12.5px] leading-relaxed text-mute">
          自动探测窗口与控件中的倍速控制，
          <br />
          适用于常见网课客户端（实验特性）
        </p>
      )}

      <button
        onClick={save}
        className={cn(
          "mt-6 h-11 w-full rounded-xl text-sm font-semibold transition-all active:scale-[0.98]",
          justSaved ? "bg-emerald-500 text-white" : "bg-accent text-on-accent",
        )}
      >
        {justSaved ? "已保存 ✓" : "保存规则"}
      </button>
    </>
  );
}

export function AppsPage() {
  const [query, setQuery] = useState("");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const selectedId = useAppStore((s) => s.selectedAppId);
  const setSelected = useAppStore((s) => s.setSelectedApp);
  const ruleSavedIds = useAppStore((s) => s.ruleSavedIds);

  const q = query.trim().toLowerCase();
  const list = managedApps.filter(
    (a) => a.name.toLowerCase().includes(q) || a.process.toLowerCase().includes(q),
  );
  const selected = managedApps.find((a) => a.id === selectedId) ?? managedApps[managedApps.length - 1];

  const statusOf = (app: ManagedApp): AppStatus =>
    app.status === "needs-setup" && ruleSavedIds.includes(app.id) ? "adapted" : app.status;

  const onRowClick = (app: ManagedApp) => {
    setSelected(app.id);
    if (app.kind === "browser") setExpandedId((cur) => (cur === app.id ? null : app.id));
  };

  return (
    <div className="@container">
      <header className="flex items-start justify-between gap-6">
        <div>
          <h1 className="text-[26px] font-bold tracking-tight">应用管理</h1>
          <p className="mt-1.5 text-[13px] text-mute">管理浏览器与桌面播放器的控制方式</p>
        </div>
        <label className="flex w-[248px] items-center gap-2.5 rounded-xl border border-line bg-card px-3.5 py-2.5 transition-colors focus-within:border-accent/50">
          <Search size={15} className="shrink-0 text-mute" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="搜索应用"
            className="w-full bg-transparent text-[13px] outline-none placeholder:text-mute"
          />
        </label>
      </header>

      {/* 容器 <48rem 时右侧面板换行到列表下方并居中，避免列表被挤压 */}
      <div className="mt-7 flex flex-col gap-6 @3xl:flex-row @3xl:items-start">
        {/* 应用列表 */}
        <div className="flat-card min-w-0 overflow-hidden rounded-2xl @3xl:flex-1">
          <div className="grid grid-cols-[minmax(0,1.3fr)_96px_118px_20px] items-center gap-2 border-b border-line px-5 py-3 text-[12.5px] text-mute">
            <span>应用</span>
            <span>状态</span>
            <span>控制方式</span>
            <span />
          </div>
          {list.map((app) => {
            const st = statusOf(app);
            const expanded = expandedId === app.id;
            return (
              <Fragment key={app.id}>
                <button
                  onClick={() => onRowClick(app)}
                  title={app.kind === "browser" ? "点击展开 / 收起网站适配" : undefined}
                  className={cn(
                    "grid w-full grid-cols-[minmax(0,1.3fr)_96px_118px_20px] items-center gap-2 border-b border-line px-5 py-3 text-left transition-colors last:border-b-0",
                    selectedId === app.id ? "bg-accent-soft/70" : "hover:bg-card-2/60",
                  )}
                >
                  <span className="flex min-w-0 items-center gap-3">
                    <AppIcon id={app.icon} size={34} />
                    <span className="truncate text-sm font-semibold">{app.name}</span>
                  </span>
                  <span className="flex items-center gap-1.5 text-[13px] text-ink-2">
                    <span className={cn("size-2 shrink-0 rounded-full", dotColor[st])} />
                    {statusText[st]}
                  </span>
                  <span className="truncate text-[13px] text-ink-2">{app.method}</span>
                  <ChevronRight
                    size={15}
                    className={cn("text-mute transition-transform", expanded && "rotate-90")}
                  />
                </button>
                {app.kind === "browser" && expanded && <SiteRulesPanel />}
              </Fragment>
            );
          })}
          {list.length === 0 && (
            <div className="px-5 py-9 text-center text-[13px] text-mute">
              没有匹配「{query}」的应用
            </div>
          )}
        </div>

        {/* 规则编辑面板（拟物图块为设计稿原图素材） */}
        <aside className="flat-card w-full max-w-[420px] self-center rounded-2xl p-6 @3xl:w-[288px] @3xl:max-w-none @3xl:shrink-0 @3xl:self-auto">
          <div className="flex flex-col items-center">
            <img
              src={playerWindow}
              className="h-[124px] w-auto select-none @3xl:h-[136px]"
              alt=""
              draggable={false}
            />
            <h3 className="mt-3 text-[17px] font-bold">设置控制方式</h3>
            <p className="mt-1 text-[13px] text-mute">进程：{selected.process}</p>
          </div>
          <RuleEditor app={selected} />
        </aside>
      </div>
    </div>
  );
}
