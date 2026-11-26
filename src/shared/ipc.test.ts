import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

// Drift-vagt: ipc.ts er håndskrevet, og intet tjekker compile-time at
// kommando-strengene matcher Rusts invoke_handler. Denne test sammenligner
// de to sider, så en omdøbt/manglende kommando fejler CI i stedet for først
// at blive opdaget ved runtime. Dækker kun kommando-navne — argument-nøgler
// verificeres ikke.

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");

function ipcCommands(): Set<string> {
  const source = readFileSync(join(root, "src", "shared", "ipc.ts"), "utf8");
  const names = new Set<string>();
  for (const match of source.matchAll(/invoke(?:<[^>]*>)?\(\s*"([a-zA-Z_0-9]+)"/g)) {
    if (match[1]) names.add(match[1]);
  }
  return names;
}

function rustCommands(): Set<string> {
  const source = readFileSync(join(root, "src-tauri", "src", "lib.rs"), "utf8");
  const start = source.indexOf("generate_handler![");
  if (start < 0) throw new Error("generate_handler ikke fundet i lib.rs");
  const end = source.indexOf("])", start);
  if (end < 0) throw new Error("generate_handler blok ikke afsluttet i lib.rs");
  const block = source.slice(start, end);
  const names = new Set<string>();
  for (const match of block.matchAll(/commands::\w+::(\w+)/g)) {
    if (match[1]) names.add(match[1]);
  }
  return names;
}

describe("IPC-kommandoer er i sync mellem ipc.ts og invoke_handler", () => {
  it("har ingen kommandoer i ipc.ts som Rust ikke kender", () => {
    const missing = [...ipcCommands()].filter((name) => !rustCommands().has(name));
    expect(missing).toEqual([]);
  });

  it("har ingen kommandoer i invoke_handler som ipc.ts ikke eksponerer", () => {
    const missing = [...rustCommands()].filter((name) => !ipcCommands().has(name));
    expect(missing).toEqual([]);
  });
});
