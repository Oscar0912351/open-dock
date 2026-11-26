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
      label: string;
      options: Array<{ value: string; label: string }>;
      help?: string;
    }
  | {
      key: string;
      type: "switch";
      label: string;
      help?: string;
    }
  | {
      key: string;
      type: "slider";
      label: string;
      min: number;
      max: number;
      step: number;
      unit?: string;
    }
  | {
      key: string;
      type: "text";
      label: string;
      placeholder?: string;
      maxLength?: number;
    }
  | {
      key: string;
      type: "timezone" | "extraZones" | "metrics" | "weatherLocation";
      label: string;
      help?: string;
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
  name: string;
  description: string;
  allowedSizes: readonly WidgetSize[];
  defaultSize: WidgetSize;
  defaultSettings: Record<string, unknown>;
  settingsSchema: readonly WidgetSettingField[];
  render: (ctx: WidgetRenderContext) => ReactNode;
}

export const WIDGET_REGISTRY: Record<FreeWidgetKind, WidgetMeta> = {
  clock: {
    kind: "clock",
    name: "Ur",
    description: "Tid, dato og op til tre ekstra tidszoner.",
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
        label: "Stil",
        options: [
          { value: "digital", label: "Digital" },
          { value: "analog", label: "Analog" },
        ],
      },
      { key: "showSeconds", type: "switch", label: "Vis sekunder" },
      { key: "showDate", type: "switch", label: "Vis dato" },
      { key: "timezone", type: "timezone", label: "Tidszone" },
      { key: "label", type: "text", label: "Etiket", placeholder: "Lokal tid", maxLength: 32 },
      { key: "extraZones", type: "extraZones", label: "Ekstra byer" },
    ],
  },
  nowPlaying: {
    kind: "nowPlaying",
    name: "Afspiller",
    description: "Styr Spotify, browser og medieafspillere via Windows.",
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
        label: "Kilde",
        options: [
          { value: "auto", label: "Automatisk" },
          { value: "Spotify", label: "Spotify" },
        ],
      },
      { key: "ambient", type: "switch", label: "Farv glasset efter cover" },
      { key: "showProgress", type: "switch", label: "Vis tidslinje" },
      { key: "hideWhenIdle", type: "switch", label: "Skjul når intet spiller" },
    ],
  },
  system: {
    kind: "system",
    name: "System",
    description: "CPU, RAM, GPU, VRAM, netværk og disk i realtid.",
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
      { key: "metrics", type: "metrics", label: "Målinger" },
      {
        key: "primary",
        type: "select",
        label: "Hovedmåling",
        options: [
          { value: "cpu", label: "CPU" },
          { value: "mem", label: "RAM" },
          { value: "gpu", label: "GPU" },
          { value: "vram", label: "VRAM" },
          { value: "net", label: "Netværk" },
          { value: "disk", label: "Disk" },
        ],
      },
      { key: "showTemps", type: "switch", label: "Vis temperatur" },
      { key: "unitsBits", type: "switch", label: "Netværk i bit/s" },
    ],
  },
  weather: {
    kind: "weather",
    name: "Vejr",
    description: "Vejrudsigt med bølgekurve time for time eller 7 dage.",
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
      { key: "locationId", type: "weatherLocation", label: "Sted" },
      {
        key: "unit",
        type: "segmented",
        label: "Enhed",
        options: [
          { value: "c", label: "°C" },
          { value: "f", label: "°F" },
        ],
      },
      {
        key: "chart",
        type: "segmented",
        label: "Graf",
        options: [
          { value: "hourly", label: "Time for time" },
          { value: "daily", label: "7 dage" },
        ],
      },
    ],
  },
  note: {
    kind: "note",
    name: "Note",
    description: "Hurtig tekst eller tjekliste gemt lokalt på din computer.",
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
      { key: "title", type: "text", label: "Titel", placeholder: "Note", maxLength: 32 },
      {
        key: "mode",
        type: "segmented",
        label: "Tilstand",
        options: [
          { value: "text", label: "Tekst" },
          { value: "checklist", label: "Tjekliste" },
        ],
      },
      {
        key: "fontSize",
        type: "segmented",
        label: "Skrift",
        options: [
          { value: "s", label: "S" },
          { value: "m", label: "M" },
          { value: "l", label: "L" },
        ],
      },
    ],
  },
  timer: {
    kind: "timer",
    name: "Timer",
    description: "Skriv fx 'pasta 10' og få besked når tiden er gået. Flere timere ad gangen.",
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
        label: "Lyd",
        options: [
          { value: "chime", label: "Klokke" },
          { value: "marimba", label: "Marimba" },
          { value: "crystal", label: "Krystal" },
        ],
      },
      { key: "volume", type: "slider", label: "Lydstyrke", min: 0, max: 100, step: 5, unit: "%" },
      { key: "notify", type: "switch", label: "Notifikation" },
    ],
  },
  discord: {
    kind: "discord",
    name: "Discord",
    description: "Vis aktivt Discord-call med deltagere, tale-indikator og mute-styring.",
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
    name: "AI Agenter",
    description:
      "Se aktive lokale AI-agenter – Antigravity, Devin, Claude Code, Codex, Cursor m.fl.",
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
        label: "Vis afsluttede",
        options: [
          { value: "0", label: "Kun aktive" },
          { value: "15", label: "15 min" },
          { value: "60", label: "1 time" },
        ],
      },
    ],
  },
};

export const WIDGET_LIST: readonly WidgetMeta[] = Object.values(WIDGET_REGISTRY);

export interface ClusterPresetMeta {
  presetId: "allInOne" | "miniTrio" | "pillBar";
  name: string;
  description: string;
  boxCount: number;
  width: number;
}

export const CLUSTER_PRESETS: readonly ClusterPresetMeta[] = [
  {
    presetId: "allInOne",
    name: "Alt-i-én Boks",
    description: "Ur, Vejr, System, Afspiller og Discord samlet i én adaptiv glasboks.",
    boxCount: 1,
    width: 264,
  },
  {
    presetId: "miniTrio",
    name: "Mini Trio",
    description: "Tre ultrakompakte bokse: Tid & Vejr, Aktivitet og Ressourcer.",
    boxCount: 3,
    width: 232,
  },
  {
    presetId: "pillBar",
    name: "Kapsel-Bar",
    description: "48u høj glas-pille der viser Ur, Vejr og CPU – og folder sig ud ved behov.",
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
