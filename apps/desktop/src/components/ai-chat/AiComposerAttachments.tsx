import { AtSign, Braces, FileCode2, FileText, ImagePlus, X, ZoomIn } from "lucide-react";
import { memo, useEffect, useMemo, useState } from "react";
import type { ComposerAttachment } from "../../lib/aiChatComposerAttachments";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

export type AiComposerAttachmentView = {
  id: string;
  name: string;
  size: number;
  kind: "file" | "editor" | "mention" | "selection";
  isImage?: boolean;
  previewUrl?: string;
  detail?: string;
};

type AiComposerAttachmentsProps = {
  attachments: AiComposerAttachmentView[];
  draggingFiles: boolean;
  removeAttachment: (id: string) => void;
  t: TranslateFn;
};

export function mapComposerAttachments(attachments: ComposerAttachment[]): AiComposerAttachmentView[] {
  return attachments.map((attachment) => {
    if (attachment.kind === "mention") {
      return {
        id: attachment.id,
        name: attachment.name,
        size: attachment.size,
        kind: "mention",
        detail: attachment.path ?? attachment.mentionType,
      };
    }
    if (attachment.kind === "selection") {
      return {
        id: attachment.id,
        name: attachment.name,
        size: attachment.size,
        kind: "selection",
        detail: attachment.path,
      };
    }
    return {
      id: attachment.id,
      name: attachment.name,
      size: attachment.size,
      kind: attachment.kind,
      isImage: attachment.kind === "file" ? attachment.isImage : false,
      previewUrl: attachment.kind === "file" ? attachment.previewUrl : undefined,
    };
  });
}

// Memoized like its sibling composer sections: the composer parent re-renders on
// every streamed token, so identity-stable props let the attachment tray bail out
// instead of reconciling its full chip/image list when attachments are unchanged.
export const AiComposerAttachments = memo(function AiComposerAttachments({ attachments, draggingFiles, removeAttachment, t }: AiComposerAttachmentsProps) {
  const [previewId, setPreviewId] = useState<string | null>(null);
  const imageAttachments = useMemo(
    () => attachments.filter((attachment) => attachment.kind === "file" && attachment.isImage && attachment.previewUrl),
    [attachments],
  );
  const otherAttachments = useMemo(
    () => attachments.filter((attachment) => attachment.kind !== "file" || !attachment.isImage),
    [attachments],
  );
  const chipIcon = (attachment: AiComposerAttachmentView) => {
    if (attachment.kind === "mention") return AtSign;
    if (attachment.kind === "selection") return Braces;
    if (attachment.kind === "editor") return FileCode2;
    return FileText;
  };
  const previewAttachment = attachments.find((attachment) => attachment.id === previewId) ?? null;

  useEffect(() => {
    if (!previewId) return;
    if (!previewAttachment?.previewUrl) setPreviewId(null);
  }, [previewAttachment?.previewUrl, previewId]);

  useEffect(() => {
    if (!previewId) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setPreviewId(null);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [previewId]);

  if (attachments.length === 0 && !draggingFiles) return null;

  return (
    <>
      {draggingFiles && (
        <div className="ai-composer-dropzone" aria-hidden="true">
          <ImagePlus size={18} strokeWidth={2} />
          <span>{t("aiChat.attachments.dropHint")}</span>
        </div>
      )}
      {attachments.length > 0 && (
        <div className="ai-composer-attachments" aria-label={t("aiChat.attachments.aria")}>
          {imageAttachments.length > 0 && (
            <div className="ai-composer-image-strip" role="list">
              {imageAttachments.map((attachment) => (
                <figure className="ai-composer-image-card" key={attachment.id} role="listitem">
                  <button
                    type="button"
                    className="ai-composer-image-preview"
                    aria-label={t("aiChat.attachments.previewAria", { name: attachment.name })}
                    onClick={() => setPreviewId(attachment.id)}
                  >
                    <img src={attachment.previewUrl} alt="" draggable={false} decoding="async" />
                    <span className="ai-composer-image-zoom" aria-hidden="true">
                      <ZoomIn size={14} />
                    </span>
                  </button>
                  <figcaption className="ai-composer-image-meta">
                    <span className="ai-composer-image-name" title={attachment.name}>
                      {attachment.name}
                    </span>
                    <span className="ai-composer-image-size">{formatBytes(attachment.size, t)}</span>
                  </figcaption>
                  <button
                    type="button"
                    className="ai-composer-image-remove"
                    aria-label={t("aiChat.attachment.removeAria", { name: attachment.name })}
                    title={t("common.remove")}
                    onClick={() => removeAttachment(attachment.id)}
                  >
                    <X size={13} />
                  </button>
                </figure>
              ))}
            </div>
          )}
          {otherAttachments.length > 0 && (
            <div className="ai-attachment-list">
              {otherAttachments.map((attachment) => {
                const size = attachment.size > 0 ? formatBytes(attachment.size, t) : null;
                const Icon = chipIcon(attachment);
                const label = attachment.kind === "editor"
                  ? t("aiChat.attachment.editorTab", { name: attachment.name })
                  : attachment.kind === "selection"
                    ? t("aiChat.attachment.selection", { name: attachment.name })
                    : attachment.name;
                const path = attachment.detail;
                return (
                  <div
                    className="ai-attachment-card"
                    data-kind={attachment.kind}
                    key={attachment.id}
                    title={path ? `${label}\n${path}` : label}
                  >
                    <span className="ai-attachment-card-icon" aria-hidden="true">
                      <Icon size={15} />
                    </span>
                    <span className="ai-attachment-card-body">
                      <span className="ai-attachment-card-name">{label}</span>
                      {path && <span className="ai-attachment-card-path">{path}</span>}
                    </span>
                    {size && <small className="ai-attachment-card-size">{size}</small>}
                    <button
                      type="button"
                      className="ai-attachment-card-remove"
                      aria-label={t("aiChat.attachment.removeAria", { name: attachment.name })}
                      title={t("common.remove")}
                      onClick={() => removeAttachment(attachment.id)}
                    >
                      <X size={13} />
                    </button>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      )}
      {previewAttachment?.previewUrl && (
        <div className="ai-composer-preview-lightbox" role="dialog" aria-modal="true" aria-label={t("aiChat.attachments.lightboxLabel")}>
          <button type="button" className="ai-composer-preview-backdrop" aria-label={t("common.close")} onClick={() => setPreviewId(null)} />
          <div className="ai-composer-preview-dialog">
            <header className="ai-composer-preview-head">
              <div>
                <strong>{previewAttachment.name}</strong>
                <span>{formatBytes(previewAttachment.size, t)}</span>
              </div>
              <button type="button" aria-label={t("common.close")} title={t("common.close")} onClick={() => setPreviewId(null)}>
                <X size={16} />
              </button>
            </header>
            <div className="ai-composer-preview-stage">
              <img src={previewAttachment.previewUrl} alt={previewAttachment.name} draggable={false} />
            </div>
          </div>
        </div>
      )}
    </>
  );
});

function formatBytes(bytes: number, t: TranslateFn) {
  if (bytes < 1024) return t("common.fileSize.bytes", { bytes });
  const kilobytes = bytes / 1024;
  if (kilobytes < 1024) return t("common.fileSize.kilobytes", { kilobytes: kilobytes >= 10 ? kilobytes.toFixed(0) : kilobytes.toFixed(1) });
  const megabytes = kilobytes / 1024;
  return t("common.fileSize.megabytes", { megabytes: megabytes >= 10 ? megabytes.toFixed(0) : megabytes.toFixed(1) });
}