import { listen } from "@tauri-apps/api/event";
import { createDesktopRuntimeError, isBrowserPreviewRuntime, isTauriRuntime } from "./runtime";

/**
 * Hardened replacement for the repeated `listen(channel, e => handler(e.payload))`
 * pattern. Every backend event stream now flows through here so that:
 *   - a malformed payload (failed `validate`) is dropped with a logged warning
 *     instead of being handed to trusted UI code,
 *   - a throwing handler is caught and reported instead of escaping the listener
 *     callback (an uncaught throw during a streamDelta/approvalRequired event used
 *     to leave the AI turn UI wedged in streaming/waiting with no normalized error),
 *   - the browser-preview no-op fallback stays in exactly one place.
 */
export type SafeListenOptions<T> = {
  /** Human-readable channel name used in error logs. */
  label: string;
  /** Optional payload validator/narrower. Return `null` to drop a malformed event. */
  validate?: (payload: unknown) => T | null;
  /** Invoked when a payload is dropped or a handler throws (channel recovery hook). */
  onError?: (error: unknown, raw: unknown) => void;
};

export async function safeListen<T>(
  channel: string,
  handler: (payload: T) => void,
  options: SafeListenOptions<T>,
): Promise<() => void> {
  if (!isTauriRuntime()) {
    if (!isBrowserPreviewRuntime()) throw createDesktopRuntimeError(`Event stream ${channel}`);
    return () => undefined;
  }

  return listen<T>(channel, (event) => {
    let payload: T | null;
    try {
      payload = options.validate ? options.validate(event.payload) : (event.payload as T);
    } catch (error) {
      reportEventError(options.label, error, event.payload, options.onError);
      return;
    }
    if (payload === null || payload === undefined) {
      reportEventError(options.label, new Error("dropped malformed event payload"), event.payload, options.onError);
      return;
    }
    try {
      handler(payload);
    } catch (error) {
      reportEventError(options.label, error, event.payload, options.onError);
    }
  });
}

function reportEventError(label: string, error: unknown, raw: unknown, onError?: (error: unknown, raw: unknown) => void) {
  console.error(`[lux] event handler failed on ${label}:`, error);
  try {
    onError?.(error, raw);
  } catch (hookError) {
    console.error(`[lux] event onError hook failed on ${label}:`, hookError);
  }
}

/** Reads a string field from an unknown payload without throwing — used by
 *  channel error hooks that need a correlation id (e.g. turnId/streamId) to
 *  synthesize a recoverable failure event. */
export function readStringField(raw: unknown, field: string): string | null {
  if (raw && typeof raw === "object" && field in raw) {
    const value = (raw as Record<string, unknown>)[field];
    if (typeof value === "string") return value;
  }
  return null;
}
