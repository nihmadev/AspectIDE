import {
  isVisionImageFile,
  maxMessageImagePreviewBytes,
  readFileAsDataUrl,
} from "./aiFileContext";
import type { AiChatMessageAttachment } from "./aiChatTypes";

export type ComposerFileAttachment = {
  kind: "file";
  file: File;
  id: string;
  name: string;
  size: number;
  isImage: boolean;
  previewUrl?: string;
};

export type ComposerEditorAttachment = {
  kind: "editor";
  documentId: string;
  id: string;
  name: string;
  size: number;
};

export type ComposerMentionAttachment = {
  kind: "mention";
  id: string;
  mentionType: "file" | "folder" | "symbol" | "codebase" | "docs";
  name: string;
  size: number;
  path?: string;
  symbolName?: string;
  line?: number;
  column?: number;
};

export type ComposerSelectionAttachment = {
  kind: "selection";
  id: string;
  documentId: string;
  name: string;
  path: string;
  size: number;
  startLine: number;
  endLine: number;
  startColumn: number;
  endColumn: number;
  text: string;
  languageId: string;
};

export type ComposerAttachment =
  | ComposerFileAttachment
  | ComposerEditorAttachment
  | ComposerMentionAttachment
  | ComposerSelectionAttachment;

export function createComposerMentionAttachment(candidate: {
  mentionType: ComposerMentionAttachment["mentionType"];
  name: string;
  path?: string;
  symbolName?: string;
  line?: number;
  column?: number;
}): ComposerMentionAttachment {
  return {
    kind: "mention",
    id: `mention:${candidate.mentionType}:${candidate.path ?? candidate.name}:${candidate.line ?? 0}:${candidate.symbolName ?? ""}:${candidate.column ?? 0}`,
    mentionType: candidate.mentionType,
    name: candidate.name,
    size: 0,
    path: candidate.path,
    symbolName: candidate.symbolName,
    line: candidate.line,
    column: candidate.column,
  };
}

export function createComposerSelectionAttachment(selection: {
  documentId: string;
  name: string;
  path: string;
  text: string;
  startLine: number;
  endLine: number;
  startColumn: number;
  endColumn: number;
  languageId: string;
}): ComposerSelectionAttachment {
  return {
    kind: "selection",
    id: `selection:${selection.documentId}:${selection.startLine}:${selection.startColumn}:${selection.endLine}:${selection.endColumn}`,
    documentId: selection.documentId,
    name: selection.name,
    path: selection.path,
    size: selection.text.length,
    startLine: selection.startLine,
    endLine: selection.endLine,
    startColumn: selection.startColumn,
    endColumn: selection.endColumn,
    text: selection.text,
    languageId: selection.languageId,
  };
}

const imageMimePrefix = "image/";

export function isComposerImageFile(file: File) {
  return isVisionImageFile(file);
}

export function createComposerFileAttachment(file: File): ComposerFileAttachment {
  const isImage = isComposerImageFile(file);
  return {
    kind: "file",
    file,
    id: attachmentId(file),
    name: file.name,
    size: file.size,
    isImage,
    previewUrl: isImage ? URL.createObjectURL(file) : undefined,
  };
}

export function normalizePastedFile(file: File, index: number) {
  if (file.name.trim()) return file;
  if (!file.type.startsWith(imageMimePrefix)) return file;
  const extension = extensionFromMime(file.type) ?? "png";
  return new File([file], `capture-${Date.now()}-${index + 1}.${extension}`, { type: file.type });
}

export function collectClipboardFiles(data: DataTransfer) {
  const direct = Array.from(data.files);
  if (direct.length > 0) return direct.map((file, index) => normalizePastedFile(file, index));
  return Array.from(data.items)
    .filter((item) => item.kind === "file")
    .map((item) => item.getAsFile())
    .filter((file): file is File => file !== null)
    .map((file, index) => normalizePastedFile(file, index));
}

export function revokeComposerAttachmentPreview(attachment: ComposerAttachment) {
  if (attachment.kind !== "file" || !attachment.previewUrl) return;
  URL.revokeObjectURL(attachment.previewUrl);
}

export function revokeComposerAttachmentPreviews(attachments: readonly ComposerAttachment[]) {
  for (const attachment of attachments) revokeComposerAttachmentPreview(attachment);
}

export async function buildMessageDisplayAttachments(
  attachments: readonly ComposerAttachment[],
): Promise<AiChatMessageAttachment[]> {
  const items: AiChatMessageAttachment[] = [];
  for (const attachment of attachments) {
    if (attachment.kind === "editor" || attachment.kind === "mention" || attachment.kind === "selection") {
      items.push({
        id: attachment.id,
        kind: "file",
        name: attachment.name,
        size: attachment.size,
      });
      continue;
    }
    if (attachment.isImage) {
      const previewUrl =
        attachment.previewUrl?.startsWith("data:")
          ? attachment.previewUrl
          : await readFileAsDataUrl(attachment.file, maxMessageImagePreviewBytes);
      if (previewUrl) {
        items.push({
          id: attachment.id,
          kind: "image",
          name: attachment.name,
          previewUrl,
          size: attachment.size,
        });
      } else {
        items.push({
          id: attachment.id,
          kind: "file",
          name: attachment.name,
          size: attachment.size,
        });
      }
      continue;
    }
    items.push({
      id: attachment.id,
      kind: "file",
      name: attachment.name,
      size: attachment.size,
    });
  }
  return items;
}

function attachmentId(file: File) {
  return `${file.name}:${file.size}:${file.lastModified}`;
}

function extensionFromMime(mime: string) {
  const map: Record<string, string> = {
    "image/png": "png",
    "image/jpeg": "jpg",
    "image/jpg": "jpg",
    "image/gif": "gif",
    "image/webp": "webp",
    "image/bmp": "bmp",
    "image/avif": "avif",
    "image/heic": "heic",
    "image/heif": "heif",
    "image/svg+xml": "svg",
  };
  return map[mime.toLowerCase()];
}