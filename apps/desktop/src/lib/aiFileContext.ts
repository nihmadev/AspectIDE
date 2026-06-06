import { isMediaPreviewDocument, isStructuredPreviewDocument } from "./documentViewRouting";
import {
  isImagePreviewDocument,
  isMonacoTextDocument,
  isPdfPreviewDocument,
  isDatabaseEditorDocument,
  isSpreadsheetEditorDocument,
  isTableEditorDocument,
} from "./documents";
import { parseSpreadsheetDocument } from "./spreadsheetDocument";
import { truncateText } from "./aiRuntimeShared";
import { isTauriRuntime, luxCommands } from "./tauri";
import type { DocumentSnapshot, FileInspection, FileInspectionOptions, FileViewStrategy } from "./types";

export const defaultAttachmentInspectOptions: FileInspectionOptions = {
  maxTextBytes: 64_000,
  maxRows: 100,
  maxColumns: 32,
  maxArchiveEntries: 160,
};

const maxAttachmentChars = 18_000;
export const maxVisionImageBytes = 4 * 1024 * 1024;
/** Stored inline on chat messages for thumbnail/history display. */
export const maxMessageImagePreviewBytes = 2 * 1024 * 1024;
const visionImageExtensions = new Set(["png", "jpg", "jpeg", "jpe", "webp", "gif", "bmp", "avif", "heif", "heic"]);

const inspectByStrategy = new Set<FileViewStrategy>([
  "spreadsheetEditor",
  "spreadsheetPreview",
  "tablePreview",
  "tableEditor",
  "databaseEditor",
  "databasePreview",
  "diagramPreview",
  "pdfPreview",
  "officePreview",
  "archivePreview",
  "notebookPreview",
  "imagePreview",
  "audioPreview",
  "videoPreview",
  "binaryPreview",
  "externalOnly",
]);

export function fileExtensionLower(path: string) {
  const normalized = path.replace(/\\/g, "/");
  const index = normalized.lastIndexOf(".");
  return index === -1 ? "" : normalized.slice(index + 1).toLowerCase();
}

/** True when Lux should assemble AI context via file_inspect instead of raw editor/disk text. */
export function shouldInspectPathForAi(path: string, strategy?: FileViewStrategy) {
  const extension = fileExtensionLower(path);
  if (extension === "ipynb") return true;
  if (strategy && inspectByStrategy.has(strategy)) return true;
  if (matchesExtensionGroup(extension, ["csv", "tsv", "psv"])) return true;
  return false;
}

export function shouldInspectDocumentForAi(document: Pick<DocumentSnapshot, "path" | "view">) {
  if (!document.path) return false;
  if (shouldInspectPathForAi(document.path, document.view.strategy)) return true;
  if (isSpreadsheetEditorDocument(document) || isTableEditorDocument(document) || isDatabaseEditorDocument(document)) {
    return true;
  }
  if (isImagePreviewDocument(document) || isPdfPreviewDocument(document) || isMediaPreviewDocument(document)) {
    return true;
  }
  if (isStructuredPreviewDocument(document)) return true;
  if (document.view.binary || document.view.mode === "preview" || document.view.mode === "external") return true;
  return !isMonacoTextDocument(document);
}

export function isVisionImagePath(path: string) {
  return visionImageExtensions.has(fileExtensionLower(path));
}

export function isVisionImageFile(file: Pick<File, "name" | "type">) {
  if (file.type.startsWith("image/")) return true;
  return isVisionImagePath(file.name);
}

export function imageAttachmentText(
  name: string,
  size: number,
  options: { visionAttached: boolean; note?: string },
) {
  const lines = [
    `Attached image: ${name}`,
    `size=${size} bytes`,
    options.visionAttached
      ? "The image bytes are attached to this request as vision input for the model."
      : "Vision input is disabled or unavailable for this image; do not infer pixels from metadata alone.",
  ];
  if (options.note) lines.push(options.note);
  return lines.join("\n");
}

export function readFileAsDataUrl(file: File, maxBytes: number): Promise<string | undefined> {
  if (file.size > maxBytes) return Promise.resolve(undefined);
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(typeof reader.result === "string" ? reader.result : undefined);
    reader.onerror = () => reject(reader.error ?? new Error("Failed to read file"));
    reader.readAsDataURL(file);
  });
}

const audioExtensions = new Set(["mp3", "wav", "flac", "ogg", "oga", "m4a", "aac", "opus", "wma", "aiff", "aif", "mid", "midi"]);
const videoExtensions = new Set(["mp4", "m4v", "webm", "mov", "mkv", "avi", "wmv", "mpeg", "mpg", "3gp", "ogv"]);

export function isAudioPath(path: string) {
  return audioExtensions.has(fileExtensionLower(path));
}

export function isVideoPath(path: string) {
  return videoExtensions.has(fileExtensionLower(path));
}

export function isMediaPath(path: string) {
  return isAudioPath(path) || isVideoPath(path);
}

export async function buildPathAttachmentContext(
  path: string,
  label: string,
  options: {
    inspect?: FileInspectionOptions;
    includeVisionImage?: boolean;
    includeMediaContext?: boolean;
    localSttCommand?: string;
    localSttModelPath?: string;
    voiceInputLanguage?: string;
    editorSupplement?: string;
  } = {},
): Promise<{ text: string; size: number; visionImageUrl?: string; visionFrameUrls?: string[] }> {
  const inspectOptions = options.inspect ?? defaultAttachmentInspectOptions;
  let size = 0;
  let visionImageUrl: string | undefined;

  if (options.includeVisionImage && isTauriRuntime() && isVisionImagePath(path)) {
    try {
      const asset = await luxCommands.fileAssetData(path);
      size = Number(asset.size);
      if (asset.size <= maxVisionImageBytes && asset.dataUrl.startsWith("data:image/")) {
        visionImageUrl = asset.dataUrl;
      }
    } catch {
      // Vision is optional; structured inspect still applies.
    }
  }

  let visionFrameUrls: string[] | undefined;
  let mediaSections: string[] = [];
  if (options.includeMediaContext && isTauriRuntime() && isMediaPath(path)) {
    try {
      const media = await luxCommands.fileMediaAiContext({
        path,
        sttCommand: options.localSttCommand?.trim() || null,
        sttModelPath: options.localSttModelPath?.trim() || null,
        language: options.voiceInputLanguage?.trim() || null,
        maxFrames: 4,
      });
      if (media.transcript?.trim()) {
        mediaSections.push(`Transcript:\n${media.transcript.trim()}`);
      }
      if (media.notes.length > 0) {
        mediaSections.push(media.notes.join("\n"));
      }
      if (media.frameDataUrls.length > 0) {
        visionFrameUrls = media.frameDataUrls;
        mediaSections.push(`Video frame snapshots attached: ${media.frameDataUrls.length}`);
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      mediaSections.push(`Media enrichment failed: ${message}`);
    }
  }

  try {
    const inspection = await luxCommands.fileInspect(path, inspectOptions);
    size = Math.max(size, Number(inspection.metadata.size));
    const supplement = options.editorSupplement?.trim();
    const body = [
      formatInspectionAttachment(label, inspection, supplement),
      ...mediaSections,
    ].join("\n\n");
    const text = truncateText(body, maxAttachmentChars);
    return { text, size, visionImageUrl, visionFrameUrls };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return {
      text: truncateText(`${label}\nStructured inspect failed: ${message}`, maxAttachmentChars),
      size,
      visionImageUrl,
      visionFrameUrls,
    };
  }
}

export function formatInspectionAttachment(label: string, inspection: FileInspection, supplement?: string) {
  const sections = [
    label,
    `displayName=${inspection.descriptor.displayName}`,
    `category=${inspection.descriptor.category}`,
    `strategy=${inspection.descriptor.strategy}`,
    `mode=${inspection.descriptor.mode}`,
    `mime=${inspection.descriptor.mimeType ?? "unknown"}`,
    `aiReadable=${inspection.descriptor.aiReadable}`,
    `previewKind=${inspection.preview.kind}`,
    inspection.truncated ? "truncated=true" : "truncated=false",
    ...(inspection.warnings.length > 0 ? [`warnings:\n${inspection.warnings.join("\n")}`] : []),
    "",
    inspection.aiContext,
  ];
  if (supplement) {
    sections.push("", "Editor state (may include unsaved changes):", supplement);
  }
  return sections.join("\n");
}

export function spreadsheetEditorSupplement(document: Pick<DocumentSnapshot, "text" | "is_dirty">) {
  if (!document.is_dirty) return "";
  const parsed = parseSpreadsheetDocument(document.text);
  if (!parsed) return "dirty=true (workbook JSON could not be parsed)";
  const lines = [
    `dirty=true`,
    `workbookType=${parsed.workbookType}`,
    `truncated=${parsed.truncated}`,
    `sheetCount=${parsed.sheets.length}`,
  ];
  for (const sheet of parsed.sheets.slice(0, 4)) {
    const rowPreview = sheet.rows.slice(0, 12).map((row) => row.join(" | ")).join("\n");
    lines.push(`Sheet ${sheet.name || "(unnamed)"}:\n${rowPreview}`);
  }
  return truncateText(lines.join("\n\n"), 6_000);
}

function matchesExtensionGroup(extension: string, values: string[]) {
  return values.includes(extension);
}