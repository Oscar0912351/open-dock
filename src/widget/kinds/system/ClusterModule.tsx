import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { memo, useState, type MouseEvent } from "react";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import type { SystemCaps } from "../../../shared/bindings/SystemCaps";
import type { SystemSample } from "../../../shared/bindings/SystemSample";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import { formatBytes, formatFrequencyGhz, formatSpeed, formatTemperature } from "../../../shared/format";
import { t, useLocale } from "../../../shared/i18n";
import { Sparkline } from "./Sparkline";
import { resolveSystemSettings } from "./System";
import { shouldIgnoreExpand, viewTransitionStyle, HintBadge } from "../../cluster/clusterModuleShared";

const SystemClusterMod = memo(function SystemClusterMod({
  mod,
  mode,
  theme,
  sample,
  history,
  caps,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  theme: ThemeId;
  sample: SystemSample | null;
  history: SystemSample[];
  caps: SystemCaps | null;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveSystemSettings(mod.settings);
  const cpuPct = sample ? Math.round(sample.cpu.usage) : 0;
  const memPct = sample && sample.mem.totalBytes > 0
    ? Math.round((sample.mem.usedBytes / sample.mem.totalBytes) * 100)
    : 0;
  const memGb = sample ? formatBytes(sample.mem.usedBytes) : t("state.dash");
  const gpuSupported = caps?.gpu ?? Boolean(sample?.gpu);
  const gpuPct = sample?.gpu?.usage !== null && sample?.gpu?.usage !== undefined
    ? Math.round(sample.gpu.usage)
    : null;
  const netDown = sample
    ? formatSpeed(sample.net.rxBps, settings.unitsBits)
    : t("state.dash");

  const [barSmooth, setBarSmooth] = useState<{
    cpu: number;
    mem: number;
    gpu: number;
    smoothCpu: boolean;
    smoothMem: boolean;
    smoothGpu: boolean;
  }>({ cpu: cpuPct, mem: memPct, gpu: gpuPct ?? 0, smoothCpu: false, smoothMem: false, smoothGpu: false });

  const gpuNum = gpuPct ?? 0;
  if (barSmooth.cpu !== cpuPct || barSmooth.mem !== memPct || barSmooth.gpu !== gpuNum) {
    setBarSmooth({
      cpu: cpuPct,
      mem: memPct,
      gpu: gpuNum,
      smoothCpu: Math.abs(cpuPct - barSmooth.cpu) >= 3,
      smoothMem: Math.abs(memPct - barSmooth.mem) >= 3,
      smoothGpu: Math.abs(gpuNum - barSmooth.gpu) >= 3,
    });
  }

  const barClass = (pct: number, smooth: boolean) => {
    const tone = pct >= 95 ? "is-crit" : pct >= 85 ? "is-warn" : "";
    return [tone, smooth ? "is-smooth" : ""].filter(Boolean).join(" ");
  };

  if (mode === "expanded") {
    const pts = history.slice(-60).map((s) => s.cpu.usage);
    const freqText = sample?.cpu.freqMhz ? formatFrequencyGhz(sample.cpu.freqMhz) : "";
    const cpuTempText =
      settings.showTemps && sample?.cpu.tempC !== null && sample?.cpu.tempC !== undefined
        ? `${t("system.cpu")} ${formatTemperature(sample.cpu.tempC)}`
        : "";
    const gpuTempText =
      settings.showTemps && sample?.gpu?.tempC !== null && sample?.gpu?.tempC !== undefined
        ? `${t("system.gpu")} ${formatTemperature(sample.gpu.tempC)}`
        : "";
    const rightMeta = [freqText, cpuTempText, gpuTempText].filter(Boolean).join(" · ");

    return (
      <div
        className="c-mod is-exp c-mod-sys-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between g8">
          <span className="label">{t("system.cpu")}</span>
          <span className="micro trunc">{rightMeta}</span>
        </div>
        <div className="row between" style={{ alignItems: "flex-end", marginTop: "calc(2 * var(--u))" }}>
          <span className="display-s" style={{ fontSize: "calc(24 * var(--u))" }}>
            {sample ? cpuPct : "—"}
            <span className="unit">%</span>
          </span>
          <span className="micro trunc">
            RAM {memGb} · {gpuSupported && gpuPct !== null ? `GPU ${gpuPct} %` : `Net ${netDown}`}
          </span>
        </div>
        <div className="c-spark" style={{ marginTop: "calc(4 * var(--u))" }}>
          <Sparkline
            values={pts.length >= 2 ? pts : [cpuPct, cpuPct]}
            width={200}
            height={36}
            theme={theme}
          />
        </div>
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-sys-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between g8">
        <div className="c-sys-stat">
          <span className="micro">
            CPU <b style={{ color: "var(--ink-1)" }}>{sample ? `${cpuPct}%` : "—"}</b>
          </span>
          <div className="c-mini-bar">
            <i className={barClass(cpuPct, barSmooth.smoothCpu)} style={{ transform: `scaleX(${Math.min(1, cpuPct / 100)})` }} />
          </div>
        </div>
        <div className="c-sys-stat">
          <span className="micro">
            RAM <b style={{ color: "var(--ink-1)" }}>{sample ? `${memPct}%` : "—"}</b>
          </span>
          <div className="c-mini-bar">
            <i className={barClass(memPct, barSmooth.smoothMem)} style={{ transform: `scaleX(${Math.min(1, memPct / 100)})` }} />
          </div>
        </div>
        <div className="c-sys-stat">
          <span className="micro">
            GPU{" "}
            <b style={{ color: "var(--ink-1)" }}>
              {gpuSupported && gpuPct !== null ? `${gpuPct}%` : t("state.dash")}
            </b>
          </span>
          <div className="c-mini-bar">
            <i
              className={gpuPct !== null ? barClass(gpuPct, barSmooth.smoothGpu) : ""}
              style={{ transform: `scaleX(${gpuPct !== null ? Math.min(1, gpuPct / 100) : 0})` }}
            />
          </div>
        </div>
        <HintBadge label={t("cluster.graph")} />
      </div>
    </div>
  );
});

export function SystemClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
  return (
    <SystemClusterMod
      mod={mod}
      mode={mode}
      theme={shared.theme}
      sample={shared.system.data}
      history={shared.systemHistory}
      caps={shared.systemCaps}
      preview={shared.preview}
      onToggleExpand={onToggleExpand}
    />
  );
}
