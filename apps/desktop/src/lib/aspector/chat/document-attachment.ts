import { documentDisplayPath, documentTitle, isMonacoTextDocument, isSpreadsheetEditorDocument } from "./../../editor/documents/documents";
import type { AiChatAttachmentInput } from "./types";
import {
  buildPathAttachmentContext,
  shouldInspectDocumentForAi,
  spreadsheetEditorSupplement,
} from "./../utils/file-context";
import { truncateText } from "./../runtime/shared";
import type { VisionImageFormat } from "./../utils/vision-format";
import type { DocumentSnapshot } from "./types";

const maxAttachmentChars = 18_000;
const selectionSurroundingLines = 8;

export async function readSelectionAttachment(
  attachment: {
    name: string;
    path: string;
    text: string;
    startLine: number;
    endLine: number;
    languageId: string;
  },
  openDocuments: DocumentSnapshot[],
): Promise<AiChatAttachmentInput> {
  const document = openDocuments.find((entry) => entry.path === attachment.path);
  const surrounding = document?.text
    ? extractSurroundingContext(document.text, attachment.startLine, attachment.endLine, selectionSurroundingLines)
    : "";
  return {
    name: attachment.name,
    size: attachment.text.length,
    text: truncateText([
      `Selected code: ${attachment.path}`,
      `lines=${attachment.startLine}-${attachment.endLine}`,
      `language=${attachment.languageId}`,
      "",
      "```",
      attachment.text,
      "```",
      surrounding ? `\nSurrounding context:\n${surrounding}` : "",
    ].filter(Boolean).join("\n"), maxAttachmentChars),
  };
}

function extractSurroundingContext(text: string, startLine: number, endLine: number, radius: number) {
  const lines = text.split(/\r?\n/);
  const start = Math.max(0, startLine - 1 - radius);
  const end = Math.min(lines.length, endLine + radius);
  return lines.slice(start, end).map((line, index) => `${start + index + 1} | ${line}`).join("\n");
}

export async function readEditorDocumentAttachment(
  document: DocumentSnapshot,
  options: {
    includeVisionImage?: boolean;
    visionImageFormat?: VisionImageFormat;
    includeMediaContext?: boolean;
    localSttCommand?: string;
    localSttModelPath?: string;
    voiceInputLanguage?: string;
  } = {},
): Promise<AiChatAttachmentInput> {
  const name = documentTitle(document);
  const path = document.path ? documentDisplayPath(document) : document.title;
  const label = `Pinned editor tab: ${path}`;

  if (!document.path) {
    return {
      name,
      size: document.text.length,
      text: truncateText(
        [label, `language=${document.language_id}`, "", document.text].join("\n"),
        maxAttachmentChars,
      ),
    };
  }

  if (shouldInspectDocumentForAi(document)) {
    const supplement = isSpreadsheetEditorDocument(document)
      ? spreadsheetEditorSupplement(document)
      : document.is_dirty && isMonacoTextDocument(document)
        ? truncateText(document.text, 8_000)
        : "";
    const context = await buildPathAttachmentContext(document.path, label, {
      includeVisionImage: options.includeVisionImage,
      visionImageFormat: options.visionImageFormat,
      includeMediaContext: options.includeMediaContext,
      localSttCommand: options.localSttCommand,
      localSttModelPath: options.localSttModelPath,
      voiceInputLanguage: options.voiceInputLanguage,
      editorSupplement: supplement || undefined,
    });
    return {
      name,
      size: context.size,
      text: context.text,
      visionImageUrl: context.visionImageUrl,
      visionFrameUrls: context.visionFrameUrls,
    };
  }

  return {
    name,
    size: document.text.length,
    text: truncateText(
      [
        label,
        `language=${document.language_id}`,
        `dirty=${document.is_dirty}`,
        "",
        document.text,
      ].join("\n"),
      maxAttachmentChars,
    ),
  };
}