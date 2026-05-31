import type { KeybindingProfile } from "./types";

export const KEYBINDINGS_SETTINGS_KEY = "workbench.keybindings";
const CHORD_TIMEOUT_MS = 1_500;

export type KeybindingMatch = {
  command: string | null;
  preventDefault: boolean;
  pendingChord: boolean;
};

export type KeybindingContext = {
  dirtyEditors: boolean;
  editor: boolean;
  workspace: boolean;
};

export type KeybindingDispatcher = {
  handleKeyDown: (event: KeyboardEvent, context: KeybindingContext) => KeybindingMatch;
  setProfile: (profile: KeybindingProfile) => void;
};

type CompiledKeybinding = {
  command: string;
  sequence: string[];
  when: string | null;
};

export function defaultKeybindingProfile(): KeybindingProfile {
  return {
    id: "default",
    name: "Default",
    bindings: [
      { command: "workbench.action.showCommands", key: "Ctrl+Shift+P", when: null },
      { command: "workbench.action.quickOpen", key: "Ctrl+P", when: null },
      { command: "workbench.action.files.newUntitledFile", key: "Ctrl+N", when: null },
      { command: "workbench.action.openSettings", key: "Ctrl+,", when: null },
      { command: "workbench.action.openFolder", key: "Ctrl+O", when: null },
      { command: "workbench.action.toggleSidebar", key: "Ctrl+B", when: "workspace" },
      { command: "workbench.view.explorer", key: "Ctrl+Shift+E", when: "workspace" },
      { command: "workbench.view.search", key: "Ctrl+Shift+F", when: "workspace" },
      { command: "workbench.view.scm", key: "Ctrl+Shift+G", when: "workspace" },
      { command: "workbench.view.debug", key: "Ctrl+Shift+D", when: "workspace" },
      { command: "workbench.view.extensions", key: "Ctrl+Shift+X", when: "workspace" },
      { command: "workbench.action.chat.toggle", key: "Ctrl+L", when: "workspace" },
      { command: "workbench.action.terminal.toggleTerminal", key: "Ctrl+`", when: "workspace" },
      { command: "editor.action.toggleWordWrap", key: "Alt+Z", when: "editor" },
      { command: "editor.action.toggleMinimap", key: "Ctrl+M Ctrl+M", when: "editor" },
      { command: "editor.action.fontZoomIn", key: "Ctrl+=", when: "editor" },
      { command: "editor.action.fontZoomIn", key: "Ctrl+Shift+=", when: "editor" },
      { command: "editor.action.fontZoomOut", key: "Ctrl+-", when: "editor" },
      { command: "editor.action.fontZoomReset", key: "Ctrl+0", when: "editor" },
      { command: "workbench.action.files.save", key: "Ctrl+S", when: "editor" },
      { command: "workbench.action.files.saveAs", key: "Ctrl+Shift+S", when: "editor" },
      { command: "workbench.action.files.saveAll", key: "Ctrl+K Ctrl+S", when: "dirtyEditors" },
      { command: "workbench.action.closeActiveEditor", key: "Ctrl+W", when: "editor" },
      { command: "workbench.action.splitEditorRight", key: "Ctrl+\\", when: "editor" },
      { command: "workbench.action.nextEditor", key: "Ctrl+PageDown", when: "editor" },
      { command: "workbench.action.previousEditor", key: "Ctrl+PageUp", when: "editor" },
    ],
  };
}

export function formatKeybindingForDisplay(command: string, profile: KeybindingProfile) {
  return profile.bindings.find((binding) => binding.command === command)?.key.replace(/\+/g, " ");
}

export function mergeDefaultKeybindings(profile: KeybindingProfile): KeybindingProfile {
  if (profile.id.trim() !== "default") return profile;
  const bindings = [...profile.bindings];
  for (const defaultBinding of defaultKeybindingProfile().bindings) {
    if (!bindings.some((binding) => binding.command === defaultBinding.command && binding.key === defaultBinding.key)) {
      bindings.push(defaultBinding);
    }
  }
  return { ...profile, bindings };
}

export function createKeybindingDispatcher(profile: KeybindingProfile = defaultKeybindingProfile()): KeybindingDispatcher {
  let bindings = compileProfile(mergeDefaultKeybindings(profile));
  let pendingChord: string[] = [];
  let pendingSince = 0;

  const resetExpiredChord = (now: number) => {
    if (pendingChord.length > 0 && now - pendingSince > CHORD_TIMEOUT_MS) pendingChord = [];
  };

  return {
    handleKeyDown: (event, context) => {
      if (isEditableKeyEvent(event)) return { command: null, preventDefault: false, pendingChord: false };

      const now = Date.now();
      resetExpiredChord(now);

      const chord = keyChordFromEvent(event);
      const sequence = pendingChord.length > 0 ? [...pendingChord, chord] : [chord];
      const candidates = bindings.filter((binding) => sequenceMatchesPrefix(binding.sequence, sequence) && whenMatches(binding.when, context));
      if (candidates.length === 0) {
        pendingChord = [];
        return { command: null, preventDefault: false, pendingChord: false };
      }

      const exact = candidates.find((binding) => binding.sequence.length === sequence.length);
      if (exact) {
        pendingChord = [];
        return { command: exact.command, preventDefault: true, pendingChord: false };
      }

      pendingChord = sequence;
      pendingSince = now;
      return { command: null, preventDefault: true, pendingChord: true };
    },
    setProfile: (nextProfile) => {
      bindings = compileProfile(mergeDefaultKeybindings(nextProfile));
      pendingChord = [];
    },
  };
}

function compileProfile(profile: KeybindingProfile): CompiledKeybinding[] {
  return profile.bindings.flatMap((binding) => {
    const sequence = binding.key.split(/\s+/).map(normalizeKeyChord).filter(Boolean);
    if (sequence.length === 0 || binding.command.trim().length === 0) return [];
    return [{ command: binding.command.trim(), sequence, when: binding.when?.trim() || null }];
  });
}

function sequenceMatchesPrefix(candidate: string[], sequence: string[]) {
  if (sequence.length > candidate.length) return false;
  return sequence.every((chord, index) => candidate[index] === chord);
}

function keyChordFromEvent(event: KeyboardEvent) {
  const parts = [];
  if (event.ctrlKey || event.metaKey) parts.push("Ctrl");
  if (event.shiftKey) parts.push("Shift");
  if (event.altKey) parts.push("Alt");
  parts.push(keyNameFromEvent(event));
  return parts.join("+");
}

function keyNameFromEvent(event: KeyboardEvent) {
  const physicalKey = keyNameFromCode(event.code);
  if (physicalKey) return physicalKey;
  return normalizeKeyName(event.key);
}

function keyNameFromCode(code: string) {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit\d$/.test(code)) return code.slice(5);

  switch (code) {
    case "Backquote":
      return "`";
    case "Minus":
      return "-";
    case "Equal":
      return "=";
    case "NumpadAdd":
      return "=";
    case "NumpadSubtract":
      return "-";
    case "Numpad0":
      return "0";
    case "BracketLeft":
      return "[";
    case "BracketRight":
      return "]";
    case "Backslash":
      return "\\";
    case "Semicolon":
      return ";";
    case "Quote":
      return "'";
    case "Comma":
      return ",";
    case "Period":
      return ".";
    case "Slash":
      return "/";
    case "Space":
      return "Space";
    case "Escape":
      return "Escape";
    case "Tab":
      return "Tab";
    case "Enter":
      return "Enter";
    case "Backspace":
      return "Backspace";
    case "Delete":
      return "Delete";
    case "Insert":
      return "Insert";
    case "Home":
      return "Home";
    case "End":
      return "End";
    case "PageUp":
      return "PageUp";
    case "PageDown":
      return "PageDown";
    case "ArrowLeft":
      return "ArrowLeft";
    case "ArrowRight":
      return "ArrowRight";
    case "ArrowUp":
      return "ArrowUp";
    case "ArrowDown":
      return "ArrowDown";
    default:
      return /^F\d{1,2}$/.test(code) ? code : "";
  }
}

function normalizeKeyChord(chord: string) {
  const parts = chord.split("+").map((part) => part.trim()).filter(Boolean);
  if (parts.length === 0) return "";

  const modifiers = new Set<string>();
  let key = "";
  for (const part of parts) {
    const normalized = part.toLowerCase();
    if (normalized === "cmd" || normalized === "command" || normalized === "meta" || normalized === "win" || normalized === "super") modifiers.add("Ctrl");
    else if (normalized === "ctrl" || normalized === "control") modifiers.add("Ctrl");
    else if (normalized === "shift") modifiers.add("Shift");
    else if (normalized === "alt" || normalized === "option") modifiers.add("Alt");
    else key = normalizeKeyName(part);
  }
  if (!key) return "";

  const ordered = [];
  if (modifiers.has("Ctrl")) ordered.push("Ctrl");
  if (modifiers.has("Shift")) ordered.push("Shift");
  if (modifiers.has("Alt")) ordered.push("Alt");
  ordered.push(key);
  return ordered.join("+");
}

function normalizeKeyName(key: string) {
  if (key === " ") return "Space";
  const lower = key.toLowerCase();
  if (lower === "esc") return "Escape";
  if (lower === "space") return "Space";
  if (lower === "pgup") return "PageUp";
  if (lower === "pgdn") return "PageDown";
  if (lower === "arrowleft" || lower === "left") return "ArrowLeft";
  if (lower === "arrowright" || lower === "right") return "ArrowRight";
  if (lower === "arrowup" || lower === "up") return "ArrowUp";
  if (lower === "arrowdown" || lower === "down") return "ArrowDown";
  if (lower.length === 1) return lower.toUpperCase();
  if (/^f\d{1,2}$/i.test(key)) return lower.toUpperCase();
  return key.slice(0, 1).toUpperCase() + key.slice(1);
}

function whenMatches(when: string | null, context: KeybindingContext) {
  if (!when) return true;
  return when.split("&&").every((part) => {
    const term = part.trim();
    if (term === "workspace") return context.workspace;
    if (term === "editor") return context.editor;
    if (term === "dirtyEditors") return context.dirtyEditors;
    if (term === "!workspace") return !context.workspace;
    if (term === "!editor") return !context.editor;
    if (term === "!dirtyEditors") return !context.dirtyEditors;
    return false;
  });
}

function isEditableKeyEvent(event: KeyboardEvent) {
  const target = event.target;
  if (!(target instanceof HTMLElement)) return false;
  if (target.closest(".monaco-editor")) return false;
  if (target.isContentEditable) return true;
  const tagName = target.tagName.toLowerCase();
  return tagName === "input" || tagName === "textarea" || tagName === "select";
}
