import { Minus, Square, X } from "lucide-react";
import { closeWindow, minimizeWindow, toggleMaximizeWindow } from "../lib/windowControls";

/** 无边框窗口的自定义标题栏：整条为拖拽区，右侧三个窗口控制按钮 */
export function Titlebar() {
  return (
    <div className="fixed inset-x-0 top-0 z-50 flex h-9 items-stretch justify-end" data-tauri-drag-region>
      <button
        aria-label="最小化"
        onClick={() => void minimizeWindow()}
        className="grid w-11 place-items-center text-ink-2 transition-colors hover:bg-black/5"
      >
        <Minus size={15} strokeWidth={1.8} />
      </button>
      <button
        aria-label="最大化"
        onClick={() => void toggleMaximizeWindow()}
        className="grid w-11 place-items-center text-ink-2 transition-colors hover:bg-black/5"
      >
        <Square size={11.5} strokeWidth={1.8} />
      </button>
      <button
        aria-label="关闭"
        onClick={() => void closeWindow()}
        className="grid w-11 place-items-center text-ink-2 transition-colors hover:bg-[#e81123] hover:text-white"
      >
        <X size={15} strokeWidth={1.8} />
      </button>
    </div>
  );
}
