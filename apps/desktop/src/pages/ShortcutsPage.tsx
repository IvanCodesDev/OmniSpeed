import { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ArrowDown, ArrowUp, Keyboard, Pencil, SkipForward, TriangleAlert } from "lucide-react";
import keycaps from "../assets/clay/keycaps.png";
import { Toggle } from "../components/Toggle";
import { shortcutItems, type ShortcutItem } from "../data";
import { cn } from "../lib/cn";
import { useAppStore, type ShortcutId } from "../store";

function ShortcutChip({ kind }: { kind: ShortcutItem["kind"] }) {
  const base = "grid size-8 shrink-0 place-items-center rounded-[10px]";
  switch (kind) {
    case "up":
      return (
        <span className={cn(base, "bg-[#3b82f6]/14 text-[#2f6fe0]")}>
          <ArrowUp size={15} strokeWidth={2.4} />
        </span>
      );
    case "down":
      return (
        <span className={cn(base, "bg-[#3b82f6]/14 text-[#2f6fe0]")}>
          <ArrowDown size={15} strokeWidth={2.4} />
        </span>
      );
    case "reset":
      return (
        <span className={cn(base, "bg-emerald-500/14 text-[11px] font-bold text-emerald-600")}>
          1.0
        </span>
      );
    case "playpause":
      return (
        <span className={cn(base, "bg-violet-500/14 text-violet-600")}>
          <SkipForward size={14} fill="currentColor" strokeWidth={0} />
        </span>
      );
    case "panel":
      return (
        <span className={cn(base, "bg-amber-400/18 text-amber-600")}>
          <Keyboard size={15} />
        </span>
      );
  }
}

const MODIFIERS = ["Control", "Alt", "Shift", "Meta"];

function normalizeKey(key: string): string | null {
  if (MODIFIERS.includes(key)) return null;
  if (key === " ") return "Space";
  if (key === "ArrowUp") return "↑";
  if (key === "ArrowDown") return "↓";
  if (key === "ArrowLeft") return "←";
  if (key === "ArrowRight") return "→";
  return key.length === 1 ? key.toUpperCase() : key;
}

/** 录制快捷键弹层：监听按键组合，Esc 取消 */
function RecordDialog({
  label,
  onConfirm,
  onClose,
}: {
  label: string;
  onConfirm: (combo: string[]) => void;
  onClose: () => void;
}) {
  const [mods, setMods] = useState<string[]>([]);
  const [main, setMain] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        onClose();
        return;
      }
      const nextMods = [
        e.ctrlKey && "Ctrl",
        e.altKey && "Alt",
        e.shiftKey && "Shift",
        e.metaKey && "Win",
      ].filter(Boolean) as string[];
      setMods(nextMods);
      setMain(normalizeKey(e.key));
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  const combo = main ? [...mods, main] : mods;
  const valid = main !== null && (mods.includes("Ctrl") || mods.includes("Alt") || mods.includes("Win"));

  return (
    <div className="fixed inset-0 z-[60] grid place-items-center bg-black/25" onClick={onClose}>
      <div
        className="flat-card w-[380px] rounded-2xl p-6 shadow-[0_24px_48px_-24px_rgba(23,26,31,0.35)]"
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className="text-[15px] font-bold">录制快捷键 · {label}</h3>
        <div className="mt-4 grid h-14 place-items-center rounded-xl border border-dashed border-line bg-card-2/50 text-[15px] font-semibold">
          {combo.length > 0 ? (
            combo.join(" + ")
          ) : (
            <span className="font-normal text-mute">按下新的组合键…</span>
          )}
        </div>
        <p className="mt-2.5 text-xs leading-relaxed text-mute">
          全局快捷键需包含 Ctrl / Alt / Win 修饰键；按 Esc 取消录制。
        </p>
        <div className="mt-5 flex justify-end gap-3">
          <button
            onClick={onClose}
            className="h-9 rounded-xl border border-line bg-card px-4 text-[13px] font-medium text-ink-2 transition-colors hover:bg-card-2"
          >
            取消
          </button>
          <button
            disabled={!valid}
            onClick={() => valid && onConfirm(combo)}
            className={cn(
              "h-9 rounded-xl px-5 text-[13px] font-semibold transition-colors",
              valid ? "bg-accent text-on-accent" : "cursor-not-allowed bg-card-2 text-mute",
            )}
          >
            确定
          </button>
        </div>
      </div>
    </div>
  );
}

export function ShortcutsPage() {
  const enabled = useAppStore((s) => s.hotkeysEnabled);
  const setEnabled = useAppStore((s) => s.setHotkeysEnabled);
  const step = useAppStore((s) => s.settings.step);
  const shortcuts = useAppStore((s) => s.shortcuts);
  const setShortcut = useAppStore((s) => s.setShortcut);
  const resetShortcuts = useAppStore((s) => s.resetShortcuts);

  const conflicts = useAppStore((s) => s.conflicts);
  const saveShortcuts = useAppStore((s) => s.saveShortcuts);

  const [recordingId, setRecordingId] = useState<ShortcutId | null>(null);
  const [justSaved, setJustSaved] = useState(false);
  const saveTimer = useRef<number>(undefined);
  useEffect(() => () => window.clearTimeout(saveTimer.current), []);

  /**
   * 冲突信息：条目间重复在录制后即时提示；
   * 系统/其他程序占用来自 Rust 侧注册结果（保存后刷新）
   */
  const conflictOf = (id: ShortcutId): string | null => {
    const key = shortcuts[id].join("+");
    const dup = shortcutItems.find((it) => it.id !== id && shortcuts[it.id].join("+") === key);
    if (dup) return `与「${dup.label}」重复`;
    return conflicts[id] ?? null;
  };

  const recordingItem = shortcutItems.find((it) => it.id === recordingId);

  return (
    <div>
      <header className="flex items-start justify-between gap-6">
        <div>
          <h1 className="text-[26px] font-bold tracking-tight">全局快捷键</h1>
          <p className="mt-1.5 text-[13px] text-mute">在任何应用中快速调整播放速度</p>
        </div>
        <div className="flex items-center gap-3 pt-1.5">
          <span className="text-[13px] font-medium text-ink-2">启用全局快捷键</span>
          <Toggle checked={enabled} onChange={setEnabled} />
        </div>
      </header>

      {/* 键帽演示卡（键帽为设计稿原图素材）：图随容器流式缩放，窄容器时上下堆叠 */}
      <section className="@container relative mt-6 overflow-hidden rounded-[28px] border border-line bg-[#f8f8f8] px-8 py-5">
        <div className="flex items-center justify-between gap-x-10 gap-y-3 @max-xl:flex-col @max-xl:py-2">
          <img
            src={keycaps}
            className="block w-[min(440px,58cqw)] select-none @max-xl:w-full @max-xl:max-w-[420px]"
            alt=""
            draggable={false}
          />
          <div className="py-4 pr-10 @max-xl:p-0 @max-xl:pb-1 @max-xl:text-center">
            <div className="text-[25px] font-bold tracking-tight">调节速度</div>
            <div className="mt-1.5 text-[15px] text-mute">每次 ±{step}×</div>
          </div>
        </div>
      </section>

      {/* 快捷键设置列表 */}
      <section className="mt-7">
        <h2 className="mb-3 text-[15px] font-semibold">快捷键设置</h2>
        <div className={cn("flat-card overflow-hidden rounded-2xl transition-opacity", !enabled && "opacity-55")}>
          {shortcutItems.map((item, i) => {
            const conflict = conflictOf(item.id);
            return (
              <div
                key={item.id}
                className={cn("flex items-center gap-3.5 px-5 py-3", i > 0 && "border-t border-line")}
              >
                <ShortcutChip kind={item.kind} />
                <span className="text-sm font-medium">{item.label}</span>
                {conflict && (
                  <span className="ml-3 flex items-center gap-1 text-xs font-medium text-amber-500">
                    <TriangleAlert size={13} />
                    {conflict}
                  </span>
                )}
                <span className="ml-auto text-[13px] font-medium text-ink-2">
                  {shortcuts[item.id].join(" + ")}
                </span>
                <button
                  onClick={() => setRecordingId(item.id)}
                  className="ml-2 text-mute transition-colors hover:text-accent"
                  aria-label={`重新录制「${item.label}」快捷键`}
                >
                  <Pencil size={14} />
                </button>
              </div>
            );
          })}
        </div>

        <div className="mt-5 flex justify-end gap-3">
          <button
            onClick={resetShortcuts}
            className="h-10 rounded-xl border border-line bg-card px-5 text-sm font-medium text-ink-2 transition-colors hover:bg-card-2"
          >
            恢复默认
          </button>
          <button
            onClick={() => {
              void saveShortcuts().then(() => {
                setJustSaved(true);
                window.clearTimeout(saveTimer.current);
                saveTimer.current = window.setTimeout(() => setJustSaved(false), 1500);
              });
            }}
            className={cn(
              "h-10 rounded-xl px-6 text-sm font-semibold transition-all active:scale-[0.98]",
              justSaved ? "bg-emerald-500 text-white" : "bg-accent text-on-accent",
            )}
          >
            {justSaved ? "已保存 ✓" : "保存更改"}
          </button>
        </div>
      </section>

      <AnimatePresence>
        {recordingItem && (
          <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: 0.12 }}>
            <RecordDialog
              label={recordingItem.label}
              onConfirm={(combo) => {
                setShortcut(recordingItem.id, combo);
                setRecordingId(null);
              }}
              onClose={() => setRecordingId(null)}
            />
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}
