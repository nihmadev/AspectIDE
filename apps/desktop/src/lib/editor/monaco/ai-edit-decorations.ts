import type { editor } from "monaco-editor";
import type { PendingFileReview } from "./../../aspector/utils/pending-file-review";
import type { MonacoEditorInstance, MonacoInstance } from "./lsp-adapters";

export type AiEditDecorationState = {
  decorationIds: string[];
  viewZoneIds: string[];
};

export function createEmptyAiEditDecorationState(): AiEditDecorationState {
  return { decorationIds: [], viewZoneIds: [] };
}

export function applyAiEditDecorations(
  editorInstance: MonacoEditorInstance | null,
  monaco: MonacoInstance | null,
  previous: AiEditDecorationState,
  review: PendingFileReview | null,
): AiEditDecorationState {
  if (!editorInstance || !monaco) return createEmptyAiEditDecorationState();

  let viewZoneIds: string[] = [];
  editorInstance.changeViewZones((accessor) => {
    for (const zoneId of previous.viewZoneIds) accessor.removeZone(zoneId);
    viewZoneIds = [];
    if (!review || review.status !== "pending") return;

    const accepted = new Set(review.acceptedHunkIds);
    for (const hunk of review.hunks) {
      if (!accepted.has(hunk.id) || hunk.beforeLineCount <= 0) continue;
      const zoneId = accessor.addZone({
        afterLineNumber: Math.max(0, hunk.afterStartLine - 1),
        heightInLines: hunk.beforeLineCount,
        domNode: buildDeletedZoneNode(hunk.beforeLines),
      });
      viewZoneIds.push(zoneId);
    }
  });

  if (!review || review.status !== "pending") {
    const decorationIds = editorInstance.deltaDecorations(previous.decorationIds, []);
    return { decorationIds, viewZoneIds: [] };
  }

  const accepted = new Set(review.acceptedHunkIds);
  const decorations: editor.IModelDeltaDecoration[] = [];

  for (const hunk of review.hunks) {
    if (!accepted.has(hunk.id) || hunk.afterLineCount <= 0) continue;
    decorations.push({
      range: new monaco.Range(
        hunk.afterStartLine,
        1,
        hunk.afterStartLine + hunk.afterLineCount - 1,
        1,
      ),
      options: {
        isWholeLine: true,
        className: "ai-edit-line-added",
        linesDecorationsClassName: "ai-edit-gutter-added",
        overviewRuler: {
          color: "#3fb950b3",
          position: monaco.editor.OverviewRulerLane.Right,
        },
      },
    });
  }

  const decorationIds = editorInstance.deltaDecorations(previous.decorationIds, decorations);
  return { decorationIds, viewZoneIds };
}

function buildDeletedZoneNode(lines: string[]) {
  const root = document.createElement("div");
  root.className = "ai-edit-deleted-zone";
  for (const line of lines) {
    const row = document.createElement("div");
    row.className = "ai-edit-deleted-line";
    row.textContent = line.length > 0 ? line : " ";
    root.appendChild(row);
  }
  return root;
}