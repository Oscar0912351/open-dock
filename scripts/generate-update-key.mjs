// Generér det ed25519-nøglepar som release-signeringen bruger.
// Privatnøglen skrives til %USERPROFILE%/.open-dock/update-signing-key.pem
// (uden for repoet — den må ALDRIG commit'es). Den offentlige nøgle printes
// som hex og indlejres i src-tauri/src/update.rs som UPDATE_PUBLIC_KEY.
//
// Kør ÉN gang:  node scripts/generate-update-key.mjs
// Bevar privatnøglen sikkert (backup!). Mistes den, kan eksisterende
// installationer aldrig opdateres igen — så skal en ny nøgle indlejres og
// brugerne installere manuelt en gang.

import { generateKeyPairSync } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const keyDir = join(homedir(), ".open-dock");
const keyPath = join(keyDir, "update-signing-key.pem");

if (existsSync(keyPath)) {
  console.error(`Nøglen findes allerede: ${keyPath}`);
  console.error("Slet den kun hvis du vil rottere nøglen (gamle installationer kan så ikke opdateres).");
  process.exit(1);
}

mkdirSync(keyDir, { recursive: true, mode: 0o700 });

const { publicKey, privateKey } = generateKeyPairSync("ed25519");
const privPem = privateKey.export({ type: "pkcs8", format: "pem" });
writeFileSync(keyPath, privPem, { mode: 0o600 });
try {
  chmodSync(keyPath, 0o600);
} catch {
  // Windows ACL'er håndhæves ikke via chmod — filen ligger i brugerens egen profil.
}

// Ed25519 SPKI-DER er et 12-bytes præfiks efterfulgt af de rå 32 nøglebytes.
const spki = publicKey.export({ type: "spki", format: "der" });
const raw = Buffer.from(spki).subarray(-32);

console.log(`Privatnøgle skrevet til: ${keyPath}`);
console.log("");
console.log("Offentlig nøgle (hex) — indlejr denne i update.rs som UPDATE_PUBLIC_KEY:");
console.log(raw.toString("hex"));
