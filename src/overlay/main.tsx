import { useEffect, useState } from "react";
import ReactDOM from "react-dom/client";
import type { Guide } from "../shared/bindings/Guide";
import type { ThemeId } from "../shared/bindings/ThemeId";
import { t, setLocale, type Locale } from "../shared/i18n";
import { ipc, on } from "../shared/ipc";
import { installErrorReporting } from "../shared/runtime";
import "../shared/theme.css";
import "./overlay.css";

installErrorReporting("overlay");

const params = new URLSearchParams(window.location.search);
const mode = params.get("mode") ?? "grid";
const monitorName = params.get("monitor") ?? "";

function GridOverlay({ monitor }: { monitor: string }) {
  const [theme, setTheme] = useState<ThemeId>("aurora");
  const [lines, setLines] = useState<Guide[]>([]);

  useEffect(() => {
    let active = true;
    const unlisteners: Array<() => void> = [];

    ipc
      .configGet()
      .then((cfg) => {
        if (active) setTheme(cfg.appearance.themeId);
      })
      .catch(() => {});

    on.config((cfg) => {
      if (active) setTheme(cfg.appearance.themeId);
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    });

    on.editGuides((payload) => {
      if (!active) return;
      if (payload.monitor === monitor) {
        setLines(payload.lines);
      } else {
        setLines([]);
      }
    }).then((dispose) => {
      if (active) unlisteners.push(dispose);
      else dispose();
    });

    return () => {
      active = false;
      for (const dispose of unlisteners) {
        dispose();
      }
    };
  }, [monitor]);

  return (
    <div className={`overlay-root theme-${theme}`} aria-hidden="true">
      <div className="grid-dots" />
      {lines.map((line, index) => {
        const span = Math.max(1, line.to - line.from);
        const cls = `guide ${line.axis === "x" ? "v" : "h"}${line.dashed ? " dash" : ""}`;
        const style =
          line.axis === "x"
            ? {
                left: `${line.pos}px`,
                top: `${line.from}px`,
                height: `${span}px`,
              }
            : {
                top: `${line.pos}px`,
                left: `${line.from}px`,
                width: `${span}px`,
              };
        return <div key={`${line.axis}-${index}-${line.pos}`} className={cls} style={style} />;
      })}
    </div>
  );
}

function OverlayBar() {
  const [, setLocaleState] = useState<Locale>("da");

  useEffect(() => {
    let active = true;
    let dispose: (() => void) | undefined;

    ipc
      .configGet()
      .then((cfg) => {
        if (!active) return;
        setLocale(cfg.general.language);
        setLocaleState(cfg.general.language);
      })
      .catch(() => {});

    on.config((cfg) => {
      if (!active) return;
      setLocale(cfg.general.language);
      setLocaleState(cfg.general.language);
    })
      .then((fn) => {
        if (active) dispose = fn;
        else fn();
      })
      .catch(() => {});

    return () => {
      active = false;
      dispose?.();
    };
  }, []);

  return (
    <div
      className="overlay-root"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          void ipc.editSet(false);
        }
      }}
    >
      <div className="editbar" role="toolbar" aria-label={t("edit.barLead")}>
        <span>
          <b>{t("edit.barLead")}</b>
          {t("edit.barRest")}
        </span>
        <button type="button" onClick={() => void ipc.editSet(false)}>
          {t("edit.done")}
        </button>
      </div>
    </div>
  );
}

const root = document.getElementById("root");
if (root) {
  ReactDOM.createRoot(root).render(
    mode === "bar" ? <OverlayBar /> : <GridOverlay monitor={monitorName} />,
  );
}
