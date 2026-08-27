import { useMemo, type ReactNode } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { BadgeCheck, ChevronRight, MonitorPlay, TriangleAlert } from "lucide-react";
import { AppIcon } from "../components/AppIcon";
import { brandOf, recentMedia, type AppStatus, type MediaSession } from "../data";
import { cn } from "../lib/cn";
import { makeSliderScale } from "../lib/sliderScale";
import { formatRate, MAX_RATE, SLIDER_MIN, useAppStore } from "../store";

function ListeningPill() {
  const listening = useAppStore((s) => s.listening);
  const toggle = useAppStore((s) => s.toggleListening);
  return (
    <button
      onClick={toggle}
      title="点击暂停 / 恢复全局监听"
      className="flex items-center gap-2 rounded-full px-2 py-1 text-[13px] text-ink-2 transition-colors hover:bg-card-2"
    >
      {listening ? "全局监听中" : "监听已暂停"}
      <span className={cn("size-2.5 rounded-full", listening ? "bg-emerald-500" : "bg-mute")} />
    </button>
  );
}

/** 接管状态徽章（PRD §7.1：已接管 / 已适配 / 需要设置 三态，颜色区分） */
const mediaBadge: Record<AppStatus, { label: string; cls: string; icon: ReactNode }> = {
  connected: { label: "已接管", cls: "bg-emerald-500/10 text-emerald-600", icon: <BadgeCheck size={13} /> },
  adapted: { label: "已适配", cls: "bg-accent-soft text-accent", icon: <BadgeCheck size={13} /> },
  "needs-setup": { label: "需要设置", cls: "bg-amber-400/15 text-amber-600", icon: <TriangleAlert size={13} /> },
};

/** 当前媒体卡片（无接管对象或监听暂停时显示空态引导） */
function MediaCard({ media }: { media: MediaSession | null }) {
  const setPage = useAppStore((s) => s.setPage);

  if (!media) {
    return (
      <div className="flat-card flex w-[248px] shrink-0 flex-col items-center justify-center gap-2 rounded-[28px] p-6 text-center">
        <MonitorPlay size={40} strokeWidth={1.3} className="text-mute" />
        <div className="mt-1 text-[15px] font-semibold">未检测到正在播放的媒体</div>
        <p className="text-xs leading-relaxed text-mute">
          播放视频后将自动识别，或到
          <button onClick={() => setPage("apps")} className="mx-0.5 font-medium text-accent hover:underline">
            「应用」
          </button>
          页查看支持的软件
        </p>
      </div>
    );
  }

  const badge = mediaBadge[media.status];
  return (
    <div className="relative w-[248px] shrink-0">
      {/* 叠放的卡片层：用灰底 + 实色边框在浅色画布上拉开层次 */}
      <span className="absolute -left-3 -top-3 h-full w-full rotate-[-5deg] rounded-[28px] border border-line bg-card-2" />
      <span className="absolute -left-1.5 -top-1.5 h-full w-full rotate-[-2.5deg] rounded-[28px] border border-line bg-[#fafafb]" />
      <div className="flat-card relative flex h-full flex-col items-center justify-center gap-1 rounded-[28px] p-6 text-center">
        <AppIcon id={brandOf(media.appId, media.source)} size={80} className="drop-shadow-[0_5px_8px_rgba(0,0,0,0.1)]" />
        <div className="mt-4 text-[19px] font-bold">{media.name}</div>
        <div className="text-[13px] text-mute">{media.source}</div>
        <span className={cn("mt-2.5 inline-flex items-center gap-1 rounded-full px-3 py-1 text-xs font-medium", badge.cls)}>
          {badge.icon} {badge.label}
        </span>
      </div>
    </div>
  );
}

export function HomePage() {
  // 大倍速显示直接用 store.rate：初始拉取与 media:changed 已用当前媒体的真实倍速覆盖它
  const rate = useAppStore((s) => s.rate);
  const setRate = useAppStore((s) => s.setRate);
  const applyRate = useAppStore((s) => s.applyRate);
  const applyToCurrentMedia = useAppStore((s) => s.applyToCurrentMedia);
  const listening = useAppStore((s) => s.listening);
  const currentMedia = useAppStore((s) => s.currentMedia);
  const update = useAppStore((s) => s.updateSettings);
  const { sliderMax, presets, highSpeedWarning } = useAppStore((s) => s.settings);

  const scale = useMemo(() => makeSliderScale(sliderMax), [sliderMax]);
  const t = scale.rateToT(rate);
  // 监听暂停时视为无接管对象（空态 + 控件禁用）
  const media = listening ? currentMedia : null;
  const hasMedia = media !== null;

  return (
    <div>
      <header className="flex items-center justify-between">
        <h1 className="text-[22px] font-bold tracking-tight">当前媒体</h1>
        <ListeningPill />
      </header>

      {/* 遥控器主区：直接落在画布上，不再套一层背景盒子（上边距为叠卡层留出探出空间） */}
      <section className="mt-11 flex items-stretch gap-9">
        <MediaCard media={media} />

        {/* 调速控制区 */}
        <div
          className={cn(
            "flex min-w-0 flex-1 flex-col items-center justify-center gap-5 px-2 py-1 transition-opacity",
            !hasMedia && "pointer-events-none opacity-40",
          )}
        >
          <div className="flex items-baseline font-black leading-none tracking-tight">
            <motion.span
              key={rate}
              initial={{ scale: 0.95, opacity: 0.6 }}
              animate={{ scale: 1, opacity: 1 }}
              transition={{ type: "spring", stiffness: 400, damping: 24 }}
              className="text-[68px]"
            >
              {formatRate(rate)}
            </motion.span>
            <span className="ml-1.5 text-[42px]">×</span>
          </div>

          {/* 分段刻度滑块：1×–2× 区间加密（PRD §7.1） */}
          <div className="w-full">
            <input
              type="range"
              aria-label="播放速度"
              className="speed-slider w-full"
              min={0}
              max={1000}
              step={1}
              value={Math.round(t * 1000)}
              onChange={(e) => {
                const raw = scale.tToRate(Number(e.target.value) / 1000);
                setRate(Math.round(raw * 20) / 20);
              }}
              title={`${formatRate(SLIDER_MIN)}× – ${formatRate(sliderMax)}×`}
              style={{
                background: `linear-gradient(to right, var(--accent) 0%, var(--accent) ${t * 100}%, var(--slider-track) ${t * 100}%)`,
              }}
            />
            {/* 显式标出滑块量程，并让 16× 上限一键可达（PRD §7.4「倍速上限」默认 6×） */}
            <div className="mt-2 flex items-center justify-between text-[11.5px] text-mute">
              <span>{formatRate(SLIDER_MIN)}×</span>
              {sliderMax < MAX_RATE ? (
                <button
                  onClick={() => update({ sliderMax: MAX_RATE })}
                  title={`当前滑块上限 ${sliderMax}×，点击提升到浏览器内核上限 ${MAX_RATE}×`}
                  className="font-medium text-accent transition-colors hover:underline"
                >
                  上限 {sliderMax}× · 提升到 {MAX_RATE}×
                </button>
              ) : (
                <span>{MAX_RATE}×（内核上限）</span>
              )}
            </div>
          </div>

          {/* >4× 高倍速静音提示（PRD §7.1 / §7.6） */}
          <AnimatePresence initial={false}>
            {highSpeedWarning && rate > 4 && (
              <motion.div
                initial={{ opacity: 0, y: -6 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: -6 }}
                transition={{ duration: 0.15 }}
                className="flex items-center gap-1.5 rounded-full bg-amber-400/15 px-4 py-1.5 text-xs font-medium text-amber-600"
              >
                <TriangleAlert size={13} />
                超过 4× 后浏览器将静音音频，仅保留画面速览
                <button
                  onClick={() => setRate(4)}
                  className="ml-1 font-semibold text-accent hover:underline"
                >
                  回到 4×
                </button>
              </motion.div>
            )}
          </AnimatePresence>

          <div className="flex flex-wrap items-center justify-center gap-2.5">
            {presets.map((p) => (
              <button
                key={p}
                onClick={() => applyRate(p)}
                className={cn(
                  "rounded-full border px-4 py-2 text-[13px] font-semibold transition-colors",
                  rate === p
                    ? "border-accent bg-accent-soft text-accent"
                    : "border-line bg-card text-ink-2 hover:bg-card-2 hover:text-ink",
                )}
              >
                {formatRate(p)}×
              </button>
            ))}
          </div>

          <button
            disabled={!hasMedia}
            onClick={() => void applyToCurrentMedia()}
            className="mt-1.5 h-[52px] w-full rounded-full bg-accent text-[15px] font-semibold text-on-accent transition-transform active:scale-[0.98]"
          >
            应用到当前媒体
          </button>
        </div>
      </section>

      {/* 最近媒体（点击恢复其上次倍速，PRD §7.1）；真实历史数据 M4 接入，当前为占位展示 */}
      <section className="mt-9">
        <h2 className="mb-3 text-[15px] font-semibold">最近媒体</h2>
        <div className="flat-card overflow-hidden rounded-xl">
          {recentMedia.map((m, i) => (
            <button
              key={m.id}
              onClick={() => applyRate(m.rate)}
              title={`重新接管并恢复 ${formatRate(m.rate)}×`}
              className={cn(
                "flex w-full items-center gap-3.5 px-5 py-3 text-left transition-colors hover:bg-card-2/60",
                i > 0 && "border-t border-line",
              )}
            >
              <AppIcon id={m.icon} size={32} />
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-semibold">{m.name}</span>
                <span className="block truncate text-xs text-mute">{m.source}</span>
              </span>
              <span className="text-[13px] font-medium text-ink-2">{formatRate(m.rate)}×</span>
              <span className="w-20 text-right text-xs text-mute">{m.time}</span>
              <ChevronRight size={16} className="text-mute" />
            </button>
          ))}
        </div>
      </section>
    </div>
  );
}
