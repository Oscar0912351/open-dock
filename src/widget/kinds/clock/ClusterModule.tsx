import type { ClusterModuleRenderProps } from "../../cluster/clusterModuleShared";
import { memo, type MouseEvent } from "react";
import type { ClusterModule } from "../../../shared/bindings/ClusterModule";
import { formatDateLong, formatDateShort, formatTime, timeZoneCity } from "../../../shared/format";
import { t, useLocale } from "../../../shared/i18n";
import { resolveClockSettings, validTimeZone } from "./Clock";
import { useClock } from "./useClock";
import { RollingNumber } from "./RollingNumber";
import { shouldIgnoreExpand, viewTransitionStyle, HintBadge } from "../../cluster/clusterModuleShared";

const ClockClusterMod = memo(function ClockClusterMod({
  mod,
  mode,
  now,
  preview,
  onToggleExpand,
}: {
  mod: ClusterModule;
  mode: "compact" | "expanded";
  now: Date;
  preview?: boolean;
  onToggleExpand: (id: string) => void;
}) {
  useLocale();
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    if (!shouldIgnoreExpand(e)) onToggleExpand(mod.id);
  };
  const settings = resolveClockSettings(mod.settings);
  const tz = validTimeZone(settings.timezone);
  const timeStr = formatTime(now, tz ?? undefined, settings.showSeconds);

  if (mode === "expanded") {
    const zones = (
      settings.extraZones.length > 0 ? settings.extraZones : ["Asia/Tokyo", "America/New_York"]
    )
      .filter((z): z is string => validTimeZone(z) !== null)
      .slice(0, 2);

    return (
      <div
        className="c-mod is-exp c-mod-clock-exp"
        style={viewTransitionStyle(mod.id, preview)}
        onClick={onClick}
      >
        <div className="row between">
          <div className="col">
            <div className="display-m" style={{ fontSize: "calc(30 * var(--u))" }}>
              <RollingNumber value={timeStr} />
            </div>
            <div className="meta">{formatDateLong(now, tz ?? undefined)}</div>
          </div>
          <div className="col g4" style={{ alignItems: "flex-end" }}>
            {zones.map((zone) => (
              <div className="row g8" key={zone}>
                <span className="micro">{timeZoneCity(zone)}</span>
                <span className="value" style={{ fontSize: "calc(12 * var(--u))" }}>
                  {formatTime(now, zone)}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className="c-mod c-mod-clock-cmp"
      style={viewTransitionStyle(mod.id, preview)}
      onClick={onClick}
    >
      <div className="row between">
        <div className="row g8">
          <span className="display-s" style={{ fontSize: "calc(19 * var(--u))" }}>
            <RollingNumber value={timeStr} />
          </span>
          <span className="meta">{formatDateShort(now, tz ?? undefined)}</span>
        </div>
        <HintBadge label={t("cluster.more")} />
      </div>
    </div>
  );
});

export function ClockClusterModule({ mod, mode, shared, onToggleExpand }: ClusterModuleRenderProps) {
  const now = useClock(resolveClockSettings(mod.settings).showSeconds);
  return (
    <ClockClusterMod
      mod={mod}
      mode={mode}
      now={now}
      preview={shared.preview}
      onToggleExpand={onToggleExpand}
    />
  );
}
