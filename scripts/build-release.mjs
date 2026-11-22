// Bygger Open Dock med et indlejret build-id (unix-sekunder) og lægger et
// konsistent release-par i release/: Open-Dock-Setup.exe + latest.json.
// latest.json bruges af appens opdateringstjek til at se om et nyere build
// er publiceret — også når versionsnummeret er uændret.

import "./kill-app.mjs";

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const tauriConf = JSON.parse(readFileSync(join(root, "src-tauri/tauri.conf.json"), "utf8"));
const version = tauriConf.version;
const buildId = Math.floor(Date.now() / 1000);

const RELEASE_ASSET_NAME = "Open-Dock-Setup.exe";
const REPO = "https://github.com/Oscar0912351/open-dock";

console.log(`Bygger Open Dock ${version} (build ${buildId})…`);

// Kør tauri.js via node direkte — node blokerer .cmd/.bat-spawn uden shell på Windows.
const tauriCli = join(root, "node_modules", "@tauri-apps", "cli", "tauri.js");
const result = spawnSync(process.execPath, [tauriCli, "build"], {
  cwd: root,
  stdio: "inherit",
  env: { ...process.env, OPEN_DOCK_BUILD_ID: String(buildId) },
});
if (result.error || result.status !== 0) {
  if (result.error) console.error(`Kunne ikke starte tauri build: ${result.error.message}`);
  process.exit(result.status ?? 1);
}

// Find den nyeste *-setup.exe i bundle-mappen — matcher "det nyeste build".
const nsisDir = join(root, "src-tauri", "target", "release", "bundle", "nsis");
if (!existsSync(nsisDir)) {
  console.error(`Ingen NSIS-mappe fundet: ${nsisDir}`);
  process.exit(1);
}
const installers = readdirSync(nsisDir)
  .filter((name) => name.endsWith("-setup.exe"))
  .map((name) => ({ name, mtime: statSync(join(nsisDir, name)).mtimeMs }))
  .sort((a, b) => b.mtime - a.mtime);
if (installers.length === 0) {
  console.error(`Ingen *-setup.exe fundet i ${nsisDir}`);
  process.exit(1);
}
const picked = installers[0];
console.log(`Bruger nyeste installer: ${picked.name}`);

const releaseDir = join(root, "release");
mkdirSync(releaseDir, { recursive: true });

const source = join(nsisDir, picked.name);
const staged = join(releaseDir, RELEASE_ASSET_NAME);
copyFileSync(source, staged);

const bytes = readFileSync(staged);
const sha256 = createHash("sha256").update(bytes).digest("hex");

const manifest = {
  version,
  buildId,
  url: `${REPO}/releases/latest/download/${RELEASE_ASSET_NAME}`,
  sha256,
  size: bytes.length,
  notes: "",
};
writeFileSync(join(releaseDir, "latest.json"), JSON.stringify(manifest, null, 2) + "\n");

console.log(`Release klar i ${releaseDir}`);
console.log(`  ${RELEASE_ASSET_NAME} (${(bytes.length / 1024 / 1024).toFixed(1)} MB)`);
console.log(`  latest.json  buildId=${buildId}  sha256=${sha256.slice(0, 12)}…`);
console.log(`Kør "npm run publish" for at udgive på GitHub.`);
