import { error as logError } from "@tauri-apps/plugin-log";

export async function reportError(message: string): Promise<void> {
  console.error(message);
  try {
    await logError(message);
  } catch {
    // Browserbaserede komponenttests har ingen Tauri-logtransport.
  }
}

export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (typeof error === "object" && error !== null && "message" in error) {
    const message = (error as { message: unknown }).message;
    if (typeof message === "string" && message) return message;
  }
  return String(error);
}

export function reportFailure(context: string, reason: unknown): void {
  void reportError(`${context}: ${errorMessage(reason)}`);
}

/** Fire-and-forget IPC: fejl logges med kontekst, aldrig tavst. */
export function fireIpc(promise: Promise<unknown>, context: string): void {
  promise.catch((reason: unknown) => reportFailure(context, reason));
}
