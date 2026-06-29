import { marked } from "marked";
import { lazy, Suspense, useMemo } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import type { editor } from "monaco-editor";
import { useTranslation } from "../lib/i18n/useTranslation";
import { sanitizeMarkdownHtml } from "../lib/sanitizeHtml";
import { useDebouncedValue } from "../lib/useDebouncedValue";
import type { DocumentSnapshot } from "../lib/types";

const MonacoEditor = lazy(() => import("@monaco-editor/react"));

// Re-rendering the preview on every keypress is wasted work for large docs; the source
// pane stays live while the preview catches up shortly after typing pauses.
const PREVIEW_DEBOUNCE_MS = 180;

type MarkdownEditorPaneProps = {
  document: DocumentSnapshot;
  fontFamily: string;
  fontLigatures: boolean;
  fontSize: number;
  lineHeight: number;
  minimap: boolean;
  onChange: (value: string | undefined, event: editor.IModelContentChangedEvent) => void;
  readOnly: boolean;
  renderWhitespace: "none" | "boundary" | "selection" | "trailing" | "all";
  smoothScrolling: boolean;
  tabSize: number;
  wordWrap: "on" | "off";
};

marked.setOptions({ gfm: true, breaks: true });

export function MarkdownEditorPane({
  document,
  fontFamily,
  fontLigatures,
  fontSize,
  lineHeight,
  minimap,
  onChange,
  readOnly,
  renderWhitespace,
  smoothScrolling,
  tabSize,
  wordWrap,
}: MarkdownEditorPaneProps) {
  const { t } = useTranslation();
  // Debounce only the (expensive, secondary) preview render — document state still
  // updates synchronously via incremental edits so saves/LSP stay accurate.
  const debouncedText = useDebouncedValue(document.text, PREVIEW_DEBOUNCE_MS);
  const previewHtml = useMemo(() => {
    try {
      // Workspace markdown is untrusted: parse then sanitize before injecting as HTML.
      return sanitizeMarkdownHtml(marked.parse(debouncedText || "", { async: false }) as string);
    } catch {
      return `<pre>${escapeHtml(debouncedText)}</pre>`;
    }
  }, [debouncedText]);

  return (
    <div className="markdown-editor-pane">
      <Group orientation="horizontal" className="markdown-editor-split">
        <Panel defaultSize={52} minSize={28}>
          <Suspense fallback={<div className="editor-loading">{t("editor.status.loading")}</div>}>
            <MonacoEditor
              height="100%"
              theme="vs-dark"
              path={`markdown-source:${document.id}`}
              language={document.language_id}
              value={document.text}
              onChange={onChange}
              options={{
                automaticLayout: true,
                fontFamily,
                fontLigatures,
                fontSize,
                lineHeight,
                minimap: { enabled: minimap, scale: 0.75 },
                mouseWheelZoom: false,
                padding: { top: 18, bottom: 18 },
                readOnly,
                renderWhitespace,
                smoothScrolling,
                scrollBeyondLastLine: false,
                tabSize,
                wordWrap,
              }}
            />
          </Suspense>
        </Panel>
        <Separator className="markdown-editor-separator" />
        <Panel defaultSize={48} minSize={24}>
          <div className="markdown-editor-preview-wrap">
            <div className="markdown-editor-preview-toolbar">{t("markdownEditor.preview")}</div>
            <div
              className="markdown-editor-preview ai-chat-markdown"
              dangerouslySetInnerHTML={{ __html: previewHtml }}
            />
          </div>
        </Panel>
      </Group>
    </div>
  );
}

function escapeHtml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}
