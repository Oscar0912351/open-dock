import { ArrowDown, ArrowUp, Cpu, GraphicsCard, HardDrives, Memory } from "@phosphor-icons/react";
import { useEffect, useRef, useState } from "react";
import type { SystemCaps } from "../../../shared/bindings/SystemCaps";
import type { SystemSample } from "../../../shared/bindings/SystemSample";
import type { ThemeId } from "../../../shared/bindings/ThemeId";
import type { WidgetSize } from "../../../shared/bindings/WidgetSize";
import {
  formatBytes,
  formatFrequencyGhz,
  formatPercent,
  formatSpeed,
  formatTemperature,
} from "../../../shared/format";
import { useProvider } from "../../../shared/hooks/useProvider";
import { t } from "../../../shared/i18n";
import { ipc } from "../../../shared/ipc";
import { Skeleton, StateError } from "../../components/States";
import { Ring, type MetricTone } from "./Ring";
import { Sparkline } from "./Sparkline";
import { useSystemHistory } from "./useSystemHistory";
import "./system.css";

export type SystemMetric = "cpu" | "mem" | "gpu" | "vram" | "net" | "disk";

export interface SystemSettings {
  metrics: SystemMetric[];
  primary: SystemMetric;
  showTemps: boolean;
  unitsBits: boolean;
}

export const defaultSystemSettings: SystemSettings = {
  metrics: ["cpu", "mem", "gpu"],
  primary: "cpu",
  showTemps: true,
  unitsBits: false,
};

const VALID_METRICS: readonly SystemMetric[] = ["cpu", "mem", "gpu", "vram", "net", "disk"];

export function resolveSystemSettings(raw: Record<string, unknown> | undefined): SystemSettings {
  if (!raw) return defaultSystemSettings;
  const metrics = Array.isArray(raw.metrics)
    ? raw.metrics.filter(
        (item): item is SystemMetric =>
          typeof item === "string" && (VALID_METRICS as readonly string[]).includes(item),
      )
    : [];
  const primary =
    typeof raw.primary === "string" && (VALID_METRICS as readonly string[]).includes(raw.primary)
      ? (raw.primary as SystemMetric)
      : defaultSystemSettings.primary;
  return {
    metrics: metrics.length > 0 ? metrics : defaultSystemSettings.metrics,
    primary,
    showTemps: raw.showTemps === undefined ? true : Boolean(raw.showTemps),
    unitsBits: Boolean(raw.unitsBits),
  };
}

interface MetricDisplay {
  key: SystemMetric;
  label: string;
  shortLabel: string;
  supported: boolean;
  pct: number | null;
  valueText: string;
  integerPart: string;
  unitPart: string;
  secondaryText: string;
  tempText: string;
  tempValue: number | null;
}

interface SystemProps {
  size: WidgetSize;
  theme: ThemeId;
  settings: SystemSettings;
  onAriaChange?: (aria: string) => void;
}

export function System({ size, theme, settings, onAriaChange }: SystemProps) {
  const { status, data: sample, showSkeleton } = useProvider("system");
  const history = useSystemHistory(true, sample);
  const [caps, setCaps] = useState<SystemCaps | null>(null);
  const lastAriaBucketRef = useRef<string>("");

  useEffect(() => {
    let active = true;
    ipc
      .systemCaps()
      .then((c) => {
        if (active) setCaps(c);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, []);

  useEffect(() => {
    if (!sample || !onAriaChange) return;
    const cpuBucket = Math.round(sample.cpu.usage / 5) * 5;
    const memGb = Math.round(Number(sample.mem.usedBytes) / (1024 * 1024 * 1024));
    const gpuBucket =
      sample.gpu?.usage !== null && sample.gpu?.usage !== undefined
        ? Math.round(sample.gpu.usage / 5) * 5
        : -1;
    const bucketKey = `${cpuBucket}:${memGb}:${gpuBucket}`;
    if (lastAriaBucketRef.current === bucketKey) return;
    lastAriaBucketRef.current = bucketKey;

    const cpuStr = `${Math.round(sample.cpu.usage)} procent`;
    const used = formatBytes(Number(sample.mem.usedBytes));
    const total = formatBytes(Number(sample.mem.totalBytes));
    const gpuStr =
      sample.gpu?.usage !== null && sample.gpu?.usage !== undefined
        ? `, GPU ${Math.round(sample.gpu.usage)} procent`
        : "";
    onAriaChange(`System: CPU ${cpuStr}, RAM ${used} af ${total}${gpuStr}`);
  }, [sample, onAriaChange]);

  const computeTone = (metric: SystemMetric, pct: number | null, tempC: number | null): MetricTone => {
    if (pct !== null && pct >= 95 && sample && sustainedHighMs(history, sample, metric) >= 5000) {
      return "crit";
    }
    if (tempC !== null && tempC >= 95) return "crit";
    if ((pct !== null && pct >= 85) || (tempC !== null && tempC >= 85)) return "warn";
    return "normal";
  };

  if (status === "error") {
    return <StateError />;
  }

  if (!sample) {
    if (!showSkeleton) return null;
    return <SystemSkeleton size={size} />;
  }

  const readMetric = (metric: SystemMetric): MetricDisplay => {
    switch (metric) {
      case "cpu": {
        const pct = Math.min(100, Math.max(0, sample.cpu.usage));
        const freq = sample.cpu.freqMhz ? formatFrequencyGhz(sample.cpu.freqMhz) : "";
        const coreCount = sample.cpu.coreCount || sample.cpu.perCore.length;
        return {
          key: "cpu",
          label: t("system.cpu"),
          shortLabel: t("system.cpu"),
          supported: true,
          pct,
          valueText: formatPercent(pct / 100),
          integerPart: `${Math.round(pct)}`,
          unitPart: "%",
          secondaryText: freq || `${coreCount} kerner`,
          tempText:
            settings.showTemps && sample.cpu.tempC !== null ? formatTemperature(sample.cpu.tempC) : "",
          tempValue: sample.cpu.tempC,
        };
      }
      case "mem": {
        const used = Number(sample.mem.usedBytes);
        const total = Math.max(1, Number(sample.mem.totalBytes));
        const ratio = Math.min(1, Math.max(0, used / total));
        const pct = ratio * 100;
        const usedShort = formatBytes(used).replace(/\u202F?(B|kB|MB|GB|TB)$/, "");
        const totalText = formatBytes(total);
        return {
          key: "mem",
          label: t("system.mem"),
          shortLabel: t("system.mem"),
          supported: true,
          pct,
          valueText: formatPercent(ratio),
          integerPart: `${Math.round(pct)}`,
          unitPart: "%",
          secondaryText: `${usedShort} / ${totalText}`,
          tempText: "",
          tempValue: null,
        };
      }
      case "gpu": {
        const supported = (caps?.gpu ?? true) && sample.gpu !== null && sample.gpu.usage !== null;
        if (!supported) {
          return {
            key: "gpu",
            label: t("system.gpu"),
            shortLabel: t("system.gpu"),
            supported: false,
            pct: null,
            valueText: t("state.dash"),
            integerPart: t("state.dash"),
            unitPart: "",
            secondaryText: t("system.unsupported"),
            tempText: "",
            tempValue: null,
          };
        }
        const pct = Math.min(100, Math.max(0, sample.gpu!.usage!));
        const tempC = sample.gpu?.tempC ?? null;
        return {
          key: "gpu",
          label: t("system.gpu"),
          shortLabel: t("system.gpu"),
          supported: true,
          pct,
          valueText: formatPercent(pct / 100),
          integerPart: `${Math.round(pct)}`,
          unitPart: "%",
          secondaryText: sample.gpu?.name ?? t("system.gpu"),
          tempText: settings.showTemps && tempC !== null ? formatTemperature(tempC) : "",
          tempValue: tempC,
        };
      }
      case "vram": {
        const used = sample.gpu?.vramUsedBytes;
        const total = sample.gpu?.vramTotalBytes;
        const supported =
          (caps?.vram ?? true) && used !== null && used !== undefined && total !== null && total !== undefined && Number(total) > 0;
        if (!supported) {
          return {
            key: "vram",
            label: t("system.vram"),
            shortLabel: t("system.vram"),
            supported: false,
            pct: null,
            valueText: t("state.dash"),
            integerPart: t("state.dash"),
            unitPart: "",
            secondaryText: t("system.unsupported"),
            tempText: "",
            tempValue: null,
          };
        }
        const u = Number(used);
        const tot = Number(total);
        const pct = Math.min(100, Math.max(0, (u / tot) * 100));
        return {
          key: "vram",
          label: t("system.vram"),
          shortLabel: t("system.vram"),
          supported: true,
          pct,
          valueText: formatBytes(u),
          integerPart: formatBytes(u),
          unitPart: "",
          secondaryText: `/ ${formatBytes(tot)}`,
          tempText: "",
          tempValue: null,
        };
      }
      case "net": {
        const down = formatSpeed(sample.net.rxBps, settings.unitsBits);
        const up = formatSpeed(sample.net.txBps, settings.unitsBits);
        const pct = Math.min(100, (sample.net.rxBps / (100 * 1024 * 1024)) * 100);
        return {
          key: "net",
          label: t("system.net"),
          shortLabel: "Net",
          supported: true,
          pct,
          valueText: `↓ ${down}`,
          integerPart: down,
          unitPart: "",
          secondaryText: `↑ ${up}`,
          tempText: "",
          tempValue: null,
        };
      }
      case "disk": {
        const supported = caps?.disk ?? true;
        if (!supported) {
          return {
            key: "disk",
            label: t("system.disk"),
            shortLabel: t("system.disk"),
            supported: false,
            pct: null,
            valueText: t("state.dash"),
            integerPart: t("state.dash"),
            unitPart: "",
            secondaryText: t("system.unsupported"),
            tempText: "",
            tempValue: null,
          };
        }
        const read = formatSpeed(sample.disk.readBps, false);
        const write = formatSpeed(sample.disk.writeBps, false);
        const pct = sample.disk.activePct ?? Math.min(
          100,
          ((sample.disk.readBps + sample.disk.writeBps) / (500 * 1024 * 1024)) * 100,
        );
        return {
          key: "disk",
          label: t("system.disk"),
          shortLabel: t("system.disk"),
          supported: true,
          pct,
          valueText: read,
          integerPart: read,
          unitPart: "",
          secondaryText: write,
          tempText: "",
          tempValue: null,
        };
      }
    }
  };

  if (size === "pill") {
    const pair = settings.metrics.slice(0, 2);
    return (
      <div className="row g8 sys-pill">
        {pair.map((metricKey, idx) => {
          const info = readMetric(metricKey);
          const tone = computeTone(metricKey, info.pct, info.tempValue);
          return (
            <div key={metricKey} className="row g8 grow" style={{ minWidth: 0, display: "contents" }}>
              {idx > 0 && <div className="vsep sys-pill-vsep" />}
              {metricKey === "net" ? (
                <div className="col grow sys-pill-net" data-enter>
                  <div className="row sys-pill-net-row">
                    <ArrowDown size="calc(10 * var(--u))" />
                    <span className="value trunc sys-pill-val" style={{ fontSize: "calc(11 * var(--u))" }}>
                      {formatSpeed(sample.net.rxBps, settings.unitsBits)}
                    </span>
                  </div>
                  <div className="row sys-pill-net-row">
                    <ArrowUp size="calc(10 * var(--u))" />
                    <span className="micro trunc">{formatSpeed(sample.net.txBps, settings.unitsBits)}</span>
                  </div>
                </div>
              ) : (
                <div className="row g6 grow sys-pill-metric" style={{ minWidth: 0 }} data-enter>
                  <Ring size={28} stroke={2.8} pct={info.pct ?? 0} tone={tone}>
                    <MetricIcon metric={metricKey} size={11} />
                  </Ring>
                  <div className="col" style={{ minWidth: 0 }}>
                    <div className="value sys-pill-val">{info.valueText}</div>
                    <div className="micro sys-pill-lab trunc">
                      {info.supported ? info.label : t("system.unsupported")}
                    </div>
                  </div>
                </div>
              )}
            </div>
          );
        })}
      </div>
    );
  }

  if (size === "s") {
    const primary = readMetric(settings.primary);
    const tone = computeTone(settings.primary, primary.pct, primary.tempValue);
    const freq = sample.cpu.freqMhz ? formatFrequencyGhz(sample.cpu.freqMhz) : "";
    const topRight =
      settings.primary === "cpu"
        ? [freq, primary.tempText].filter(Boolean).join(" · ")
        : primary.tempText;
    const coreCount = sample.cpu.coreCount || sample.cpu.perCore.length;
    const footer =
      settings.primary === "cpu"
        ? `${coreCount} kerner`
        : primary.secondaryText;

    return (
      <>
        <div className="row between" data-enter>
          <span className="label">{primary.label}</span>
          <span className="micro">{topRight}</span>
        </div>
        <div className="grow sys-s-ring-wrap">
          <Ring size={74} stroke={6} pct={primary.pct ?? 0} tone={tone}>
            {primary.supported ? (
              <div className="display-m sys-s-num">
                {primary.integerPart}
                {primary.unitPart && <span className="unit">{primary.unitPart}</span>}
              </div>
            ) : (
              <div className="display-m ink4 sys-s-num">{t("state.dash")}</div>
            )}
          </Ring>
        </div>
        <div className="micro sys-s-foot trunc" data-enter>
          {primary.supported ? footer : t("system.unsupported")}
        </div>
      </>
    );
  }

  if (size === "m") {
    const uptimeS = Number(sample.uptimeS);
    const hours = Math.floor(uptimeS / 3600);
    const mins = Math.floor((uptimeS % 3600) / 60);
    const cols = settings.metrics.slice(0, 3);

    return (
      <>
        <div className="row between" data-enter>
          <span className="label">System</span>
          <span className="micro">{t("system.uptime", { h: hours, m: mins })}</span>
        </div>
        <div className="row g10 grow sys-m-cols">
          {cols.map((metricKey) => {
            const m = readMetric(metricKey);
            const tone = computeTone(metricKey, m.pct, m.tempValue);
            return (
              <div key={metricKey} className="col grow" style={{ minWidth: 0 }} data-enter>
                <div className="row between">
                  <span className="label sys-m-label">{m.label}</span>
                  <span className="micro">{m.tempText}</span>
                </div>
                <div className={`display-s sys-m-val${m.supported ? "" : " ink4"}`}>
                  {m.valueText}
                </div>
                <div className={`bar${tone !== "normal" ? ` ${tone}` : ""}`}>
                  {m.pct !== null && <i style={{ "--v": Math.min(1, Math.max(0, m.pct / 100)) } as React.CSSProperties} />}
                </div>
                <div className="micro sys-m-sub trunc">{m.secondaryText}</div>
              </div>
            );
          })}
        </div>
      </>
    );
  }

  // Size: "l" (276x276)
  const primaryKey = settings.primary;
  const primary = readMetric(primaryKey);
  const sparkValues = history.map((item) => extractHistoryPct(item, primaryKey));
  const sparkMax = primaryKey === "net" || primaryKey === "disk" ? Math.max(10, ...sparkValues) : 100;
  const gridMetrics: SystemMetric[] = [
    ...settings.metrics.filter((m) => m !== primaryKey),
    ...VALID_METRICS.filter((m) => m !== primaryKey && !settings.metrics.includes(m)),
  ].slice(0, 4);

  return (
    <>
      <div className="row between" data-enter>
        <span className="label">{primary.label}</span>
        <span className="micro">{primary.tempText || primary.secondaryText}</span>
      </div>
      <div className="display-l sys-l-primary" data-enter>
        {primary.supported ? (
          <>
            {primary.integerPart}
            {primary.unitPart && <span className="unit">{primary.unitPart}</span>}
          </>
        ) : (
          <span className="ink4">{t("state.dash")}</span>
        )}
      </div>
      <div className="sys-l-spark" data-enter>
        <Sparkline values={sparkValues} width={244} height={60} theme={theme} min={0} max={sparkMax} />
      </div>
      <div className="grow" />
      <div className="sys-l-grid">
        {gridMetrics.map((metricKey) => {
          const m = readMetric(metricKey);
          const tone = computeTone(metricKey, m.pct, m.tempValue);
          return (
            <div key={metricKey} className="od-panel col sys-l-card" data-enter>
              <div className="row between g4">
                <span className="label sys-l-card-label">{m.shortLabel}</span>
                <span className={`value sys-l-card-val${m.supported ? "" : " ink4"}`}>
                  {m.valueText}
                </span>
              </div>
              <div className={`bar${tone !== "normal" ? ` ${tone}` : ""}`}>
                {m.pct !== null && (
                  <i style={{ "--v": Math.min(1, Math.max(0, m.pct / 100)) } as React.CSSProperties} />
                )}
              </div>
            </div>
          );
        })}
      </div>
    </>
  );
}

function MetricIcon({ metric, size }: { metric: SystemMetric; size: number }) {
  const px = `calc(${size} * var(--u))`;
  switch (metric) {
    case "cpu":
      return <Cpu size={px} />;
    case "mem":
      return <Memory size={px} />;
    case "gpu":
    case "vram":
      return <GraphicsCard size={px} />;
    default:
      return <HardDrives size={px} />;
  }
}

function sustainedHighMs(
  history: SystemSample[],
  current: SystemSample,
  metric: SystemMetric,
): number {
  let earliest = Number(current.t);
  for (let i = history.length - 1; i >= 0; i--) {
    const row = history[i];
    if (!row || extractHistoryPct(row, metric) < 95) break;
    earliest = Number(row.t);
  }
  return Math.max(0, Number(current.t) - earliest);
}

function extractHistoryPct(sample: SystemSample, metric: SystemMetric): number {
  switch (metric) {
    case "cpu":
      return Math.min(100, Math.max(0, sample.cpu.usage));
    case "mem": {
      const total = Math.max(1, Number(sample.mem.totalBytes));
      return Math.min(100, Math.max(0, (Number(sample.mem.usedBytes) / total) * 100));
    }
    case "gpu":
      return Math.min(100, Math.max(0, sample.gpu?.usage ?? 0));
    case "vram": {
      const total = Math.max(1, Number(sample.gpu?.vramTotalBytes ?? 1));
      return Math.min(100, Math.max(0, (Number(sample.gpu?.vramUsedBytes ?? 0) / total) * 100));
    }
    case "net":
      return sample.net.rxBps / 1024;
    case "disk":
      return sample.disk.activePct ?? (sample.disk.readBps + sample.disk.writeBps) / 1024;
  }
}

function SystemSkeleton({ size }: { size: WidgetSize }) {
  if (size === "pill") {
    return (
      <div className="row g10" style={{ height: "100%" }}>
        <Skeleton width={28} height={28} circle />
        <div className="col grow g4">
          <Skeleton width="65%" height={11} />
          <Skeleton width="40%" height={8} />
        </div>
        <Skeleton width={28} height={28} circle />
        <div className="col grow g4">
          <Skeleton width="65%" height={11} />
          <Skeleton width="40%" height={8} />
        </div>
      </div>
    );
  }
  if (size === "s") {
    return (
      <div className="col between" style={{ height: "100%", alignItems: "center" }}>
        <div className="row between" style={{ width: "100%" }}>
          <Skeleton width={32} height={10} />
          <Skeleton width={42} height={9} />
        </div>
        <Skeleton width={74} height={74} circle />
        <Skeleton width={54} height={9} />
      </div>
    );
  }
  if (size === "m") {
    return (
      <div className="col between" style={{ height: "100%" }}>
        <div className="row between">
          <Skeleton width={52} height={10} />
          <Skeleton width={68} height={9} />
        </div>
        <div className="row g10">
          {[0, 1, 2].map((i) => (
            <div key={i} className="col grow g6">
              <Skeleton width="45%" height={9} />
              <Skeleton width="75%" height={20} />
              <Skeleton width="100%" height={3} radius={99} />
              <Skeleton width="65%" height={9} />
            </div>
          ))}
        </div>
      </div>
    );
  }
  return (
    <div className="col between" style={{ height: "100%" }}>
      <div className="row between">
        <Skeleton width={36} height={10} />
        <Skeleton width={48} height={9} />
      </div>
      <Skeleton width={92} height={36} />
      <Skeleton width="100%" height={60} radius={10} />
      <div className="sys-l-grid">
        {[0, 1, 2, 3].map((i) => (
          <Skeleton key={i} width="100%" height={48} radius={12} />
        ))}
      </div>
    </div>
  );
}
