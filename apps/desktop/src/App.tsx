import { AnimatePresence, motion } from "framer-motion";
import { Sidebar } from "./components/Sidebar";
import { Titlebar } from "./components/Titlebar";
import { AppsPage } from "./pages/AppsPage";
import { HomePage } from "./pages/HomePage";
import { SettingsPage } from "./pages/SettingsPage";
import { ShortcutsPage } from "./pages/ShortcutsPage";
import { useAppStore } from "./store";

const pages = {
  home: HomePage,
  apps: AppsPage,
  shortcuts: ShortcutsPage,
  settings: SettingsPage,
} as const;

export default function App() {
  const page = useAppStore((s) => s.page);
  const Page = pages[page];

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-canvas font-app text-ink antialiased">
      <Titlebar />
      <Sidebar />
      <main className="h-full min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-[920px] px-9 pb-10 pt-12">
          <AnimatePresence mode="wait" initial={false}>
            <motion.div
              key={page}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: 0.16, ease: "easeOut" }}
            >
              <Page />
            </motion.div>
          </AnimatePresence>
        </div>
      </main>
    </div>
  );
}
