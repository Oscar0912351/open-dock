import { useEffect, useState } from "react";
import type { ControlTheme } from "../shared/bindings/ControlTheme";
import { DockLogo } from "../shared/icons/DockLogo";
import { WeatherSprite } from "../shared/icons/WeatherSprite";
import { windowControls } from "../shared/ipc";
import { reportFailure } from "../shared/logging";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { ToastStack } from "./components/Toast";
import { Desktop } from "./routes/Desktop";
import { Gallery } from "./routes/Gallery";
import { Onboarding } from "./routes/Onboarding";
import { Settings } from "./routes/Settings";
import { Wallpapers } from "./routes/Wallpapers";
import { initControlStore, useControlStore } from "./store";

export function App() {
  const config = useControlStore((s) => s.config);
  const route = useControlStore((s) => s.route);
  const setRoute = useControlStore((s) => s.setRoute);
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
        .catch((reason: unknown) => reportFailure("window_is_maximized", reason));
    };
    sync();

    let dispose: (() => void) | undefined;
    windowControls
      .onResized(() => sync())
      .then((fn) => {
        if (active) dispose = fn;
        else fn();
      })
      .catch((reason: unknown) => reportFailure("window_on_resized", reason));

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
          setRoute("settings");
          return;
        }
        if (event.key.toLowerCase() === "n") {
          event.preventDefault();
          setRoute("gallery");
          return;
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [showOnboarding, setRoute]);

  const shellClass = [
    "cc-window",
    !isWindows11 ? "is-win10" : "",
    maximized ? "is-maximized" : "",
    showOnboarding ? "is-onboarding" : "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <section className={shellClass} data-ui-theme={uiTheme}>
      <WeatherSprite />
      <TitleBar
        maximized={maximized}
        showOnboarding={showOnboarding}
      />
      <div className="cc-body">
        <Sidebar />
        <main className="cc-main">
          {!config ? (
            <div className="cc-boot-placeholder" aria-hidden="true">
              <DockLogo size={36} />
            </div>
          ) : (
            <>
              {route === "gallery" && <Gallery key="gallery" />}
              {route === "desktop" && <Desktop key="desktop" />}
              {route === "wallpapers" && <Wallpapers key="wallpapers" />}
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
