export const EDITOR_PREFERENCES_KEY = "editor.preferences";

export type WordWrapSetting = "off" | "on";
export type RenderWhitespaceSetting = "none" | "selection" | "all";

export type EditorPreferences = {
  /** Code editor font family; empty string = the built-in mono stack. */
  fontFamily: string;
  /** Application UI font family; empty string = the built-in UI stack. */
  uiFontFamily: string;
  /** AI chat panel font family; empty string = inherit the UI font. */
  chatFontFamily: string;
  fontSize: number;
  lineHeight: number;
  tabSize: number;
  wordWrap: WordWrapSetting;
  minimap: boolean;
  mouseWheelZoom: boolean;
  fontLigatures: boolean;
  smoothScrolling: boolean;
  renderWhitespace: RenderWhitespaceSetting;
  unicodeHighlightAmbiguousCharacters: boolean;
  /** Auto-open (and focus) files the agent creates or edits. Off keeps the agent's
   *  edits from stealing the editor / piling up tabs; already-open files still sync. */
  autoOpenEditedFiles: boolean;
};

// Mirrors --font-ui in styles/tokens.css; used when composing a custom UI font
// with graceful fallbacks (the stylesheet default stays authoritative when unset).
export const DEFAULT_UI_FONT_STACK = '-apple-system, BlinkMacSystemFont, "Segoe UI", "Inter", ui-sans-serif, system-ui, sans-serif';
// The editor stack previously hardcoded across Monaco/diagram/markdown panes.
export const DEFAULT_EDITOR_FONT_STACK = "JetBrains Mono, Cascadia Code, Consolas, monospace";

export const defaultEditorPreferences: EditorPreferences = {
  fontFamily: "",
  uiFontFamily: "",
  chatFontFamily: "",
  fontSize: 13,
  lineHeight: 21,
  tabSize: 2,
  wordWrap: "off",
  minimap: true,
  mouseWheelZoom: true,
  fontLigatures: true,
  smoothScrolling: true,
  renderWhitespace: "selection",
  unicodeHighlightAmbiguousCharacters: false,
  autoOpenEditedFiles: true,
};

export function mergeEditorPreferences(current: EditorPreferences, patch: Partial<EditorPreferences>) {
  return normalizeEditorPreferences({ ...current, ...patch });
}

export function normalizeEditorPreferences(value: unknown): EditorPreferences {
  const source = isRecord(value) ? value : {};
  return {
    fontFamily: sanitizeFontFamily(source.fontFamily),
    uiFontFamily: sanitizeFontFamily(source.uiFontFamily),
    chatFontFamily: sanitizeFontFamily(source.chatFontFamily),
    fontSize: clampInteger(source.fontSize, 8, 32, defaultEditorPreferences.fontSize),
    lineHeight: clampInteger(source.lineHeight, 12, 48, defaultEditorPreferences.lineHeight),
    tabSize: clampInteger(source.tabSize, 2, 8, defaultEditorPreferences.tabSize),
    wordWrap: source.wordWrap === "on" ? "on" : "off",
    minimap: typeof source.minimap === "boolean" ? source.minimap : defaultEditorPreferences.minimap,
    mouseWheelZoom: typeof source.mouseWheelZoom === "boolean" ? source.mouseWheelZoom : defaultEditorPreferences.mouseWheelZoom,
    fontLigatures: typeof source.fontLigatures === "boolean" ? source.fontLigatures : defaultEditorPreferences.fontLigatures,
    smoothScrolling: typeof source.smoothScrolling === "boolean" ? source.smoothScrolling : defaultEditorPreferences.smoothScrolling,
    renderWhitespace: isRenderWhitespaceSetting(source.renderWhitespace) ? source.renderWhitespace : defaultEditorPreferences.renderWhitespace,
    unicodeHighlightAmbiguousCharacters: typeof source.unicodeHighlightAmbiguousCharacters === "boolean"
      ? source.unicodeHighlightAmbiguousCharacters
      : defaultEditorPreferences.unicodeHighlightAmbiguousCharacters,
    autoOpenEditedFiles: typeof source.autoOpenEditedFiles === "boolean"
      ? source.autoOpenEditedFiles
      : defaultEditorPreferences.autoOpenEditedFiles,
  };
}

/**
 * A single font-family name from settings. Quotes/braces/semicolons are stripped so a
 * persisted value can never break out of the CSS `font-family` slot it is injected
 * into (inline style vars, Monaco options), and length is capped defensively.
 */
export function sanitizeFontFamily(value: unknown): string {
  if (typeof value !== "string") return "";
  return value.replace(/["'`;{}\\<>]/g, "").trim().slice(0, 80);
}

/** Compose `family` (quoted) ahead of a fallback stack; empty family = the stack itself. */
export function withFontFallback(family: string, fallbackStack: string): string {
  const cleaned = sanitizeFontFamily(family);
  return cleaned ? `"${cleaned}", ${fallbackStack}` : fallbackStack;
}

function clampInteger(value: unknown, min: number, max: number, fallback: number) {
  const numberValue = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(numberValue)) return fallback;
  return Math.min(max, Math.max(min, Math.round(numberValue)));
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isRenderWhitespaceSetting(value: unknown): value is RenderWhitespaceSetting {
  return value === "none" || value === "selection" || value === "all";
}
