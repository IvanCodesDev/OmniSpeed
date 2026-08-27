import { CirclePlay, Keyboard, LayoutGrid, Settings, type LucideIcon } from "lucide-react";
import logo from "../assets/brands/omnispeed.png";
import { cn } from "../lib/cn";
import { useAppStore, type Page } from "../store";

const navItems: { id: Page; label: string; icon: LucideIcon }[] = [
  { id: "home", label: "控制", icon: CirclePlay },
  { id: "apps", label: "应用", icon: LayoutGrid },
  { id: "shortcuts", label: "快捷键", icon: Keyboard },
  { id: "settings", label: "设置", icon: Settings },
];

export function Sidebar() {
  const page = useAppStore((s) => s.page);
  const setPage = useAppStore((s) => s.setPage);

  return (
    <aside className="flex h-full w-[218px] shrink-0 flex-col bg-rail px-3.5 pb-6 pt-12">
      <div className="mb-9 flex items-center gap-2.5 px-3">
        <img src={logo} className="h-[26px] w-auto shrink-0 select-none" alt="" draggable={false} />
        <span className="text-[16px] font-bold tracking-tight">OmniSpeed</span>
      </div>

      <nav className="flex flex-col gap-1">
        {navItems.map(({ id, label, icon: Icon }) => {
          const active = page === id;
          return (
            <button
              key={id}
              onClick={() => setPage(id)}
              className={cn(
                "flex items-center gap-3.5 rounded-2xl px-4.5 py-3.5 text-[15px] transition-colors",
                active
                  ? "bg-black/[0.055] font-semibold text-accent"
                  : "font-medium text-ink-2 hover:bg-black/[0.03]",
              )}
            >
              <Icon size={19} strokeWidth={active ? 2.1 : 1.9} />
              {label}
            </button>
          );
        })}
      </nav>
    </aside>
  );
}
