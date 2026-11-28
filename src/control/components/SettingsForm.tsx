import { ArrowDown, ArrowUp, CheckSquare, DotsSixVertical, Plus, Square, Trash, X } from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { GeoResult } from "../../shared/bindings/GeoResult";
import type { Location } from "../../shared/bindings/Location";
import type { SystemCaps } from "../../shared/bindings/SystemCaps";
import type { WidgetSize } from "../../shared/bindings/WidgetSize";
import { translate, useLocale, useT, type Locale } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { reportFailure } from "../../shared/logging";
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

const EMPTY_LOCATIONS: Location[] = [];

const ALL_METRICS: ReadonlyArray<{ id: string }> = [
  { id: "cpu" },
  { id: "mem" },
  { id: "gpu" },
  { id: "vram" },
  { id: "net" },
  { id: "disk" },
];

const METRICS_MAX_BY_SIZE: Record<WidgetSize, number> = {
  pill: 1,
  s: 2,
  m: 4,
  l: 5,
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

function formatTimezoneLabel(zone: string, now: Date, locale: Locale): string {
  const parts = zone.split("/");
  const regionKey = `timezones.regions.${parts[0] ?? ""}`;
  const translatedRegion = translate(locale, regionKey);
  const region = translatedRegion === regionKey ? parts[0] : translatedRegion;
  const rest = parts.slice(1).join("/").replaceAll("_", " ");
  const cityText = rest ? `${region}/${rest}` : zone;
  const off = formatTimezoneOffset(zone, now);
  return `${cityText} (${off})`;
}

const cachedTzOptions: Partial<Record<Locale, SelectOption<string>[]>> = {};
function getTimezoneOptions(locale: Locale): SelectOption<string>[] {
  const cached = cachedTzOptions[locale];
  if (cached) return cached;
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
  const options = zones.map((zone) => ({
    value: zone,
    label: formatTimezoneLabel(zone, now, locale),
  }));
  cachedTzOptions[locale] = options;
  return options;
}

export function SettingsForm({ kind, size, settings, onPatch }: SettingsFormProps) {
  useLocale();
  const meta = WIDGET_REGISTRY[kind];
  const locations = useControlStore((s) => s.config?.weather.locations ?? EMPTY_LOCATIONS);
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
      .catch((reason: unknown) => reportFailure("system_caps", reason));
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
  const locale = useLocale();
  const tr = useT();
  const tzOptions = useMemo(() => getTimezoneOptions(locale), [locale]);
  const val = settings[field.key];
  const fieldLabel = tr(field.labelKey);
  const fieldHelp = "helpKey" in field && field.helpKey ? tr(field.helpKey) : undefined;
  const fieldPlaceholder = field.type === "text" && field.placeholderKey ? tr(field.placeholderKey) : undefined;
  const fieldOptions =
    field.type === "segmented" || field.type === "select"
      ? field.options.map((opt) => ({ value: opt.value, label: tr(opt.labelKey) }))
      : [];

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
            <span className="cc-field-label">{fieldLabel}</span>
            <Switch
              checked={checked}
              disabled={tempUnsupported}
              ariaLabel={fieldLabel}
              onChange={(next) => onPatch({ [field.key]: next })}
            />
          </div>
          {tempUnsupported ? (
            <span className="cc-field-help">{tr("form.noTemperatureSensor")}</span>
          ) : (
            fieldHelp && <span className="cc-field-help">{fieldHelp}</span>
          )}
        </div>
      );
    }

    case "segmented": {
      const current = typeof val === "string" ? val : (field.options[0]?.value ?? "");
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{fieldLabel}</span>
          <Segmented
            fullWidth
            options={fieldOptions}
            value={current}
            ariaLabel={fieldLabel}
            onChange={(next) => onPatch({ [field.key]: next })}
          />
          {fieldHelp && <span className="cc-field-help">{fieldHelp}</span>}
        </div>
      );
    }

    case "select": {
      const current = typeof val === "string" ? val : (field.options[0]?.value ?? "");
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{fieldLabel}</span>
          <Select
            options={fieldOptions}
            value={current}
            ariaLabel={fieldLabel}
            onChange={(next) => onPatch({ [field.key]: next })}
          />
          {fieldHelp && <span className="cc-field-help">{fieldHelp}</span>}
        </div>
      );
    }

    case "slider": {
      const num = typeof val === "number" && Number.isFinite(val) ? val : field.min;
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{fieldLabel}</span>
          <Slider
            value={num}
            min={field.min}
            max={field.max}
            step={field.step}
            ariaLabel={fieldLabel}
            formatValue={(v) => (field.unit ? `${v} ${field.unit}` : String(v))}
            onChange={(next) => onPatch({ [field.key]: next })}
          />
        </div>
      );
    }

    case "text": {
      return (
        <DebouncedTextField
          label={fieldLabel}
          value={typeof val === "string" ? val : ""}
          placeholder={fieldPlaceholder}
          maxLength={field.maxLength}
          onChange={(next) => onPatch({ [field.key]: next })}
        />
      );
    }

    case "timezone": {
      const current = typeof val === "string" ? val : "";
      const options: SelectOption<string>[] = [
        { value: "", label: tr("clock.local") },
        ...tzOptions,
      ];
      return (
        <div className="cc-insp-field">
          <span className="cc-field-label">{fieldLabel}</span>
          <Select
            searchable
            searchPlaceholder={tr("form.searchTimezone")}
            options={options}
            value={current}
            ariaLabel={fieldLabel}
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
            <span className="cc-field-label">{fieldLabel}</span>
            <span className="cc-field-help">{zones.length} / 3</span>
          </div>
          {zones.length > 0 && (
            <div className="cc-sort-list">
              {zones.map((z, idx) => (
                <div key={z} className="cc-sort-chip">
                  <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {formatTimezoneLabel(z, new Date(), locale)}
                  </span>
                  <button
                    type="button"
                    className="cc-iconbtn is-danger"
                    aria-label={tr("window.close")}
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
              searchPlaceholder={tr("form.searchTimezone")}
              placeholder={tr("form.addZone")}
              options={available}
              value=""
              ariaLabel={tr("form.addZone")}
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
          : [{ value: "", label: tr("form.noLocations") }];

      return (
        <div className="cc-insp-field">
          <div className="cc-insp-field-head">
            <span className="cc-field-label">{fieldLabel}</span>
            <button type="button" className="cc-link-btn" onClick={onOpenPlaceDrawer}>
              {tr("weather.addPlace")}
            </button>
          </div>
          <Select
            options={options}
            value={activeId}
            disabled={locations.length === 0}
            ariaLabel={fieldLabel}
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
          label={fieldLabel}
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
  const tr = useT();
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
          {tr("form.selectedCount", { n: Math.min(selected.length, maxCount), max: maxCount })}
        </span>
      </div>
      <div className="cc-sort-list">
        {orderedIds.map((id, idx) => {
          const checked = activeSet.has(id);
          const metricLabel = tr(`widgetSettings.system.primary.options.${id}`);
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
                  title={unsupported ? tr("form.metricUnsupported") : undefined}
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
                    aria-label={tr("desktop.moveUp")}
                    onClick={() => move(idx, idx - 1)}
                  >
                    <ArrowUp size={12} />
                  </button>
                  <button
                    type="button"
                    className="cc-iconbtn"
                    style={{ width: 24, height: 24 }}
                    disabled={idx >= selected.length - 1}
                    aria-label={tr("desktop.moveDown")}
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
  const tr = useT();
  const locations = useControlStore((s) => s.config?.weather.locations ?? EMPTY_LOCATIONS);
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
    <Drawer open={open} title={tr("weather.addPlace")} onClose={handleClose}>
      <Input
        autoFocus
        type="text"
        value={query}
        placeholder={tr("weather.searchPh")}
        onChange={(e) => handleQueryChange(e.target.value)}
      />

      {query.trim().length < 2 ? (
        <span className="cc-field-help">{tr("form.searchPlacesHint")}</span>
      ) : searching ? (
        <span className="cc-field-help">{tr("form.searching")}</span>
      ) : results.length === 0 ? (
        <span className="cc-field-help">{tr("form.noSearchResults")}</span>
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
            {tr("desktop.secContent")}
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
                  aria-label={tr("form.removePlace")}
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
