import { cn } from "../lib/cn";

export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  format = String,
}: {
  options: readonly T[];
  value: T;
  onChange: (value: T) => void;
  format?: (value: T) => string;
}) {
  return (
    <div className="flex items-center rounded-full border border-line bg-card p-[3px]">
      {options.map((opt) => (
        <button
          key={String(opt)}
          type="button"
          onClick={() => onChange(opt)}
          className={cn(
            "rounded-full px-3.5 py-1 text-[13px] font-medium transition-colors",
            opt === value
              ? "bg-accent-soft font-semibold text-accent"
              : "text-ink-2 hover:text-ink",
          )}
        >
          {format(opt)}
        </button>
      ))}
    </div>
  );
}
