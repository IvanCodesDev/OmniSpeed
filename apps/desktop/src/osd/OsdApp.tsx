import { useEffect, useRef, useState } from "react";
import { ArrowDown, ArrowUp, Pause, RotateCcw } from "lucide-react";
import { cn } from "../lib/cn";
import { formatRate, type ShortcutId } from "../store";
import { onHotkeyTriggered, type HotkeyPayload } from "../lib/ipc";

/** Rust 在 1500ms 后隐藏窗口，前端提前开始淡出（350ms 过渡） */
const FADE_AFTER_MS = 1100;

/** 动作图标与语义色（PRD §13.2：加速=珊瑚、减速=琥珀、恢复=绿） */
function ActionBadge({ action }: { action: ShortcutId }) {
  const base = "grid size-9 shrink-0 place-items-center rounded-full";
  switch (action) {
    case "speedUp":
      return (
        <span className={cn(base, "bg-[#fb7185]/25 text-[#fda4af]")}>
          <ArrowUp size={19} strokeWidth={2.6} />
        </span>
      );
    case "speedDown":
      return (
        <span className={cn(base, "bg-[#fbbf24]/22 text-[#fcd34d]")}>
          <ArrowDown size={19} strokeWidth={2.6} />
        </span>
      );
    case "reset":
      return (
        <span className={cn(base, "bg-emerald-400/22 text-emerald-300")}>
          <RotateCcw size={17} strokeWidth={2.4} />
        </span>
      );
    default:
      return (
        <span className={cn(base, "bg-violet-400/22 text-violet-300")}>
          <Pause size={17} fill="currentColor" strokeWidth={0} />
        </span>
      );
  }
}

/**
 * 全局 OSD 悬浮内容（独立透明窗口，PRD §7.5）。
 * 窗口的显示/定位/隐藏由 Rust 控制，这里只负责内容与淡出动画。
 */
export function OsdApp() {
  const [payload, setPayload] = useState<HotkeyPayload | null>(null);
  const [fading, setFading] = useState(false);
  const timer = useRef<number>(undefined);

  useEffect(() => {
    const unlisten = onHotkeyTriggered((p) => {
      setPayload(p);
      setFading(false);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setFading(true), FADE_AFTER_MS);
    });
    return () => {
      void unlisten.then((fn) => fn?.());
      window.clearTimeout(timer.current);
    };
  }, []);

  if (!payload) return null;

  return (
    <div className="grid h-screen w-screen place-items-center">
      <div key={payload.seq} className={cn("osd-pill", fading && "osd-pill-fading")}>
        <ActionBadge action={payload.action} />
        {payload.action === "playPause" ? (
          <span className="text-[21px] font-bold tracking-wide">暂停 / 继续</span>
        ) : (
          <span className="flex items-baseline">
            <span className="text-[34px] font-black leading-none tracking-tight">
              {formatRate(payload.rate)}
            </span>
            <span className="ml-0.5 text-[21px] font-bold">×</span>
          </span>
        )}
      </div>
    </div>
  );
}
