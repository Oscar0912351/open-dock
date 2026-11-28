import type { ThemeId } from "../../../shared/bindings/ThemeId";
import { Clock, resolveClockSettings, type ClockSize } from "./Clock";
import { useClock } from "./useClock";

export function LiveClock({
  settings,
  size,
  theme,
}: {
  settings: Record<string, unknown>;
  size: ClockSize;
  theme: ThemeId;
}) {
  const resolved = resolveClockSettings(settings);
  const now = useClock(resolved.showSeconds);
  return <Clock now={now} settings={resolved} size={size} theme={theme} />;
}
