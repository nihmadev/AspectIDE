import {
  isVisionImageFile,
  maxMessageImagePreviewBytes,
  readFileAsDataUrl,
} from "./../utils/file-context";
import type { AiChatMessageAttachment } from "./types";
import { luxCommands } from "./../../tauri/commands";
import { isTauriRuntime } from "./../../tauri/runtime";

async function dataUrlToFile(dataUrl: string, name: string, mime: string): Promise<File> {
  const response = await fetch(dataUrl);
  const blob = await response.blob();
  return new File([blob], name, { type: mime || blob.type || "application/octet-stream" });
}

/**
 * Read an image from the OS clipboard as a PNG File (paste fallback for Linux
 * WebKitGTK, where the webview `ClipboardEvent` carries no image). Returns
 * `null` when there is no image or the native read is unavailable. The native
 * command yields raw RGBA; we encode to PNG via an offscreen canvas.
 */
export async function readClipboardImageFile(): Promise<File | null> {
  if (!isTauriRuntime()) return null;
  try {
    const image = await luxCommands.clipboardReadImage();
    if (!image || image.width <= 0 || image.height <= 0) return null;
    const binary = atob(image.rgbaBase64);
    const rgba = new Uint8ClampedArray(binary.length);
    for (let i = 0; i < binary.length; i += 1) rgba[i] = binary.charCodeAt(i);
    if (rgba.length !== image.width * image.height * 4) return null;

    const canvas = document.createElement("canvas");
    canvas.width = image.width;
    canvas.height = image.height;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    ctx.putImageData(new ImageData(rgba, image.width, image.height), 0, 0);
    const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
    if (!blob) return null;
    return new File([blob], `pasted-${Date.now()}.png`, { type: "image/png" });
  } catch {
    return null;
  }
}

/**
 * Open the native OS multi-file picker (desktop only) and read the selections
 * into File objects. Returns `null` when the native dialog is unavailable or
 * fails (caller falls back to the HTML `<input type=file>`), or `[]` when the
 * user cancels the dialog.
 */
export async function pickNativeAttachmentFiles(): Promise<File[] | null> {
  if (!isTauriRuntime()) return null;
  try {
    const paths = await luxCommands.pickAttachmentFiles();
    if (paths.length === 0) return [];
    return await Promise.all(
      paths.map(async (path) => {
        const asset = await luxCommands.readExternalFile(path);
        const name = path.split(/[\\/]/).pop() || "attachment";
        return dataUrlToFile(asset.dataUrl, name, asset.mimeType);
      }),
    );
  } catch {
    return null;
  }
}

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
      // A rejected read (backing file deleted/moved after attach) must not throw
      // the whole send away — degrade to the same name-only card an oversized
      // image gets. This runs BEFORE the send's try block, so a throw here would
      // silently drop the message and leak the pending-send lock.
      const previewUrl =
        attachment.previewUrl?.startsWith("data:")
          ? attachment.previewUrl
          : await readFileAsDataUrl(attachment.file, maxMessageImagePreviewBytes).catch(() => undefined);
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