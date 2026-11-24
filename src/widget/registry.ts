import type { WidgetKind } from "../shared/bindings/WidgetKind";
import type { WidgetSize } from "../shared/bindings/WidgetSize";
import { defaultAgentsSettings } from "./kinds/agents/Agents";
import { defaultClockSettings } from "./kinds/clock/Clock";
import { defaultNoteSettings } from "./kinds/note/Note";
import { defaultTimerSettings } from "./kinds/timer/Timer";
import { defaultNowPlayingSettings } from "./kinds/nowPlaying/NowPlaying";
import { defaultSystemSettings } from "./kinds/system/System";
import { defaultWeatherSettings } from "./kinds/weather/Weather";

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

export interface WidgetMeta {
  kind: FreeWidgetKind;
  name: string;
  description: string;
  allowedSizes: readonly WidgetSize[];
  defaultSize: WidgetSize;
  defaultSettings: Record<string, unknown>;
  settingsSchema: readonly WidgetSettingField[];
}

export const WIDGET_REGISTRY: Record<FreeWidgetKind, WidgetMeta> = {
  clock: {
    kind: "clock",
    name: "Ur",
    description: "Tid, dato og op til tre ekstra tidszoner.",
    allowedSizes: ["pill", "s", "m"],
    defaultSize: "m",
    defaultSettings: { ...defaultClockSettings },
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
    description: "Nedtælling eller pomodoro med en lille note og en blid lyd når tiden er gået.",
    allowedSizes: ["pill", "s", "m"],
    defaultSize: "s",
    defaultSettings: { ...defaultTimerSettings },
    settingsSchema: [
      {
        key: "mode",
        type: "segmented",
        label: "Tilstand",
        options: [
          { value: "countdown", label: "Nedtælling" },
          { value: "pomodoro", label: "Pomodoro" },
        ],
      },
      { key: "durationMin", type: "slider", label: "Varighed", min: 1, max: 480, step: 1, unit: "min" },
      { key: "focusMin", type: "slider", label: "Fokus", min: 5, max: 120, step: 5, unit: "min" },
      { key: "breakMin", type: "slider", label: "Pause", min: 1, max: 30, step: 1, unit: "min" },
      { key: "longBreakMin", type: "slider", label: "Lang pause", min: 5, max: 60, step: 5, unit: "min" },
      { key: "rounds", type: "slider", label: "Runder", min: 2, max: 8, step: 1 },
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
      { key: "autoStartBreak", type: "switch", label: "Start pauser automatisk" },
    ],
  },
  discord: {
    kind: "discord",
    name: "Discord",
    description: "Vis aktivt Discord-call med deltagere, tale-indikator og mute-styring.",
    allowedSizes: ["pill", "s", "m"],
    defaultSize: "m",
    defaultSettings: {},
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
