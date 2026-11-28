import { ipc } from "./ipc";
import { fireIpc, reportError, reportFailure } from "./logging";

export function installErrorReporting(context: string): void {
  window.addEventListener("error", (event) => {
    void reportError(`${context}: ${event.message}`);
  });
  window.addEventListener("unhandledrejection", (event) => {
    reportFailure(context, event.reason);
  });
}

export function installDevtoolsShortcut(): void {
  window.addEventListener("keydown", (event) => {
    const isCtrlShiftI =
      event.ctrlKey && event.shiftKey && !event.altKey && event.code === "KeyI";
    const isF12 = event.key === "F12";
    if (!isCtrlShiftI && !isF12) return;
    event.preventDefault();
    fireIpc(ipc.appToggleDevtools(), "app_toggle_devtools");
  });
}
