// Lukker en kørende Open Dock, før `tauri build` overskriver exe'en.
// Dækker release (open-dock.exe), gammelt debug-navn (ossi-dock.exe)
// og udgaven fra før omdøbningen ("Open Dock.exe").

import { execFileSync } from "node:child_process";

const IMAGES = ["open-dock.exe", "ossi-dock.exe", "Open Dock.exe"];

if (process.platform === "win32") {
  for (const image of IMAGES) {
    try {
      const out = execFileSync("taskkill", ["/F", "/IM", image], {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      });
      process.stdout.write(out);
    } catch {
      // Processen kører ikke — det er i orden.
    }
  }
} else {
  try {
    execFileSync("pkill", ["-f", "open-dock|ossi-dock"], { stdio: "inherit" });
  } catch {
    // Ingen match — i orden.
  }
}
