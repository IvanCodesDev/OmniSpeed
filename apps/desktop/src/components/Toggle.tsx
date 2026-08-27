import { cn } from "../lib/cn";

export function Toggle({
  checked,
  onChange,
  small = false,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  small?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative shrink-0 rounded-full transition-colors duration-200",
        small ? "h-[18px] w-[32px]" : "h-[26px] w-[46px]",
        checked ? "bg-accent" : "bg-[#d6d9de]",
      )}
    >
      <span
        className={cn(
          "absolute rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.3)] transition-[left] duration-200",
          small
            ? cn("top-[2px] size-3.5", checked ? "left-[16px]" : "left-[2px]")
            : cn("top-[3px] size-5", checked ? "left-[23px]" : "left-[3px]"),
        )}
      />
    </button>
  );
}
