import type { ReactNode } from "react";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { Toggle } from "../components/Toggle";
import { presetCandidates } from "../data";
import { cn } from "../lib/cn";
import { formatRate, SLIDER_MAX_OPTIONS, useAppStore, type StepSize } from "../store";

function Row({ label, children, last }: { label: string; children: ReactNode; last?: boolean }) {
  return (
    <div className={cn("flex items-center justify-between gap-6 py-3", !last && "border-b border-line")}>
      <span className="text-sm">{label}</span>
      {children}
    </div>
  );
}

/** 预设档位编辑：点击切换档位是否出现在控制页 */
function PresetsEditor() {
  const presets = useAppStore((s) => s.settings.presets);
  const update = useAppStore((s) => s.updateSettings);

  const togglePreset = (v: number) => {
    const active = presets.includes(v);
    if (active && presets.length <= 1) return; // 至少保留一个档位
    const next = active ? presets.filter((p) => p !== v) : [...presets, v].sort((a, b) => a - b);
    update({ presets: next });
  };

  return (
    <div className="border-b border-line py-3">
      <div className="flex items-center justify-between gap-6">
        <span className="text-sm">预设档位</span>
        <span className="text-xs text-mute">点击增减，控制页同步更新</span>
      </div>
      <div className="mt-2.5 flex flex-wrap gap-2">
        {presetCandidates.map((v) => {
          const active = presets.includes(v);
          return (
            <button
              key={v}
              onClick={() => togglePreset(v)}
              className={cn(
                "rounded-full border px-3 py-1 text-[12.5px] font-medium transition-colors",
                active
                  ? "border-accent/50 bg-accent-soft font-semibold text-accent"
                  : "border-line bg-card text-mute hover:text-ink-2",
              )}
            >
              {formatRate(v)}×
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function SettingsPage() {
  const settings = useAppStore((s) => s.settings);
  const update = useAppStore((s) => s.updateSettings);
  const updateAvailable = useAppStore((s) => s.updateAvailable);
  const updating = useAppStore((s) => s.updating);
  const installUpdateNow = useAppStore((s) => s.installUpdateNow);

  return (
    <div>
      <header>
        <h1 className="text-[26px] font-bold tracking-tight">设置</h1>
        <p className="mt-1.5 text-[13px] text-mute">调整 OmniSpeed 的运行方式</p>
      </header>

      {/* 播放控制 / 系统 */}
      <section className="flat-card mt-6 rounded-2xl px-6 pb-2">
        <h2 className="pb-1 pt-4 text-[15px] font-bold">播放控制</h2>
        <Row label="默认倍速">
          <Select
            aria-label="默认倍速"
            className="w-[108px]"
            value={settings.defaultRate}
            onChange={(v) => update({ defaultRate: v })}
            options={[0.5, 0.75, 1, 1.25, 1.5, 2].map((v) => ({
              value: v,
              label: `${formatRate(v)}×`,
            }))}
          />
        </Row>
        <Row label="调节步长">
          <Segmented
            options={[0.1, 0.25, 0.5] as const}
            value={settings.step}
            onChange={(v) => update({ step: v as StepSize })}
            format={(v) => `${v}×`}
          />
        </Row>
        <Row label="倍速上限">
          <div className="flex items-center gap-3">
            <span className="text-xs text-mute">滑块显示范围，浏览器内核上限 16×</span>
            <Segmented
              options={SLIDER_MAX_OPTIONS}
              value={settings.sliderMax as (typeof SLIDER_MAX_OPTIONS)[number]}
              onChange={(v) => update({ sliderMax: v })}
              format={(v) => `${v}×`}
            />
          </div>
        </Row>
        <PresetsEditor />
        <Row label="高倍速提示（>4× 浏览器静音）">
          <Toggle checked={settings.highSpeedWarning} onChange={(v) => update({ highSpeedWarning: v })} />
        </Row>
        <Row label="智能降速（缓冲不足自动回落）">
          <Toggle checked={settings.smartSlowdown} onChange={(v) => update({ smartSlowdown: v })} />
        </Row>
        <Row label="变速不变调">
          <Toggle checked={settings.preservesPitch} onChange={(v) => update({ preservesPitch: v })} />
        </Row>
        <Row label="记住每个应用的倍速" last>
          <Toggle checked={settings.rememberPerApp} onChange={(v) => update({ rememberPerApp: v })} />
        </Row>

        <h2 className="border-t border-line pb-1 pt-4 text-[15px] font-bold">系统</h2>
        <Row label="开机自动启动">
          <Toggle checked={settings.startOnBoot} onChange={(v) => update({ startOnBoot: v })} />
        </Row>
        <Row label="关闭窗口时最小化到托盘">
          <Toggle checked={settings.minimizeToTray} onChange={(v) => update({ minimizeToTray: v })} />
        </Row>
        <Row label="自动检查更新" last>
          <Toggle checked={settings.autoUpdate} onChange={(v) => update({ autoUpdate: v })} />
        </Row>
      </section>

      <footer className="mt-6 flex items-center justify-between border-t border-line pb-2 pt-4 text-[13px] text-mute">
        <span className="flex items-center gap-3">
          <span>OmniSpeed 0.1.0</span>
          {updateAvailable && (
            <button
              onClick={() => void installUpdateNow()}
              disabled={updating}
              className="font-medium text-accent hover:underline disabled:cursor-default disabled:opacity-60"
            >
              {updating ? "正在更新…" : `发现新版本 v${updateAvailable} · 更新并重启`}
            </button>
          )}
        </span>
        <span className="flex items-center gap-3">
          <a className="font-medium text-accent hover:underline" href="#license">
            开源许可
          </a>
          <span className="text-line">|</span>
          <a className="font-medium text-accent hover:underline" href="#github">
            GitHub
          </a>
        </span>
      </footer>
    </div>
  );
}
