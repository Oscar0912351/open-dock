// Udgiver det stagede release-par (release/Open-Dock-Setup.exe + release/latest.json)
// som et nyt GitHub-release markeret "latest". Appen slår latest.json op via den
// stabile releases/latest/download/-URL og sammenligner buildId.

import { spawnSync } from "node:child_process";
import { createHash, createPublicKey, verify } from "node:crypto";
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
const { version, buildId, sha256, signature } = manifest;
const stagedExe = join(releaseDir, "Open-Dock-Setup.exe");
if (!existsSync(stagedExe)) {
  fail(`${stagedExe} findes ikke. Kør "npm run build:tauri" først.`);
}
if (!signature) {
  fail(
    "latest.json mangler ed25519-signaturen. Kør en frisk " +
    '"npm run build:tauri" — builds uden signatur kan nye installationer ikke verificere.',
  );
}

// Verificér at den stagede exe stadig matcher manifestet — både hash og signatur.
const stagedBytes = readFileSync(stagedExe);
const actual = createHash("sha256").update(stagedBytes).digest("hex");
if (actual !== sha256) {
  fail(`SHA-256 på ${stagedExe} matcher ikke latest.json — kør "npm run build:tauri" igen.`);
}
const pubDerPrefix = Buffer.from("302a300506032b6570032100", "hex");
const pubHex = "f8403449d1bc826bdd204d77cdf099e0a7a14069f1089b23f10cfa01266d33dc";
const pubKey = createPublicKey({
  key: Buffer.concat([pubDerPrefix, Buffer.from(pubHex, "hex")]),
  format: "der",
  type: "spki",
});
// Samme kanoniske beskedformat som appen verificerer mod (signed_message i update.rs).
const signedMessage = Buffer.from(`open-dock-update-v1\n${buildId}\n${sha256}\n`, "utf8");
if (!verify(null, signedMessage, pubKey, Buffer.from(signature, "base64"))) {
  fail(
    "Signaturen i latest.json verificerer ikke mod den indlejrede offentlige nøgle. " +
    "Er filen signet med en anden nøgle? Kør en frisk build.",
  );
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
