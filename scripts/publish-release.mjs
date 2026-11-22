// Udgiver det stagede release-par (release/Open-Dock-Setup.exe + release/latest.json)
// som et nyt GitHub-release markeret "latest". Appen slår latest.json op via den
// stabile releases/latest/download/-URL og sammenligner buildId.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const releaseDir = join(root, "release");
const manifestPath = join(releaseDir, "latest.json");
const REPO = "https://github.com/Oscar0912351/open-dock";

function fail(msg) {
  console.error(`Fejl: ${msg}`);
  process.exit(1);
}

if (!existsSync(manifestPath)) {
  fail(`release/latest.json findes ikke. Kør "npm run build:tauri" først.`);
}
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const { version, buildId, sha256 } = manifest;
const stagedExe = join(releaseDir, "Open-Dock-Setup.exe");
if (!existsSync(stagedExe)) {
  fail(`${stagedExe} findes ikke. Kør "npm run build:tauri" først.`);
}

// Verificér at den stagede exe stadig matcher manifestet.
const actual = createHash("sha256").update(readFileSync(stagedExe)).digest("hex");
if (actual !== sha256) {
  fail(`SHA-256 på ${stagedExe} matcher ikke latest.json — kør "npm run build:tauri" igen.`);
}

// Advar hvis nsis-mappen rummer et nyere build end det stagede.
const nsisDir = join(root, "src-tauri", "target", "release", "bundle", "nsis");
if (existsSync(nsisDir)) {
  const newest = readdirSync(nsisDir)
    .filter((name) => name.endsWith("-setup.exe"))
    .map((name) => statSync(join(nsisDir, name)).mtimeMs)
    .sort((a, b) => b - a)[0];
  if (newest && newest > statSync(stagedExe).mtimeMs + 2000) {
    console.warn("Advarsel: nsis-mappen indeholder et nyere build end release/.");
    console.warn('  Kør "npm run build:tauri" igen hvis du vil udgive det i stedet.');
  }
}

// gh skal være installeret og logget ind.
const auth = spawnSync("gh", ["auth", "status"], { stdio: "pipe" });
if (auth.error || auth.status !== 0) {
  fail('gh CLI er ikke installeret eller ikke logget ind. Kør "gh auth login" først.');
}

const tag = `v${version}-b${buildId}`;
const date = new Date(buildId * 1000).toLocaleString("da-DK");
const title = `Open Dock ${version} (build ${date})`;

console.log(`Udgiver ${tag} til ${REPO} …`);
const args = [
  "release", "create", tag,
  stagedExe,
  manifestPath,
  "--repo", "Oscar0912351/open-dock",
  "--latest",
  "--target", "main",
  "--title", title,
  "--notes", `Bygget ${date}.`,
];
const gh = spawnSync("gh", args, { stdio: "inherit" });
if (gh.status !== 0) {
  console.error("");
  fail(
    `gh release create fejlede. Er kodeændringerne pushet til main? ` +
    `(Tag-target skal findes på remote.)`
  );
}

console.log(`Udgivet: ${REPO}/releases/tag/${tag}`);
console.log(`Manifest-URL: ${REPO}/releases/latest/download/latest.json`);
