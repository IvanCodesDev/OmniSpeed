import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { AnimatePresence, motion } from "framer-motion";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "../lib/cn";

export interface SelectOption<T extends string | number> {
  value: T;
  label: string;
}

const GAP = 6;
const MAX_MENU_H = 260;
const EDGE = 8;

interface MenuPos {
  left: number;
  top: number;
  minWidth: number;
  maxHeight: number;
}

/**
 * 下拉选择器：替代原生 `<select>`。
 * 原生 `<option>` 列表由系统绘制、无法套用主题，因此选项面板自绘并 portal 到 body
 * （应用页的表格卡片是 overflow-hidden，绝对定位会被裁切）。
 */
export function Select<T extends string | number>({
  options,
  value,
  onChange,
  size = "md",
  className,
  "aria-label": ariaLabel,
}: {
  options: readonly SelectOption<T>[];
  value: T;
  onChange: (value: T) => void;
  size?: "sm" | "md";
  className?: string;
  "aria-label"?: string;
}) {
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const [pos, setPos] = useState<MenuPos | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const listId = useId();

  const selectedIndex = Math.max(
    0,
    options.findIndex((o) => o.value === value),
  );
  const selected = options[selectedIndex];
  const sm = size === "sm";

  const openMenu = () => {
    setActiveIndex(selectedIndex);
    setOpen(true);
  };

  const commit = (index: number) => {
    const opt = options[index];
    if (opt) onChange(opt.value);
    setOpen(false);
    triggerRef.current?.focus();
  };

  // 定位：下方空间不足则向上翻转，并把面板夹在视口内
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const trigger = triggerRef.current?.getBoundingClientRect();
      const menu = menuRef.current;
      if (!trigger || !menu) return;

      const below = window.innerHeight - trigger.bottom - GAP - EDGE;
      const above = trigger.top - GAP - EDGE;
      const wanted = Math.min(MAX_MENU_H, menu.scrollHeight);
      const flip = below < wanted && above > below;
      const maxHeight = Math.max(96, Math.min(MAX_MENU_H, flip ? above : below));
      const height = Math.min(menu.scrollHeight, maxHeight);

      setPos({
        left: Math.max(EDGE, Math.min(trigger.left, window.innerWidth - menu.offsetWidth - EDGE)),
        top: flip ? trigger.top - GAP - height : trigger.bottom + GAP,
        minWidth: trigger.width,
        maxHeight,
      });
    };

    place();
    // 捕获阶段才能收到内层滚动容器的事件
    window.addEventListener("scroll", place, true);
    window.addEventListener("resize", place);
    return () => {
      window.removeEventListener("scroll", place, true);
      window.removeEventListener("resize", place);
    };
  }, [open]);

  useEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (triggerRef.current?.contains(target) || menuRef.current?.contains(target)) return;
      setOpen(false);
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    return () => window.removeEventListener("pointerdown", onPointerDown, true);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    menuRef.current?.children[activeIndex]?.scrollIntoView({ block: "nearest" });
  }, [open, activeIndex]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (!open) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        openMenu();
      }
      return;
    }
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setActiveIndex((i) => Math.min(options.length - 1, i + 1));
        break;
      case "ArrowUp":
        e.preventDefault();
        setActiveIndex((i) => Math.max(0, i - 1));
        break;
      case "Home":
        e.preventDefault();
        setActiveIndex(0);
        break;
      case "End":
        e.preventDefault();
        setActiveIndex(options.length - 1);
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        commit(activeIndex);
        break;
      case "Escape":
        e.preventDefault();
        setOpen(false);
        break;
      case "Tab":
        setOpen(false);
        break;
    }
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        role="combobox"
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-controls={open ? listId : undefined}
        aria-activedescendant={open ? `${listId}-${activeIndex}` : undefined}
        aria-label={ariaLabel}
        onClick={() => (open ? setOpen(false) : openMenu())}
        onKeyDown={onKeyDown}
        className={cn(
          "flex items-center justify-between gap-1 border border-line bg-card text-ink transition-colors",
          "outline-none hover:border-[#d8dade] focus-visible:border-accent/60",
          open && "border-accent/60",
          sm
            ? "rounded-lg py-[3px] pl-2 pr-1.5 text-[12px] text-ink-2"
            : "rounded-xl py-2 pl-4 pr-3 text-[13px] font-medium",
          className,
        )}
      >
        <span className="truncate">{selected?.label}</span>
        <ChevronDown
          size={sm ? 12 : 14}
          className={cn("shrink-0 text-mute transition-transform duration-150", open && "rotate-180")}
        />
      </button>

      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={menuRef}
              id={listId}
              role="listbox"
              initial={{ opacity: 0, scale: 0.97 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.97 }}
              transition={{ duration: 0.12, ease: "easeOut" }}
              style={{
                left: pos?.left ?? 0,
                top: pos?.top ?? 0,
                minWidth: pos?.minWidth,
                maxHeight: pos?.maxHeight,
                visibility: pos ? "visible" : "hidden",
              }}
              className={cn(
                "fixed z-[70] overflow-y-auto overscroll-contain rounded-xl border border-line bg-card p-1",
                "shadow-[0_14px_36px_-14px_rgba(23,26,31,0.3),0_2px_8px_-4px_rgba(23,26,31,0.12)]",
              )}
            >
              {options.map((opt, i) => {
                const isSelected = i === selectedIndex;
                return (
                  <div
                    key={String(opt.value)}
                    id={`${listId}-${i}`}
                    role="option"
                    aria-selected={isSelected}
                    onPointerEnter={() => setActiveIndex(i)}
                    onClick={() => commit(i)}
                    className={cn(
                      "flex cursor-pointer items-center justify-between gap-4 rounded-lg px-2.5 py-[7px] text-[13px] whitespace-nowrap transition-colors",
                      isSelected ? "font-semibold text-accent" : "text-ink-2",
                      i === activeIndex && (isSelected ? "bg-accent-soft" : "bg-card-2"),
                    )}
                  >
                    {opt.label}
                    {isSelected && <Check size={13.5} strokeWidth={2.6} className="shrink-0" />}
                  </div>
                );
              })}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}
