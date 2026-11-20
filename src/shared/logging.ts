import { error as logError } from "@tauri-apps/plugin-log";

export async function reportError(message: string): Promise<void> {
  console.error(message);
  try {
    await logError(message);
  } catch {
    // Browserbaserede komponenttests har ingen Tauri-logtransport.
  }
}
