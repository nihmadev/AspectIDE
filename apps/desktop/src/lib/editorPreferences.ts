export const EDITOR_PREFERENCES_KEY = "editor.preferences";

export type WordWrapSetting = "off" | "on";
export type RenderWhitespaceSetting = "none" | "selection" | "all";

export type EditorPreferences = {
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

export const defaultEditorPreferences: EditorPreferences = {
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
