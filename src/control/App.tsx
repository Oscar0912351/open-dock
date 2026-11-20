import { useEffect, useState } from "react";
import type { ControlTheme } from "../shared/bindings/ControlTheme";
import { DockLogo } from "../shared/icons/DockLogo";
import { WeatherSprite } from "../shared/icons/WeatherSprite";
import { ipc, windowControls } from "../shared/ipc";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { ToastStack } from "./components/Toast";
import { Desktop } from "./routes/Desktop";
import { Gallery } from "./routes/Gallery";
import { Onboarding } from "./routes/Onboarding";
import { Settings } from "./routes/Settings";
import { Themes } from "./routes/Themes";
import { Wallpapers } from "./routes/Wallpapers";
import { initControlStore, useControlStore } from "./store";

export function App() {
  const config = useControlStore((s) => s.config);
  const route = useControlStore((s) => s.route);
  const setRoute = useControlStore((s) => s.setRoute);
  const selectedId = useControlStore((s) => s.selectedWidgetId);
  const isWindows11 = useControlStore((s) => s.isWindows11);
  const locale = useControlStore((s) => s.locale);

  const showOnboarding = Boolean(config && !config.onboarded);
  const uiTheme: ControlTheme = config?.general.controlTheme ?? "dark";
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    return initControlStore();
  }, []);

  useEffect(() => {
    document.documentElement.lang = locale;
  }, [locale]);

  useEffect(() => {
    document.documentElement.dataset.uiTheme = uiTheme;
    document.documentElement.style.colorScheme = uiTheme;
  }, [uiTheme]);

  useEffect(() => {
    let active = true;
    const sync = () => {
      void windowControls
        .isMaximized()
        .then((val) => {
          if (active) setMaximized(val);
        })
        .catch(() => {});
    };
    sync();

    let dispose: (() => void) | undefined;
    windowControls
      .onResized(() => sync())
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

  useEffect(() => {
    if (showOnboarding) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const inInput = Boolean(
        target &&
          (target.tagName === "INPUT" ||
            target.tagName === "TEXTAREA" ||
            target.tagName === "SELECT" ||
            target.isContentEditable),
      );
      if (inInput) return;

      if (event.ctrlKey && !event.altKey && !event.shiftKey) {
        if (event.key === "1") {
          event.preventDefault();
          setRoute("gallery");
          return;
        }
        if (event.key === "2") {
          event.preventDefault();
          setRoute("desktop");
          return;
        }
        if (event.key === "3") {
          event.preventDefault();
          setRoute("wallpapers");
          return;
        }
        if (event.key === "4") {
          event.preventDefault();
          setRoute("themes");
          return;
        }
        if (event.key === "5") {
          event.preventDefault();
          setRoute("settings");
          return;
        }
        if (event.key.toLowerCase() === "n") {
          event.preventDefault();
          setRoute("gallery");
          return;
        }
      }

      if (event.key === "Delete" && route === "desktop" && selectedId) {
        if (document.querySelector(".cc-backdrop")) return;
        event.preventDefault();
        void ipc.widgetRemove(selectedId);
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [showOnboarding, route, selectedId, setRoute]);

  const shellClass = [
    "cc-window",
    !isWindows11 ? "is-win10" : "",
    maximized ? "is-maximized" : "",
    showOnboarding ? "is-onboarding" : "",
  ]
    .filter(Boolean)
    .join(" ");

  const mainClass = [
    "cc-main",
    config && route === "desktop" ? "is-flush" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <section className={shellClass} data-ui-theme={uiTheme}>
      <WeatherSprite />
      <TitleBar
        maximized={maximized}
        showOnboarding={showOnboarding}
        uiTheme={uiTheme}
      />
      <div className="cc-body">
        <Sidebar />
        <main className={mainClass}>
          {!config ? (
            <div className="cc-boot-placeholder" aria-hidden="true">
              <DockLogo size={36} />
            </div>
          ) : (
            <>
              {route === "gallery" && <Gallery key="gallery" />}
              {route === "desktop" && <Desktop key="desktop" />}
              {route === "wallpapers" && <Wallpapers key="wallpapers" />}
              {route === "themes" && <Themes key="themes" />}
              {route === "settings" && <Settings key="settings" />}
            </>
          )}
        </main>
      </div>
      {showOnboarding && <Onboarding />}
      <ToastStack />
    </section>
  );
}
