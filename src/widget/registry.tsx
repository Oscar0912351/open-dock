import type { ReactNode } from "react";
import type { Location } from "../shared/bindings/Location";
import type { ThemeId } from "../shared/bindings/ThemeId";
import type { WidgetKind } from "../shared/bindings/WidgetKind";
import type { WidgetSize } from "../shared/bindings/WidgetSize";
import { Agents, defaultAgentsSettings, resolveAgentsSettings } from "./kinds/agents/Agents";
import {
  Clock,
  defaultClockSettings,
  resolveClockSettings,
  type ClockSize,
} from "./kinds/clock/Clock";
import { Discord } from "./kinds/discord/Discord";
import { Note, defaultNoteSettings, resolveNoteSettings } from "./kinds/note/Note";
import { Timer, defaultTimerSettings, resolveTimerSettings } from "./kinds/timer/Timer";
import {
  NowPlaying,
  defaultNowPlayingSettings,
  resolveNowPlayingSettings,
} from "./kinds/nowPlaying/NowPlaying";
import { System, defaultSystemSettings, resolveSystemSettings } from "./kinds/system/System";
import { Weather, defaultWeatherSettings, resolveWeatherSettings } from "./kinds/weather/Weather";

export type WidgetSettingField =
  | {
      key: string;
      type: "segmented" | "select";
      labelKey: string;
      options: Array<{ value: string; labelKey: string }>;
      helpKey?: string;
    }
  | {
      key: string;
      type: "switch";
      labelKey: string;
      helpKey?: string;
    }
  | {
      key: string;
      type: "slider";
      labelKey: string;
      min: number;
      max: number;
      step: number;
      unit?: string;
    }
  | {
      key: string;
      type: "text";
      labelKey: string;
      placeholderKey?: string;
      maxLength?: number;
    }
  | {
      key: string;
      type: "timezone" | "extraZones" | "metrics" | "weatherLocation";
      labelKey: string;
      helpKey?: string;
    };

export type FreeWidgetKind = Exclude<WidgetKind, "cluster">;

/** Alt hvad en widget-komponent kan få brug for — samles ét sted pr. site
    (widget-vindue eller kontrolcenter-preview) og gives videre til registryens
    render-funktion, så kind→komponent-afhængigheden kun findes ét sted. */
export interface WidgetRenderContext {
  id: string;
  /** Rå størrelse fra instansen — renderWidget clamper til allowedSizes før render. */
  size: WidgetSize;
  theme: ThemeId;
  settings: Record<string, unknown>;
  now: Date;
  editMode: boolean;
  preview: boolean;
  pausedApp: boolean;
  locations: Location[];
  onAriaChange?: (aria: string) => void;
  onAmbientChange?: (rgb: [number, number, number] | null) => void;
  onHiddenIdleChange?: (hidden: boolean) => void;
  onPinVisibleChange?: (pinned: boolean) => void;
}

export interface WidgetMeta {
  kind: FreeWidgetKind;
  nameKey: string;
  descriptionKey: string;
  allowedSizes: readonly WidgetSize[];
  defaultSize: WidgetSize;
  defaultSettings: Record<string, unknown>;
  settingsSchema: readonly WidgetSettingField[];
  render: (ctx: WidgetRenderContext) => ReactNode;
}

export const WIDGET_REGISTRY: Record<FreeWidgetKind, WidgetMeta> = {
  clock: {
    kind: "clock",
    nameKey: "widgets.clock.name",
    descriptionKey: "widgets.clock.desc",
    allowedSizes: ["pill", "s", "m"],
    defaultSize: "m",
    defaultSettings: { ...defaultClockSettings },
    render: ({ now, settings, size, theme }) => (
      <Clock
        now={now}
        settings={resolveClockSettings(settings)}
        size={size as ClockSize}
        theme={theme}
      />
    ),
    settingsSchema: [
      {
        key: "style",
        type: "segmented",
        labelKey: "widgetSettings.clock.style.label",
        options: [
          {
            value: "digital",
            labelKey: "widgetSettings.clock.style.options.digital"
          },
          {
            value: "analog",
            labelKey: "widgetSettings.clock.style.options.analog"
          }
        ]
      },
      {
        key: "showSeconds",
        type: "switch",
        labelKey: "widgetSettings.clock.showSeconds.label"
      },
      {
        key: "showDate",
        type: "switch",
        labelKey: "widgetSettings.clock.showDate.label"
      },
      {
        key: "timezone",
        type: "timezone",
        labelKey: "widgetSettings.clock.timezone.label"
      },
      {
        key: "label",
        type: "text",
        labelKey: "widgetSettings.clock.label.label",
        placeholderKey: "widgetSettings.clock.label.placeholder",
        maxLength: 32
      },
      {
        key: "extraZones",
        type: "extraZones",
        labelKey: "widgetSettings.clock.extraZones.label"
      }
    ],
  },
  nowPlaying: {
    kind: "nowPlaying",
    nameKey: "widgets.nowPlaying.name",
    descriptionKey: "widgets.nowPlaying.desc",
    allowedSizes: ["pill", "s", "m", "l"],
    defaultSize: "m",
    defaultSettings: { ...defaultNowPlayingSettings },
    render: ({
      size,
      theme,
      settings,
      pausedApp,
      onAmbientChange,
      onHiddenIdleChange,
      onAriaChange,
    }) => (
      <NowPlaying
        size={size}
        theme={theme}
        settings={resolveNowPlayingSettings(settings)}
        pausedApp={pausedApp}
        onAmbientChange={onAmbientChange}
        onHiddenIdleChange={onHiddenIdleChange}
        onAriaChange={onAriaChange}
      />
    ),
    settingsSchema: [
      {
        key: "preferredSource",
        type: "select",
        labelKey: "widgetSettings.nowPlaying.preferredSource.label",
        options: [
          {
            value: "auto",
            labelKey: "widgetSettings.nowPlaying.preferredSource.options.auto"
          },
          {
            value: "Spotify",
            labelKey: "widgetSettings.nowPlaying.preferredSource.options.spotify"
          }
        ]
      },
      {
        key: "ambient",
        type: "switch",
        labelKey: "widgetSettings.nowPlaying.ambient.label"
      },
      {
        key: "showProgress",
        type: "switch",
        labelKey: "widgetSettings.nowPlaying.showProgress.label"
      },
      {
        key: "hideWhenIdle",
        type: "switch",
        labelKey: "widgetSettings.nowPlaying.hideWhenIdle.label"
      }
    ],
  },
  system: {
    kind: "system",
    nameKey: "widgets.system.name",
    descriptionKey: "widgets.system.desc",
    allowedSizes: ["pill", "s", "m", "l"],
    defaultSize: "m",
    defaultSettings: { ...defaultSystemSettings },
    render: ({ size, theme, settings, onAriaChange }) => (
      <System
        size={size}
        theme={theme}
        settings={resolveSystemSettings(settings)}
        onAriaChange={onAriaChange}
      />
    ),
    settingsSchema: [
      {
        key: "metrics",
        type: "metrics",
        labelKey: "widgetSettings.system.metrics.label"
      },
      {
        key: "primary",
        type: "select",
        labelKey: "widgetSettings.system.primary.label",
        options: [
          {
            value: "cpu",
            labelKey: "widgetSettings.system.primary.options.cpu"
          },
          {
            value: "mem",
            labelKey: "widgetSettings.system.primary.options.mem"
          },
          {
            value: "gpu",
            labelKey: "widgetSettings.system.primary.options.gpu"
          },
          {
            value: "vram",
            labelKey: "widgetSettings.system.primary.options.vram"
          },
          {
            value: "net",
            labelKey: "widgetSettings.system.primary.options.net"
          },
          {
            value: "disk",
            labelKey: "widgetSettings.system.primary.options.disk"
          }
        ]
      },
      {
        key: "showTemps",
        type: "switch",
        labelKey: "widgetSettings.system.showTemps.label"
      },
      {
        key: "unitsBits",
        type: "switch",
        labelKey: "widgetSettings.system.unitsBits.label"
      }
    ],
  },
  weather: {
    kind: "weather",
    nameKey: "widgets.weather.name",
    descriptionKey: "widgets.weather.desc",
    allowedSizes: ["s", "m", "l"],
    defaultSize: "m",
    defaultSettings: { ...defaultWeatherSettings },
    render: ({ size, theme, settings, locations, onAriaChange }) => (
      <Weather
        size={size}
        theme={theme}
        settings={resolveWeatherSettings(settings)}
        locations={locations}
        onAriaChange={onAriaChange}
      />
    ),
    settingsSchema: [
      {
        key: "locationId",
        type: "weatherLocation",
        labelKey: "widgetSettings.weather.locationId.label"
      },
      {
        key: "unit",
        type: "segmented",
        labelKey: "widgetSettings.weather.unit.label",
        options: [
          {
            value: "c",
            labelKey: "widgetSettings.weather.unit.options.c"
          },
          {
            value: "f",
            labelKey: "widgetSettings.weather.unit.options.f"
          }
        ]
      },
      {
        key: "chart",
        type: "segmented",
        labelKey: "widgetSettings.weather.chart.label",
        options: [
          {
            value: "hourly",
            labelKey: "widgetSettings.weather.chart.options.hourly"
          },
          {
            value: "daily",
            labelKey: "widgetSettings.weather.chart.options.daily"
          }
        ]
      }
    ],
  },
  note: {
    kind: "note",
    nameKey: "widgets.note.name",
    descriptionKey: "widgets.note.desc",
    allowedSizes: ["s", "m", "l"],
    defaultSize: "m",
    defaultSettings: { ...defaultNoteSettings },
    render: ({ id, settings, editMode, preview, onAriaChange }) => (
      <Note
        id={id}
        settings={resolveNoteSettings(settings)}
        editMode={editMode}
        preview={preview}
        onAriaChange={onAriaChange}
      />
    ),
    settingsSchema: [
      {
        key: "title",
        type: "text",
        labelKey: "widgetSettings.note.title.label",
        placeholderKey: "widgetSettings.note.title.placeholder",
        maxLength: 32
      },
      {
        key: "mode",
        type: "segmented",
        labelKey: "widgetSettings.note.mode.label",
        options: [
          {
            value: "text",
            labelKey: "widgetSettings.note.mode.options.text"
          },
          {
            value: "checklist",
            labelKey: "widgetSettings.note.mode.options.checklist"
          }
        ]
      },
      {
        key: "fontSize",
        type: "segmented",
        labelKey: "widgetSettings.note.fontSize.label",
        options: [
          {
            value: "s",
            labelKey: "widgetSettings.note.fontSize.options.s"
          },
          {
            value: "m",
            labelKey: "widgetSettings.note.fontSize.options.m"
          },
          {
            value: "l",
            labelKey: "widgetSettings.note.fontSize.options.l"
          }
        ]
      }
    ],
  },
  timer: {
    kind: "timer",
    nameKey: "widgets.timer.name",
    descriptionKey: "widgets.timer.desc",
    allowedSizes: ["pill", "s", "m", "l"],
    defaultSize: "m",
    defaultSettings: { ...defaultTimerSettings },
    render: ({ id, settings, size, editMode, preview, onAriaChange, onPinVisibleChange }) => (
      <Timer
        id={id}
        settings={resolveTimerSettings(settings)}
        size={size}
        editMode={editMode}
        preview={preview}
        onAriaChange={onAriaChange}
        onPinVisibleChange={onPinVisibleChange}
      />
    ),
    settingsSchema: [
      {
        key: "sound",
        type: "segmented",
        labelKey: "widgetSettings.timer.sound.label",
        options: [
          {
            value: "chime",
            labelKey: "widgetSettings.timer.sound.options.chime"
          },
          {
            value: "marimba",
            labelKey: "widgetSettings.timer.sound.options.marimba"
          },
          {
            value: "crystal",
            labelKey: "widgetSettings.timer.sound.options.crystal"
          }
        ]
      },
      {
        key: "volume",
        type: "slider",
        labelKey: "widgetSettings.timer.volume.label",
        min: 0,
        max: 100,
        step: 5,
        unit: "%"
      },
      {
        key: "notify",
        type: "switch",
        labelKey: "widgetSettings.timer.notify.label"
      }
    ],
  },
  discord: {
    kind: "discord",
    nameKey: "widgets.discord.name",
    descriptionKey: "widgets.discord.desc",
    allowedSizes: ["pill", "s", "m"],
    defaultSize: "m",
    defaultSettings: {},
    render: ({ size, theme, preview, onAriaChange }) => (
      <Discord size={size} theme={theme} preview={preview} onAriaChange={onAriaChange} />
    ),
    settingsSchema: [],
  },
  agents: {
    kind: "agents",
    nameKey: "widgets.agents.name",
    descriptionKey: "widgets.agents.desc",
    allowedSizes: ["s", "m", "l"],
    defaultSize: "m",
    defaultSettings: { recentWindowMin: String(defaultAgentsSettings.recentWindowMin) },
    render: ({ size, theme, settings, preview, onAriaChange }) => (
      <Agents
        size={size}
        theme={theme}
        settings={resolveAgentsSettings(settings)}
        preview={preview}
        onAriaChange={onAriaChange}
      />
    ),
    settingsSchema: [
      {
        key: "recentWindowMin",
        type: "segmented",
        labelKey: "widgetSettings.agents.recentWindowMin.label",
        options: [
          {
            value: "0",
            labelKey: "widgetSettings.agents.recentWindowMin.options.active"
          },
          {
            value: "15",
            labelKey: "widgetSettings.agents.recentWindowMin.options.minutes15"
          },
          {
            value: "60",
            labelKey: "widgetSettings.agents.recentWindowMin.options.hour1"
          }
        ]
      }
    ],
  },
};

export const WIDGET_LIST: readonly WidgetMeta[] = Object.values(WIDGET_REGISTRY);

export interface ClusterPresetMeta {
  presetId: "allInOne" | "miniTrio" | "pillBar";
  nameKey: string;
  descriptionKey: string;
  boxCount: number;
  width: number;
}

export const CLUSTER_PRESETS: readonly ClusterPresetMeta[] = [
  {
    presetId: "allInOne",
    nameKey: "cluster.presets.allInOne",
    descriptionKey: "gallery.presetDesc.allInOne",
    boxCount: 1,
    width: 264,
  },
  {
    presetId: "miniTrio",
    nameKey: "cluster.presets.miniTrio",
    descriptionKey: "gallery.presetDesc.miniTrio",
    boxCount: 3,
    width: 232,
  },
  {
    presetId: "pillBar",
    nameKey: "cluster.presets.pillBar",
    descriptionKey: "gallery.presetDesc.pillBar",
    boxCount: 1,
    width: 264,
  },
];

const SIZE_ORDER: readonly WidgetSize[] = ["pill", "s", "m", "l"];

/** Clamp til nærmeste tilladte størrelse — defensivt net mod legacy-configs,
    da add/cycle-flows allerede kun uddeler tilladte størrelser. */
export function clampSizeToAllowed(
  size: WidgetSize,
  allowed: readonly WidgetSize[],
): WidgetSize {
  if (allowed.includes(size)) return size;
  const index = SIZE_ORDER.indexOf(size);
  let best = allowed[0] ?? "m";
  let bestDist = Infinity;
  for (const candidate of allowed) {
    const dist = Math.abs(SIZE_ORDER.indexOf(candidate) - index);
    if (dist < bestDist) {
      bestDist = dist;
      best = candidate;
    }
  }
  return best;
}

/** Én sandhed for kind→komponent: bruges af både widget-vinduet og
    kontrolcentrets preview, så de to aldrig kan drifte fra hinanden. */
export function renderWidget(kind: FreeWidgetKind, ctx: WidgetRenderContext): ReactNode {
  const meta = WIDGET_REGISTRY[kind];
  return meta.render({ ...ctx, size: clampSizeToAllowed(ctx.size, meta.allowedSizes) });
}
