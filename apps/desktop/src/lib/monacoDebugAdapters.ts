import type { editor } from "monaco-editor";
import { normalizePath } from "./fileTree";
import type { MonacoDisposable, MonacoEditorInstance, MonacoInstance } from "./monacoLspAdapters";
import type { DebugResolvedBreakpoint, DebugSourceBreakpoint, DocumentSnapshot } from "./types";

export function applyDebugBreakpointDecorations(
  editorInstance: MonacoEditorInstance | null,
  monaco: MonacoInstance | null,
  previousDecorations: string[],
  document: DocumentSnapshot,
  sourceBreakpoints: DebugSourceBreakpoint[],
  resolvedBreakpoints: DebugResolvedBreakpoint[],
) {
  if (!editorInstance || !monaco || !document.path) return previousDecorations;

  const resolvedByLine = new Map(resolvedBreakpoints.map((breakpoint) => [breakpoint.line, breakpoint]));
  return editorInstance.deltaDecorations(previousDecorations, sourceBreakpoints.map((breakpoint) => {
    const resolved = resolvedByLine.get(breakpoint.line);
    const stateClass = resolved ? (resolved.verified ? "debug-breakpoint-verified" : "debug-breakpoint-unverified") : "debug-breakpoint-pending";
    const message = resolved?.message ?? (resolved?.verified ? "Verified breakpoint" : "Breakpoint pending adapter verification");
    return {
      range: new monaco.Range(breakpoint.line, 1, breakpoint.line, 1),
      options: {
        glyphMarginClassName: `debug-breakpoint-glyph ${stateClass}`,
        glyphMarginHoverMessage: { value: message },
      },
    } satisfies editor.IModelDeltaDecoration;
  }));
}

export function registerDebugBreakpointGutter(
  editorInstance: MonacoEditorInstance,
  monaco: MonacoInstance,
  document: DocumentSnapshot,
  onToggleBreakpoint: (path: string, line: number) => void,
): MonacoDisposable {
  return editorInstance.onMouseDown((event) => {
    if (!document.path) return;
    const targetType = event.target.type;
    const isBreakpointTarget = targetType === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN
      || targetType === monaco.editor.MouseTargetType.GUTTER_LINE_NUMBERS;
    const lineNumber = event.target.position?.lineNumber;
    if (!isBreakpointTarget || !lineNumber || lineNumber < 1) return;
    onToggleBreakpoint(normalizePath(document.path), lineNumber);
  });
}
