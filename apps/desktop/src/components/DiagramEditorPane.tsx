import { lazy, Suspense, useEffect, useState } from "react";
import { Group, Panel, Separator } from "react-resizable-panels";
import { useTranslation } from "../lib/i18n/useTranslation";
import { renderDiagramPreview } from "../lib/diagramPreview";
import type { DocumentSnapshot } from "../lib/types";

const MonacoEditor = lazy(() => import("@monaco-editor/react"));

type DiagramEditorPaneProps = {
  document: DocumentSnapshot;
  fontFamily: string;
  fontLigatures: boolean;
  fontSize: number;
  lineHeight: number;
  minimap: boolean;
  onChange: (value: string) => void;
  readOnly: boolean;
  renderWhitespace: "none" | "boundary" | "selection" | "trailing" | "all";
  smoothScrolling: boolean;
  tabSize: number;
  wordWrap: "on" | "off";
};

export function DiagramEditorPane({
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
}: DiagramEditorPaneProps) {
  const { t } = useTranslation();
  const [previewHtml, setPreviewHtml] = useState("<p class=\"diagram-preview-empty\">…</p>");
  const [previewError, setPreviewError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void renderDiagramPreview(document.text || "", document.path).then((result) => {
      if (cancelled) return;
      setPreviewHtml(result.html);
      setPreviewError(result.error);
    });
    return () => {
      cancelled = true;
    };
  }, [document.path, document.text]);

  return (
    <div className="markdown-editor-pane diagram-editor-pane">
      <Group orientation="horizontal" className="markdown-editor-split">
        <Panel defaultSize={52} minSize={28}>
          <Suspense fallback={<div className="editor-loading">{t("editor.status.loading")}</div>}>
            <MonacoEditor
              height="100%"
              theme="vs-dark"
              path={`diagram-source:${document.id}`}
              language={document.language_id}
              value={document.text}
              onChange={(value) => onChange(value ?? "")}
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
          <div className="markdown-editor-preview diagram-editor-preview">
            {previewError ? <div className="diagram-preview-error">{previewError}</div> : null}
            <div dangerouslySetInnerHTML={{ __html: previewHtml }} />
          </div>
        </Panel>
      </Group>
    </div>
  );
}