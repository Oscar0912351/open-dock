import {
  Eye,
  EyeSlash,
  GearSix,
  Image,
  Monitor,
  NotePencil,
  SquaresFour,
} from "@phosphor-icons/react";
import { t } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { useControlStore, type ControlRoute } from "../store";

const NAV_ITEMS: ReadonlyArray<{
  route: ControlRoute;
  labelKey: string;
  Icon: typeof SquaresFour;
}> = [
  { route: "gallery", labelKey: "nav.gallery", Icon: SquaresFour },
  { route: "desktop", labelKey: "nav.desktop", Icon: Monitor },
  { route: "wallpapers", labelKey: "nav.wallpapers", Icon: Image },
  { route: "settings", labelKey: "nav.settings", Icon: GearSix },
];

export function Sidebar() {
  useControlStore((s) => s.locale);
  const route = useControlStore((s) => s.route);
  const setRoute = useControlStore((s) => s.setRoute);
  const config = useControlStore((s) => s.config);
  const visible = useControlStore((s) => s.visible);
  const paused = useControlStore((s) => s.paused);
  const editMode = useControlStore((s) => s.editMode);

  const widgetCount = config?.widgets.length ?? 0;

  let statusText: string;
  let statusWarn = false;
  if (!visible) {
    statusText = t("status.hidden");
    statusWarn = true;
  } else if (paused.paused) {
    statusWarn = true;
    statusText = t("status.pausedUser");
  } else {
    statusText =
      widgetCount === 1
        ? t("status.activeOne")
        : t("status.active", { n: widgetCount });
  }

  return (
    <aside className="cc-sidebar">
      <nav className="cc-nav" aria-label="Navigation">
        {NAV_ITEMS.map(({ route: itemRoute, labelKey, Icon }) => {
          const active = route === itemRoute;
          return (
            <button
              key={itemRoute}
              type="button"
              className={`cc-nav-item${active ? " is-active" : ""}`}
              aria-current={active ? "page" : undefined}
              onClick={() => setRoute(itemRoute)}
            >
              {active && <span key={itemRoute} className="cc-nav-pip" aria-hidden="true" />}
              <Icon size={16} weight={active ? "fill" : "regular"} aria-hidden="true" />
              <span className="cc-nav-label">{t(labelKey)}</span>
              {itemRoute === "desktop" && widgetCount > 0 && (
                <span className="cc-nav-badge">{widgetCount}</span>
              )}
            </button>
          );
        })}
      </nav>

      <div className="cc-sidebar-footer">
        <div className={`cc-status-line${statusWarn ? " is-warn" : ""}`}>
          <span className="cc-status-dot" aria-hidden="true" />
          <span>{statusText}</span>
        </div>
        <div className="cc-status-actions">
          <button
            type="button"
            className={`cc-status-action-btn${!visible ? " is-active" : ""}`}
            title={visible ? t("status.hideAll") : t("status.showAll")}
            aria-label={visible ? t("status.hideAll") : t("status.showAll")}
            onClick={() => void ipc.appSetVisible(!visible)}
          >
            {visible ? (
              <Eye size={14} aria-hidden="true" />
            ) : (
              <EyeSlash size={14} aria-hidden="true" />
            )}
            <span>{visible ? t("status.hideShort") : t("status.showShort")}</span>
          </button>
          <button
            type="button"
            className={`cc-status-action-btn${editMode ? " is-active" : ""}`}
            title={t("status.editLayout")}
            aria-label={t("status.editLayout")}
            onClick={() => void ipc.editSet(!editMode)}
          >
            <NotePencil size={14} aria-hidden="true" />
            <span>{t("status.editShort")}</span>
          </button>
        </div>
      </div>
    </aside>
  );
}
