import { Fragment, useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, ChevronRight, Puzzle, RotateCcw, Search } from "lucide-react";
import playerWindow from "../assets/clay/player-window.png";
import { AppIcon } from "../components/AppIcon";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { Toggle } from "../components/Toggle";
import {
  brandOf,
  statusText,
  type AppInfo,
  type AppKeyRate,
  type AppRulePatch,
  type AppStatus,
} from "../data";
import { cn } from "../lib/cn";
import { takeoverClient } from "../lib/ipc";
import { formatRate, useAppStore } from "../store";

const dotColor: Record<AppStatus, string> = {
  connected: "bg-emerald-500",
  adapted: "bg-emerald-500",
  "needs-setup": "bg-amber-400",
};

type RuleMethod = AppRulePatch["method"];

const methodTabs: readonly RuleMethod[] = ["ipc", "hotkey", "auto"];
const methodTabLabel: Record<RuleMethod, string> = { ipc: "IPC", hotkey: "快捷键", auto: "自动识别" };

/** 目标软件自身的调速快捷键三项（语义色：加速红 / 减速黄 / 恢复绿，PRD §7.2） */
const keyFields = [
  { field: "up", label: "加速", cls: "bg-[#f0604d]/15 text-[#dd4531]", Icon: ArrowUp, example: "]" },
  { field: "down", label: "减速", cls: "bg-[#f4b62e]/20 text-[#a97a10]", Icon: ArrowDown, example: "[" },
  { field: "reset", label: "恢复 1.0×", cls: "bg-emerald-500/14 text-emerald-600", Icon: RotateCcw, example: "=" },
] as const;

const inputCls =
  "w-full min-w-0 flex-1 rounded-xl border border-line bg-card-2/50 px-3 py-2 text-[13px] outline-none transition-colors focus:border-accent/60";

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

/**
 * 该播放器的某些按键给的是绝对倍速（如百度网盘桌面端的数字键 1–5），
 * OmniSpeed 会「先按档位键钉住、再按步进键补差额」，所以按键通道也能一步到精确值。
 * 说明可用区间与精度，以及按键通道绕不开的前提：播放器窗口得在前台。
 */
function KeyRateNote({ keyRate }: { keyRate: AppKeyRate }) {
  const anchors = keyRate.anchors.map((a) => a.key).join(" / ");
  return (
    <p className="rounded-xl border border-accent/25 bg-accent/[0.06] px-3 py-2.5 text-[12px] leading-relaxed text-ink-2">
      该播放器的 <b>{anchors}</b> 键可直接设为对应倍速，OmniSpeed 据此先定档再微调，
      因此滑块与预设也能精确生效——范围 {keyRate.min}×–{keyRate.max}×，精度{" "}
      {keyRate.step}×（超出范围取端点，中间值就近取整）。
      <br />
      档位之间的值靠连按步进键补足，每次间隔 {keyRate.stepGapMs} 毫秒（播放器读回上一次倍速要时间，
      贴着发会被吞掉步进），所以跨档位设速时能看见倍速爬升半秒左右，属正常。
      <br />
      注意：模拟按键需要该播放器窗口在前台，OmniSpeed 会在下发前自动把它激活。
    </p>
  );
}

/**
 * 该播放器的控制消息只提供固定几档绝对倍速（MPC-HC 的「播放 → 速度」菜单命令），
 * 一条消息即到位、不需要窗口在前台，但倍速只能落在这些档上。
 * 档距不均匀，所以要把档位逐个列出来，而不是像网格那样说一个「精度」。
 */
function RateLadderNote({ ladder }: { ladder: number[] }) {
  return (
    <p className="rounded-xl border border-accent/25 bg-accent/[0.06] px-3 py-2.5 text-[12px] leading-relaxed text-ink-2">
      该播放器通过控制消息设速，一步到位且不需要窗口在前台，但倍速只能落在固定档位上：
      <br />
      <b>{ladder.map((r) => `${r}×`).join(" · ")}</b>
      <br />
      滑块与预设设成档位之间的值时会就近取档（如 2.4× 落到 2×），热键则一次挪一档。
      OSD 显示的始终是取档后的值，与播放器实际倍速一致。
    </p>
  );
}

/** Chromium 套壳客户端（B 站桌面端等）的 CDP 接管面板（M4.5） */
function CdpPanel({ app }: { app: AppInfo }) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  const takeover = async () => {
    setBusy(true);
    setMessage(null);
    try {
      setMessage(await takeoverClient(app.id));
      setFailed(false);
    } catch (e) {
      setMessage(String(e));
      setFailed(true);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="flex flex-col gap-3.5">
      <p className="rounded-xl bg-card-2/60 px-3 py-2.5 text-[12px] leading-relaxed text-mute">
        该客户端是 Chromium 套壳应用，OmniSpeed 通过仅本机可见的 CDP
        调试通道直接控制其内部视频（0.25×–16× 精确设速、可回读）。首次使用请在客户端运行时点「接管」——会重启该客户端并开启控制端口，之后全局快捷键即刻生效。
      </p>
      <button
        onClick={() => void takeover()}
        disabled={busy}
        className={cn(
          "h-10 w-full rounded-xl text-sm font-semibold transition-all active:scale-[0.98]",
          busy ? "bg-card-2 text-mute" : "bg-accent text-on-accent",
        )}
      >
        {busy ? "接管中…（客户端会重启）" : "接管客户端"}
      </button>
      {message && (
        <p
          className={cn(
            "rounded-xl px-3 py-2.5 text-[12px] leading-relaxed",
            failed ? "bg-[#f0604d]/10 text-[#dd4531]" : "bg-emerald-500/10 text-emerald-600",
          )}
        >
          {message}
        </p>
      )}
    </div>
  );
}

/** 该应用当前规则的首选编辑 Tab */
const methodOf = (app: AppInfo): RuleMethod => (app.method === "extension" ? "hotkey" : app.method);

/** 规则编辑面板：随选中应用类型变化（浏览器 / 播放器 / 未知） */
function RuleEditor({ app }: { app: AppInfo }) {
  const saveAppRule = useAppStore((s) => s.saveAppRule);
  const [method, setMethod] = useState<RuleMethod>(methodOf(app));
  const [keys, setKeys] = useState({ up: "", down: "", reset: "" });
  const [pipe, setPipe] = useState("");
  const [port, setPort] = useState("");
  const [password, setPassword] = useState("");
  const [justSaved, setJustSaved] = useState(false);
  const saveTimer = useRef<number>(undefined);

  // 仅在切换应用时用其已保存规则重置草稿（依赖 app.id 而非 app：
  // apps:status-changed / 保存回包刷新列表对象时不打断正在进行的编辑）
  useEffect(() => {
    setMethod(methodOf(app));
    setKeys({ up: app.keys?.up ?? "", down: app.keys?.down ?? "", reset: app.keys?.reset ?? "" });
    setPipe(app.ipcConfig?.pipe ?? "");
    setPort(app.ipcConfig?.port != null ? String(app.ipcConfig.port) : "");
    setPassword(app.ipcConfig?.password ?? "");
    setJustSaved(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [app.id]);

  useEffect(() => () => window.clearTimeout(saveTimer.current), []);

  const save = () => {
    const trimmed = { up: keys.up.trim(), down: keys.down.trim(), reset: keys.reset.trim() };
    const hasKeys = Boolean(trimmed.up || trimmed.down || trimmed.reset);
    const portNum = Number.parseInt(port, 10);
    const patch: AppRulePatch = {
      id: app.id,
      process: app.process,
      name: app.name,
      method,
      keys: hasKeys ? trimmed : null,
      // 按 IPC 通道类型提交对应配置（mpv 管道 / VLC HTTP 端口与密码 / CDP 调试端口），切换控制方式不丢配置
      ipcConfig:
        app.ipc === "mpv-ipc"
          ? { pipe: pipe.trim() || null, port: null, password: null }
          : app.ipc === "vlc-http"
            ? { pipe: null, port: Number.isFinite(portNum) ? portNum : null, password: password || null }
            : app.ipc === "cdp"
              ? { pipe: null, port: app.ipcConfig?.port ?? null, password: null }
              : null,
    };
    void saveAppRule(patch).then(() => {
      setJustSaved(true);
      window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => setJustSaved(false), 1500);
    });
  };

  // 浏览器条目：M2 阶段扩展尚未接入，展示引导态、不可编辑控制方式（扩展与握手 M3 接入）
  if (app.kind === "browser") {
    return (
      <>
        <div className="mt-4 flex flex-col items-center gap-2 rounded-xl border border-dashed border-line bg-card-2/40 px-4 py-6 text-center">
          <Puzzle size={22} strokeWidth={1.6} className="text-mute" />
          <div className="text-[13px] font-semibold text-ink-2">等待浏览器扩展（M3）</div>
          <p className="text-[12px] leading-relaxed text-mute">
            网页媒体将由浏览器扩展精确设速（0.25×–16×，含倍速锁定防复位），扩展随 M3 版本提供，届时此处显示连接状态。
          </p>
        </div>
        <p className="mt-3 px-1 text-[12.5px] leading-relaxed text-mute">
          点击左侧该浏览器所在行可展开「网站适配」，提前为哔哩哔哩、抖音等平台设置默认倍速、上限、倍速锁定与新视频跟随。
        </p>
      </>
    );
  }

  return (
    <>
      <div className="mt-4 flex justify-center">
        <Segmented
          options={methodTabs}
          value={method}
          onChange={setMethod}
          format={(m) => methodTabLabel[m]}
        />
      </div>

      {method === "ipc" && (
        <div className="mt-5">
          {app.ipc === "vlc-http" && (
            <div className="flex flex-col gap-3.5">
              <div>
                <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">HTTP 端口</div>
                <input
                  value={port}
                  onChange={(e) => setPort(e.target.value)}
                  placeholder="8080"
                  inputMode="numeric"
                  aria-label="VLC HTTP 端口"
                  className={inputCls}
                />
              </div>
              <div>
                <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">访问密码</div>
                <input
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="可选"
                  aria-label="VLC 访问密码"
                  className={inputCls}
                />
              </div>
              <p className="rounded-xl bg-card-2/60 px-3 py-2.5 text-[12px] leading-relaxed text-mute">
                在 VLC 中开启：工具 → 首选项 → 显示所有设置 → 主界面 → 勾选 Web。IPC 可一步设为任意精确倍速，失败时自动回退快捷键模式。
              </p>
            </div>
          )}
          {app.ipc === "mpv-ipc" && (
            <div className="flex flex-col gap-3.5">
              <div>
                <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">IPC 管道</div>
                <input
                  value={pipe}
                  onChange={(e) => setPipe(e.target.value)}
                  placeholder="\\.\pipe\mpvsocket"
                  aria-label="mpv IPC 管道"
                  className={inputCls}
                />
              </div>
              <p className="rounded-xl bg-card-2/60 px-3 py-2.5 text-[12px] leading-relaxed text-mute">
                需为 mpv 开启 IPC：启动参数或 mpv.conf 中设置 input-ipc-server=\\.\pipe\mpvsocket。IPC 可一步设为任意精确倍速，失败时自动回退快捷键模式。
              </p>
            </div>
          )}
          {app.ipc === "wm-command" && (
            <p className="rounded-xl bg-card-2/60 px-3 py-2.5 text-[12.5px] leading-relaxed text-mute">
              PotPlayer 通过窗口控制消息（WM_COMMAND）直接设速，无需配置、无需窗口前台。
            </p>
          )}
          {app.ipc === "cdp" && <CdpPanel app={app} />}
          {app.ipc === "none" && (
            <p className="rounded-xl border border-dashed border-line bg-card-2/40 px-4 py-6 text-center text-[12.5px] leading-relaxed text-mute">
              未发现该应用的已知控制接口，
              <br />
              请使用「快捷键」或「自动识别」。
            </p>
          )}
        </div>
      )}

      {method === "hotkey" && (
        <div className="mt-5 flex flex-col gap-4">
          {keyFields.map(({ field, label, cls, Icon, example }) => (
            <div key={field}>
              <div className="mb-1.5 text-[12.5px] font-medium text-ink-2">{label}</div>
              <div className="flex items-center gap-2.5">
                <span className={cn("grid size-8 shrink-0 place-items-center rounded-lg", cls)}>
                  <Icon size={14} strokeWidth={2.4} />
                </span>
                <input
                  value={keys[field]}
                  onChange={(e) => setKeys((k) => ({ ...k, [field]: e.target.value }))}
                  placeholder={`目标软件按键，如 ${example}`}
                  aria-label={`「${label}」按键`}
                  className={inputCls}
                />
              </div>
            </div>
          ))}
          <p className="rounded-xl bg-card-2/60 px-3 py-2.5 text-[12px] leading-relaxed text-mute">
            这里绑定的是目标软件自身的调速快捷键，OmniSpeed 会替你按下；与「快捷键」页的全局快捷键是两回事。
          </p>
        </div>
      )}

      {method === "auto" && (
        <p className="mt-5 rounded-xl border border-dashed border-line bg-card-2/40 px-4 py-6 text-center text-[12.5px] leading-relaxed text-mute">
          自动探测窗口与控件中的倍速控制，
          <br />
          适用于常见网课客户端（实验特性）
        </p>
      )}

      {/* 这两条讲的是该应用**能力**上的边界（倍速只能落在哪些值上），
          与编辑器里当前选的控制方式无关，因此放在各分支之外常驻 */}
      {app.keyRate && (
        <div className="mt-4">
          <KeyRateNote keyRate={app.keyRate} />
        </div>
      )}
      {app.rateLadder && (
        <div className="mt-4">
          <RateLadderNote ladder={app.rateLadder} />
        </div>
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
  const apps = useAppStore((s) => s.apps);
  const selectedId = useAppStore((s) => s.selectedAppId);
  const setSelected = useAppStore((s) => s.setSelectedApp);

  const q = query.trim().toLowerCase();
  const list = apps.filter(
    (a) => a.name.toLowerCase().includes(q) || a.process.toLowerCase().includes(q),
  );
  // 默认选中：命中 id → 首个「需要设置」的应用（最需要用户动手）→ 列表首项
  const selected =
    apps.find((a) => a.id === selectedId) ??
    apps.find((a) => a.status === "needs-setup") ??
    apps[0] ??
    null;

  const onRowClick = (app: AppInfo) => {
    setSelected(app.id);
    if (app.kind === "browser") setExpandedId((cur) => (cur === app.id ? null : app.id));
  };

  const rowTitle = (app: AppInfo) => {
    if (app.kind === "browser") return "点击展开 / 收起网站适配";
    if (!app.running) return `未检测到 ${app.process} 运行，规则仍可编辑`;
    return undefined;
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
        {/* 应用列表（数据源：store.apps，Rust list_apps / apps:status-changed 实时刷新） */}
        <div className="flat-card min-w-0 overflow-hidden rounded-2xl @3xl:flex-1">
          <div className="grid grid-cols-[minmax(0,1.3fr)_96px_118px_20px] items-center gap-2 border-b border-line px-5 py-3 text-[12.5px] text-mute">
            <span>应用</span>
            <span>状态</span>
            <span>控制方式</span>
            <span />
          </div>
          {list.map((app) => {
            const expanded = expandedId === app.id;
            return (
              <Fragment key={app.id}>
                <button
                  onClick={() => onRowClick(app)}
                  title={rowTitle(app)}
                  className={cn(
                    "grid w-full grid-cols-[minmax(0,1.3fr)_96px_118px_20px] items-center gap-2 border-b border-line px-5 py-3 text-left transition-colors last:border-b-0",
                    selectedId === app.id ? "bg-accent-soft/70" : "hover:bg-card-2/60",
                    // 未运行的应用弱化显示（规则仍可选中编辑）
                    !app.running && "opacity-55",
                  )}
                >
                  <span className="flex min-w-0 items-center gap-3">
                    <AppIcon id={brandOf(app.id, app.process)} size={34} />
                    <span className="truncate text-sm font-semibold">{app.name}</span>
                  </span>
                  <span className="flex items-center gap-1.5 text-[13px] text-ink-2">
                    <span
                      className={cn(
                        "size-2 shrink-0 rounded-full",
                        app.running ? dotColor[app.status] : "bg-mute",
                      )}
                    />
                    {app.running ? statusText[app.status] : "未运行"}
                  </span>
                  <span className="truncate text-[13px] text-ink-2">{app.methodLabel}</span>
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
              {apps.length === 0 ? "正在识别系统中的应用…" : `没有匹配「${query}」的应用`}
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
            <p className="mt-1 text-[13px] text-mute">进程：{selected?.process ?? "—"}</p>
          </div>
          {selected && <RuleEditor app={selected} />}
        </aside>
      </div>
    </div>
  );
}
