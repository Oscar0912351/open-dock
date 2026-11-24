import { ArrowDown, ArrowUp, CheckSquare, DotsSixVertical, Plus, Square, Trash, X } from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { GeoResult } from "../../shared/bindings/GeoResult";
import type { SystemCaps } from "../../shared/bindings/SystemCaps";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { t } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { WIDGET_REGISTRY, type FreeWidgetKind, type WidgetSettingField } from "../../widget/registry";
import { useControlStore } from "../store";
import { Drawer } from "./Drawer";
import { Input } from "./Input";
import { Segmented } from "./Segmented";
import { Select, type SelectOption } from "./Select";
import { Slider } from "./Slider";
import { Switch } from "./Switch";

interface SettingsFormProps {
  kind: FreeWidgetKind;
  size: WidgetSize;
  settings: Record<string, unknown>;
  onPatch: (patch: Record<string, unknown>) => void;
}

const ALL_METRICS: ReadonlyArray<{ id: string; label: string }> = [
  { id: "cpu", label: "CPU" },
  { id: "mem", label: "RAM" },
  { id: "gpu", label: "GPU" },
  { id: "vram", label: "VRAM" },
  { id: "net", label: "Netværk" },
  { id: "disk", label: "Disk" },
];

const METRICS_MAX_BY_SIZE: Record<WidgetSize, number> = {
  pill: 1,
  s: 2,
  m: 4,
  l: 5,
};

const REGION_DA: Record<string, string> = {
  Europe: "Europa",
  America: "Amerika",
  Asia: "Asien",
  Africa: "Afrika",
  Australia: "Australien",
  Pacific: "Stillehavet",
  Atlantic: "Atlanterhavet",
  Indian: "Indiske Ocean",
  Arctic: "Arktis",
  Antarctica: "Antarktis",
};

function formatTimezoneOffset(zone: string, now: Date): string {
  try {
    const parts = new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      timeZoneName: "shortOffset",
    }).formatToParts(now);
    const raw = parts.find((p) => p.type === "timeZoneName")?.value ?? "UTC";
    return raw.replace("GMT", "UTC");
  } catch {
    return "UTC";
  }
}

function formatTimezoneLabel(zone: string, now: Date): string {
  const parts = zone.split("/");
  const region = REGION_DA[parts[0] ?? ""] ?? parts[0];
  const rest = parts.slice(1).join("/").replaceAll("_", " ");
  const cityText = rest ? `${region}/${rest}` : zone;
  const off = formatTimezoneOffset(zone, now);
  return `${cityText} (${off})`;
}

let cachedTzOptions: SelectOption<string>[] | null = null;
function getTimezoneOptions(): SelectOption<string>[] {
  if (cachedTzOptions) return cachedTzOptions;
  const now = new Date();
  let zones: string[] = [];
  try {
    const fn = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] })
      .supportedValuesOf;
    if (typeof fn === "function") {
      zones = fn("timeZone");
    }
  } catch {
    zones = [];
  }
  if (zones.length === 0) {
    zones = [
      "Europe/Copenhagen",
      "Europe/London",
      "Europe/Berlin",
      "America/New_York",
      "America/Los_Angeles",
      "Asia/Tokyo",
      "Australia/Sydney",
    ];
  }
  cachedTzOptions = zones.map((zone) => ({
    value: zone,
    label: formatTimezoneLabel(zone, now),
  }));
  return cachedTzOptions;
}

export function SettingsForm({ kind, size, settings, onPatch }: SettingsFormProps) {
  const meta = WIDGET_REGISTRY[kind];
  const locations = useControlStore((s) => s.config?.weather.locations ?? []);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [systemCaps, setSystemCaps] = useState<SystemCaps | null>(null);

  useEffect(() => {
    if (kind !== "system") return;
    let active = true;
    ipc
      .systemCaps()
      .then((caps) => {
        if (active) setSystemCaps(caps);
      })
      .catch(() => {});
    return () => {
      active = false;
    };
  }, [kind]);

  return (
    <>
      {meta.settingsSchema.map((field) => (
        <SettingFieldControl
          key={field.key}
          field={field}
          kind={kind}
          size={size}
          settings={settings}
          locations={locations}
          systemCaps={systemCaps}
          onPatch={onPatch}
          onOpenPlaceDrawer={() => setDrawerOpen(true)}
        />
      ))}

      {kind === "weather" && (
        <WeatherSearchDrawer
          open={drawerOpen}
          activeLocationId={
            typeof settings.locationId === "string"
              ? settings.locationId
              : (locations[0]?.id ?? null)
          }
          onSelectLocation={(locationId) => {
            onPatch({ locationId });
            setDrawerOpen(false);
          }}
          onClose={() => setDrawerOpen(false)}
        />
      )}
    </>
  );
}

function SettingFieldControl({
  field,
  kind,
  size,
  settings,
  locations,
  systemCaps,
  onPatch,
  onOpenPlaceDrawer,
}: {
  field: WidgetSettingField;
  kind: FreeWidgetKind;
  size: WidgetSize;
  settings: Record<string, unknown>;
  locations: ReadonlyArray<{ id: string; name: string; admin: string | null; country: string }>;
  systemCaps: SystemCaps | null;
  onPatch: (patch: Record<string, unknown>) => void;
  onOpenPlaceDrawer: () => void;
}) {
  const tzOptions = useMemo(() => getTimezoneOptions(), []);
  const val = settings[field.key];

  switch (field.type) {
    case "switch": {
      const checked = Boolean(val);
      const tempUnsupported =
        kind === "system" &&
        field.key === "showTemps" &&
        systemCaps !== null &&
        !systemCaps.cpuTemp &&
        !systemCaps.gpuTemp;
      return (
        <div className="cc-insp-field">
          <div className="cc-insp-field-head">
            <span className="cc-field-label">{field.label}</span>
            <Switch
              checked={checked}
              disabled={tempUnsupported}
              ariaLabel={field.label}
              onChange={(next) => onPatch({ [field.key]: next })}
            />
          </div>
          {tempUnsupported ? (
            <span className="cc-field-help">{t("form.noTemperatureSensor")}</span>
          ) : (
            field.help && <span className="cc-field-help">{field.help}</span>
          )}
        </div>
      );
    }

    case "segmented": {
      const current = typeof val === "string" ? val : (field.options[0]?.value ?? "");
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{field.label}</span>
          <Segmented
            fullWidth
            options={field.options}
            value={current}
            ariaLabel={field.label}
            onChange={(next) => onPatch({ [field.key]: next })}
          />
          {field.help && <span className="cc-field-help">{field.help}</span>}
        </div>
      );
    }

    case "select": {
      const current = typeof val === "string" ? val : (field.options[0]?.value ?? "");
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{field.label}</span>
          <Select
            options={field.options}
            value={current}
            ariaLabel={field.label}
            onChange={(next) => onPatch({ [field.key]: next })}
          />
          {field.help && <span className="cc-field-help">{field.help}</span>}
        </div>
      );
    }

    case "slider": {
      const num = typeof val === "number" && Number.isFinite(val) ? val : field.min;
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{field.label}</span>
          <Slider
            value={num}
            min={field.min}
            max={field.max}
            step={field.step}
            ariaLabel={field.label}
            formatValue={(v) => (field.unit ? `${v} ${field.unit}` : String(v))}
            onChange={(next) => onPatch({ [field.key]: next })}
          />
        </div>
      );
    }

    case "text": {
      return (
        <DebouncedTextField
          label={field.label}
          value={typeof val === "string" ? val : ""}
          placeholder={field.placeholder}
          maxLength={field.maxLength}
          onChange={(next) => onPatch({ [field.key]: next })}
        />
      );
    }

    case "timezone": {
      const current = typeof val === "string" ? val : "";
      const options: SelectOption<string>[] = [
        { value: "", label: t("clock.local") },
        ...tzOptions,
      ];
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{field.label}</span>
          <Select
            searchable
            searchPlaceholder={t("form.searchTimezone")}
            options={options}
            value={current}
            ariaLabel={field.label}
            onChange={(next) => onPatch({ [field.key]: next || null })}
          />
        </div>
      );
    }

    case "extraZones": {
      const zones = Array.isArray(val)
        ? val.filter((z): z is string => typeof z === "string")
        : [];
      const available = tzOptions.filter((opt) => !zones.includes(opt.value));
      return (
        <div className="cc-insp-field">
          <div className="cc-insp-field-head">
            <span className="cc-field-label">{field.label}</span>
            <span className="cc-field-help">{zones.length} / 3</span>
          </div>
          {zones.length > 0 && (
            <div className="cc-sort-list">
              {zones.map((z, idx) => (
                <div key={z} className="cc-sort-chip">
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {formatTimezoneLabel(z, new Date())}
                  </span>
                  <button
                    type="button"
                    className="cc-iconbtn is-danger"
                    aria-label={t("window.close")}
                    onClick={() =>
                      onPatch({ [field.key]: zones.filter((_, i) => i !== idx) })
                    }
                  >
                    <X size={13} aria-hidden="true" />
                  </button>
                </div>
              ))}
            </div>
          )}
          {zones.length < 3 && (
            <Select
              searchable
              searchPlaceholder={t("form.searchTimezone")}
              placeholder={t("form.addZone")}
              options={available}
              value=""
              ariaLabel={t("form.addZone")}
              onChange={(next) => {
                if (next && !zones.includes(next)) {
                  onPatch({ [field.key]: [...zones, next].slice(0, 3) });
                }
              }}
            />
          )}
        </div>
      );
    }

    case "weatherLocation": {
      const activeId =
        typeof val === "string" && val ? val : (locations[0]?.id ?? "");
      const options: SelectOption<string>[] =
        locations.length > 0
          ? locations.map((loc) => ({
              value: loc.id,
              label: loc.name,
              help: [loc.admin, loc.country].filter(Boolean).join(", "),
            }))
          : [{ value: "", label: t("form.noLocations") }];

      return (
        <div className="cc-insp-field">
          <div className="cc-insp-field-head">
            <span className="cc-field-label">{field.label}</span>
            <button type="button" className="cc-link-btn" onClick={onOpenPlaceDrawer}>
              {t("weather.addPlace")}
            </button>
          </div>
          <Select
            options={options}
            value={activeId}
            disabled={locations.length === 0}
            ariaLabel={field.label}
            onChange={(next) => onPatch({ [field.key]: next || null })}
          />
        </div>
      );
    }

    case "metrics": {
      const selected = Array.isArray(val)
        ? val.filter((m): m is string => typeof m === "string")
        : [];
      const maxCount = METRICS_MAX_BY_SIZE[size] ?? 4;
      return (
        <MetricsMultiSort
          label={field.label}
          selected={selected}
          maxCount={maxCount}
          systemCaps={systemCaps}
          onChange={(next) => onPatch({ [field.key]: next })}
        />
      );
    }
  }
}

function DebouncedTextField({
  label,
  value,
  placeholder,
  maxLength,
  onChange,
}: {
  label: string;
  value: string;
  placeholder?: string;
  maxLength?: number;
  onChange: (next: string) => void;
}) {
  const [draftState, setDraftState] = useState<{ base: string; text: string } | null>(null);
  const draft = draftState && draftState.base === value ? draftState.text : value;
  const timerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    };
  }, []);

  const handleChange = (next: string) => {
    setDraftState({ base: value, text: next });
    if (timerRef.current !== null) window.clearTimeout(timerRef.current);
    timerRef.current = window.setTimeout(() => {
      timerRef.current = null;
      onChange(next);
    }, 180);
  };

  return (
    <div className="cc-insp-field">
      <span className="cc-field-label">{label}</span>
      <Input
        type="text"
        value={draft}
        placeholder={placeholder}
        maxLength={maxLength}
        onChange={(e) => handleChange(e.target.value)}
        onBlur={() => {
          if (timerRef.current !== null) {
            window.clearTimeout(timerRef.current);
            timerRef.current = null;
            onChange(draft);
          }
        }}
      />
    </div>
  );
}

function MetricsMultiSort({
  label,
  selected,
  maxCount,
  systemCaps,
  onChange,
}: {
  label: string;
  selected: string[];
  maxCount: number;
  systemCaps: SystemCaps | null;
  onChange: (next: string[]) => void;
}) {
  const [dragIdx, setDragIdx] = useState<number | null>(null);

  const orderedIds = useMemo(() => {
    const valid = selected.filter((id) => ALL_METRICS.some((m) => m.id === id));
    const rest = ALL_METRICS.map((m) => m.id).filter((id) => !valid.includes(id));
    return [...valid, ...rest];
  }, [selected]);

  const activeSet = new Set(selected);

  const toggle = (id: string) => {
    if (activeSet.has(id)) {
      if (selected.length <= 1) return;
      onChange(selected.filter((item) => item !== id));
    } else if (selected.length < maxCount) {
      onChange([...selected, id]);
    }
  };

  const move = (fromIdx: number, toIdx: number) => {
    if (fromIdx === toIdx || toIdx < 0 || toIdx >= orderedIds.length) return;
    const copy = [...orderedIds];
    const [item] = copy.splice(fromIdx, 1);
    if (!item) return;
    copy.splice(toIdx, 0, item);
    onChange(copy.filter((id) => activeSet.has(id)).slice(0, maxCount));
  };

  return (
    <div className="cc-insp-field">
      <div className="cc-insp-field-head">
        <span className="cc-field-label">{label}</span>
        <span className="cc-field-help">
          {t("form.selectedCount", { n: Math.min(selected.length, maxCount), max: maxCount })}
        </span>
      </div>
      <div className="cc-sort-list">
        {orderedIds.map((id, idx) => {
          const checked = activeSet.has(id);
          const metricLabel = ALL_METRICS.find((m) => m.id === id)?.label ?? id;
          const unsupported =
            systemCaps !== null &&
            ((id === "gpu" && !systemCaps.gpu) ||
              (id === "vram" && !systemCaps.vram) ||
              (id === "disk" && !systemCaps.disk));
          const canCheck = checked ? selected.length > 1 : selected.length < maxCount;
          const canToggle = unsupported ? checked : canCheck && !unsupported;
          return (
            <div
              key={id}
              draggable={checked}
              onDragStart={() => setDragIdx(idx)}
              onDragOver={(e) => {
                if (dragIdx !== null) e.preventDefault();
              }}
              onDrop={() => {
                if (dragIdx !== null) move(dragIdx, idx);
                setDragIdx(null);
              }}
              className={`cc-sort-chip${checked ? " is-checked" : ""}`}
            >
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span className="cc-drag-grip" aria-hidden="true">
                  <DotsSixVertical size={14} />
                </span>
                <button
                  type="button"
                  disabled={!canToggle}
                  className="cc-iconbtn"
                  style={{ width: 24, height: 24 }}
                  aria-label={metricLabel}
                  title={unsupported ? t("form.metricUnsupported") : undefined}
                  onClick={() => toggle(id)}
                >
                  {checked ? (
                    <CheckSquare size={16} weight="fill" color="var(--c-accent)" />
                  ) : (
                    <Square size={16} />
                  )}
                </button>
                <span>{metricLabel}</span>
              </div>

              {checked && (
                <div style={{ display: "flex", gap: 2 }}>
                  <button
                    type="button"
                    className="cc-iconbtn"
                    style={{ width: 24, height: 24 }}
                    disabled={idx === 0}
                    aria-label={t("desktop.moveUp")}
                    onClick={() => move(idx, idx - 1)}
                  >
                    <ArrowUp size={12} />
                  </button>
                  <button
                    type="button"
                    className="cc-iconbtn"
                    style={{ width: 24, height: 24 }}
                    disabled={idx >= selected.length - 1}
                    aria-label={t("desktop.moveDown")}
                    onClick={() => move(idx, idx + 1)}
                  >
                    <ArrowDown size={12} />
                  </button>
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function WeatherSearchDrawer({
  open,
  activeLocationId,
  onSelectLocation,
  onClose,
}: {
  open: boolean;
  activeLocationId: string | null;
  onSelectLocation: (locationId: string) => void;
  onClose: () => void;
}) {
  const locations = useControlStore((s) => s.config?.weather.locations ?? []);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<GeoResult[]>([]);
  const [searching, setSearching] = useState(false);

  const trimmed = query.trim();

  const handleClose = () => {
    setQuery("");
    setResults([]);
    setSearching(false);
    onClose();
  };

  const handleQueryChange = (next: string) => {
    setQuery(next);
    if (next.trim().length < 2) {
      setResults([]);
      setSearching(false);
    } else {
      setSearching(true);
    }
  };

  useEffect(() => {
    if (!open || trimmed.length < 2) return;
    let active = true;
    const timer = window.setTimeout(() => {
      void ipc
        .weatherSearch(trimmed)
        .then((rows) => {
          if (active) setResults(rows);
        })
        .catch(() => {
          if (active) setResults([]);
        })
        .finally(() => {
          if (active) setSearching(false);
        });
    }, 300);

    return () => {
      active = false;
      window.clearTimeout(timer);
    };
  }, [open, trimmed]);

  const handleAddGeo = async (geo: GeoResult) => {
    try {
      const updatedCfg = await ipc.weatherLocationAdd(geo);
      const matched = updatedCfg.weather.locations.find(
        (loc) =>
          loc.name.toLowerCase() === geo.name.toLowerCase() &&
          Math.abs(loc.lat - geo.latitude) < 0.02 &&
          Math.abs(loc.lon - geo.longitude) < 0.02,
      );
      if (matched) {
        onSelectLocation(matched.id);
      } else {
        handleClose();
      }
    } catch {
      handleClose();
    }
  };

  return (
    <Drawer open={open} title={t("weather.addPlace")} onClose={handleClose}>
      <Input
        autoFocus
        type="text"
        value={query}
        placeholder={t("weather.searchPh")}
        onChange={(e) => handleQueryChange(e.target.value)}
      />

      {query.trim().length < 2 ? (
        <span className="cc-field-help">{t("form.searchPlacesHint")}</span>
      ) : searching ? (
        <span className="cc-field-help">{t("form.searching")}</span>
      ) : results.length === 0 ? (
        <span className="cc-field-help">{t("form.noSearchResults")}</span>
      ) : (
        <div className="cc-sort-list">
          {results.map((geo) => {
            const sub = [geo.admin1, geo.country].filter(Boolean).join(", ");
            return (
              <div key={`${geo.name}-${geo.latitude}-${geo.longitude}`} className="cc-place-row">
                <button
                  type="button"
                  className="cc-place-btn"
                  onClick={() => void handleAddGeo(geo)}
                >
                  <span className="cc-place-name">
                    {geo.name}
                    {sub ? ` · ${sub}` : ""}
                  </span>
                </button>
                <Plus size={15} color="var(--c-accent)" aria-hidden="true" />
              </div>
            );
          })}
        </div>
      )}

      {locations.length > 0 && (
        <>
          <div className="cc-section-label" style={{ marginTop: 8 }}>
            {t("desktop.secContent")}
          </div>
          <div className="cc-sort-list">
            {locations.map((loc) => (
              <div
                key={loc.id}
                className={`cc-place-row${loc.id === activeLocationId ? " is-active" : ""}`}
              >
                <button
                  type="button"
                  className="cc-place-btn"
                  onClick={() => onSelectLocation(loc.id)}
                >
                  <span className="cc-place-name">{loc.name}</span>
                  <span className="cc-place-sub">
                    {[loc.admin, loc.country].filter(Boolean).join(", ")}
                  </span>
                </button>
                <button
                  type="button"
                  className="cc-iconbtn is-danger"
                  aria-label={t("form.removePlace")}
                  onClick={() => void ipc.weatherLocationRemove(loc.id)}
                >
                  <Trash size={14} aria-hidden="true" />
                </button>
              </div>
            ))}
          </div>
        </>
      )}
    </Drawer>
  );
}
