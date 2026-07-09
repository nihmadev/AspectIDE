/**
 * Runtime-environment guards shared by the IPC command bridge and the event
 * subscription layer. Kept in a dependency-free module so both `tauri.ts` and
 * `tauriEvents.ts` can import them without a circular reference. `tauri.ts`
 * re-exports these to preserve every existing `from "./commands"` call site.
 */

export const isTauriRuntime = () =>
  Boolean((window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__);

export const isBrowserPreviewRuntime = () =>
  !isTauriRuntime() && import.meta.env.VITE_LUX_BROWSER_PREVIEW === "1";

export function desktopRuntimeRequiredMessage(feature: string) {
  return `${feature} requires the Lux desktop runtime. Browser fallbacks are available only in explicit preview mode.`;
}

export function createDesktopRuntimeError(feature: string) {
  return new Error(desktopRuntimeRequiredMessage(feature));
}
