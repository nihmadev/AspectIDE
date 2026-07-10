import { marked } from "marked";
import { Eye, EyeOff } from "lucide-react";
import { lazy, Suspense, useMemo, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import type { editor } from "monaco-editor";
import { useTranslation } from '../../lib/i18n/useTranslation';
import { sanitizeMarkdownHtml } from '../../lib/preview/sanitize-html';
import { useDebouncedValue } from '../../lib/hooks/use-debounced-value';
import type { DocumentSnapshot } from '../../lib/types/index';

const MonacoEditor = lazy(() => import("@monaco-editor/react"));

// Re-rendering the preview on every keypress is wasted work for large docs; the source
// pane stays live while the preview catches up shortly after typing pauses.
const PREVIEW_DEBOUNCE_MS = 180;

/** Persisted preview visibility вЂ” one preference for all markdown tabs. */
const PREVIEW_VISIBLE_KEY = "aspect.markdownEditor.previewVisible";

function loadPreviewVisible(): boolean {
  try {
    return localStorage.getItem(PREVIEW_VISIBLE_KEY) !== "false";
  } catch {
    return true;
  }
}

function persistPreviewVisible(visible: boolean) {
  try {
    localStorage.setItem(PREVIEW_VISIBLE_KEY, String(visible));
  } catch {
    /* preference persistence is best-effort */
  }
}

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
  const [previewVisible, setPreviewVisible] = useState(loadPreviewVisible);
  // Debounce only the (expensive, secondary) preview render вЂ” document state still
  // updates synchronously via incremental edits so saves/LSP stay accurate.
  const debouncedText = useDebouncedValue(document.text, PREVIEW_DEBOUNCE_MS);
  const previewHtml = useMemo(() => {
    if (!previewVisible) return "";
    try {
      // Workspace markdown is untrusted: parse then sanitize before injecting as HTML.
      return sanitizeMarkdownHtml(marked.parse(debouncedText || "", { async: false }) as string);
    } catch {
      return `<pre>${escapeHtml(debouncedText)}</pre>`;
    }
  }, [debouncedText, previewVisible]);

  const togglePreview = () => {
    setPreviewVisible((visible) => {
      persistPreviewVisible(!visible);
      return !visible;
    });
  };

  const editorPanel = (
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
  );

  // Preview hidden: single full-width source pane with a floating "show" button,
  // so the affordance to bring the preview back is always visible.
  if (!previewVisible) {
    return (
      <div className="markdown-editor-pane" data-preview-hidden>
        {editorPanel}
        <button
          type="button"
          className="markdown-editor-preview-toggle markdown-editor-preview-toggle-floating"
          onClick={togglePreview}
          title={t("markdownEditor.showPreview")}
          aria-label={t("markdownEditor.showPreview")}
        >
          <Eye size={13} />
          <span>{t("markdownEditor.preview")}</span>
        </button>
      </div>
    );
  }

  return (
    <div className="markdown-editor-pane">
      <Group orientation="horizontal" className="markdown-editor-split">
        <Panel defaultSize={52} minSize={28}>
          {editorPanel}
        </Panel>
        <Separator className="markdown-editor-separator" />
        <Panel defaultSize={48} minSize={24}>
          <div className="markdown-editor-preview-wrap">
            <div className="markdown-editor-preview-toolbar">
              <span>{t("markdownEditor.preview")}</span>
              <button
                type="button"
                className="markdown-editor-preview-toggle"
                onClick={togglePreview}
                title={t("markdownEditor.hidePreview")}
                aria-label={t("markdownEditor.hidePreview")}
              >
                <EyeOff size={13} />
              </button>
            </div>
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
