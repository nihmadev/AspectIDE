import { lazy, Suspense, useEffect, useMemo, useRef } from "react";
import type { editor } from "monaco-editor";
import type { FileDiffHunk } from "../../lib/aiFileDiffHunks";

const DiffEditor = lazy(() => import("@monaco-editor/react").then((module) => ({ default: module.DiffEditor })));

type AiMonacoDiffReviewProps = {
  beforeText: string;
  afterText: string;
  language?: string;
  activeHunkId?: string | null;
  onHunkPositions?: (positions: Array<{ hunkId: string; lineNumber: number }>) => void;
};

export function AiMonacoDiffReview({
  beforeText,
  afterText,
  language = "plaintext",
  activeHunkId,
  onHunkPositions,
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

  useEffect(() => {
    const instance = editorRef.current;
    if (!instance || !activeHunkId) return;
    const modified = instance.getModifiedEditor();
    const line = findHunkLine(afterText, activeHunkId);
    if (line > 0) {
      modified.revealLineInCenter(line);
      modified.setPosition({ lineNumber: line, column: 1 });
    }
  }, [activeHunkId, afterText]);

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
            onHunkPositions?.([]);
          }}
        />
      </Suspense>
    </div>
  );
}

function findHunkLine(afterText: string, hunkId: string) {
  const match = hunkId.match(/hunk-(\d+)/);
  if (!match) return 1;
  const index = Number(match[1]) - 1;
  const lines = afterText.split(/\r?\n/);
  return Math.min(lines.length, Math.max(1, index * 8 + 1));
}