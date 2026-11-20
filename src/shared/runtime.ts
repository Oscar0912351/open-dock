import { ipc } from "./ipc";
import { reportError } from "./logging";

export function installErrorReporting(context: string): void {
  window.addEventListener("error", (event) => {
    void reportError(`${context}: ${event.message}`);
  });
  window.addEventListener("unhandledrejection", (event) => {
    void reportError(`${context}: ${String(event.reason)}`);
  });
}

export function installDevtoolsShortcut(): void {
  window.addEventListener("keydown", (event) => {
    const isCtrlShiftI =
      event.ctrlKey && event.shiftKey && !event.altKey && event.code === "KeyI";
    const isF12 = event.key === "F12";
    if (!isCtrlShiftI && !isF12) return;
    event.preventDefault();
    void ipc.appToggleDevtools().catch(() => {});
  });
}
