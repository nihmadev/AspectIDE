import { defaultEditorPreferences, EDITOR_PREFERENCES_KEY, mergeEditorPreferences, type EditorPreferences } from "./preferences";
import { useLuxStore } from "./../store/index";
import { luxCommands } from "./../tauri/commands";

const settingsScope = "user" as const;
const editorFontSizeStep = 1;

export function updateEditorPreference(patch: Partial<EditorPreferences>) {
  const store = useLuxStore.getState();
  const nextPreferences = mergeEditorPreferences(store.editorPreferences, patch);
  store.setEditorPreferences(nextPreferences);
  void luxCommands.settingsSet(settingsScope, EDITOR_PREFERENCES_KEY, nextPreferences).catch(() => undefined);
  return nextPreferences;
}

export function toggleEditorWordWrap() {
  const wordWrap = useLuxStore.getState().editorPreferences.wordWrap === "on" ? "off" : "on";
  return updateEditorPreference({ wordWrap });
}

export function toggleEditorMinimap() {
  const minimap = !useLuxStore.getState().editorPreferences.minimap;
  return updateEditorPreference({ minimap });
}

export function zoomEditorFontIn() {
  return updateEditorFontSize(editorFontSizeStep);
}

export function zoomEditorFontOut() {
  return updateEditorFontSize(-editorFontSizeStep);
}

export function resetEditorFontZoom() {
  return updateEditorPreference({
    fontSize: defaultEditorPreferences.fontSize,
    lineHeight: defaultEditorPreferences.lineHeight,
  });
}

export function updateEditorFontSize(delta: number) {
  const { fontSize, lineHeight } = useLuxStore.getState().editorPreferences;
  return updateEditorPreference({
    fontSize: fontSize + delta,
    lineHeight: lineHeight + delta,
  });
}
