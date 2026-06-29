import { lazy, Suspense, useEffect, useMemo, useRef } from "react";
import type { editor } from "monaco-editor";

const DiffEditor = lazy(() => import("@monaco-editor/react").then((module) => ({ default: module.DiffEditor })));

const DEFAULT_LANGUAGE = "plaintext";

type AiMonacoDiffReviewProps = {
  beforeText: string;
  afterText: string;
  language?: string;
  /**
   * 1-based start line of the selected hunk in the modified text. Driven from the
   * real `FileDiffHunk.afterStartLine` (not an estimated ordinal), so selecting a
   * hunk reveals its true location in the review editor.
   */
  activeHunkLine?: number | null;
};

export function AiMonacoDiffReview({
  beforeText,
  afterText,
  language = DEFAULT_LANGUAGE,
  activeHunkLine,
}: AiMonacoDiffReviewProps) {
  const editorRef = useRef<editor.IStandaloneDiffEditor | null>(null);

  const options = useMemo<editor.IDiffEditorConstructionOptions>(() => ({
    readOnly: true,
    renderSideBySide: true,
    automaticLayout: true,
    minimap: { enabled: false },
    scrollBeyondLastLine: false,
    fontSize: 12,
    lineNumbers: "on",
    renderOverviewRuler: true,
    diffWordWrap: "on",
  }), []);

  // Reveal the selected hunk by its real line. The editor instance is not yet
  // mounted on the first run, so `onMount` performs the initial reveal too.
  useEffect(() => {
    revealHunkLine(editorRef.current, activeHunkLine);
  }, [activeHunkLine]);

  return (
    <div className="ai-monaco-diff-review">
      <Suspense fallback={<div className="ai-monaco-diff-review-loading" aria-busy="true" />}>
        <DiffEditor
          height="min(320px, 42vh)"
          language={language}
          original={beforeText}
          modified={afterText}
          options={options}
          onMount={(instance) => {
            editorRef.current = instance;
            revealHunkLine(instance, activeHunkLine);
          }}
        />
      </Suspense>
    </div>
  );
}

/** Center the modified editor on a 1-based line, clamped to the model's bounds. */
function revealHunkLine(instance: editor.IStandaloneDiffEditor | null, line: number | null | undefined) {
  if (!instance || line == null || line < 1) return;
  const modified = instance.getModifiedEditor();
  const lineCount = modified.getModel()?.getLineCount() ?? line;
  const target = Math.min(line, lineCount);
  modified.revealLineInCenter(target);
  modified.setPosition({ lineNumber: target, column: 1 });
}
