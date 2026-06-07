import { buildPathAttachmentContext } from "./aiFileContext";
import type { ComposerAttachment, ComposerMentionAttachment } from "./aiChatComposerAttachments";
import type { AiChatAttachmentInput, AiChatMentionHints } from "./aiChatTypes";
import { luxCommands } from "./tauri";
import { readEditorDocumentAttachment } from "./aiChatDocumentAttachment";
import { truncateText } from "./aiRuntimeShared";
import type { VisionImageFormat } from "./aiVisionFormat";
import type { DocumentSnapshot } from "./types";

export function collectMentionHints(attachments: readonly ComposerAttachment[]): AiChatMentionHints {
  const hints: AiChatMentionHints = {};
  for (const attachment of attachments) {
    if (attachment.kind !== "mention") continue;
    if (attachment.mentionType === "codebase") hints.codebase = true;
    if (attachment.mentionType === "docs") hints.docs = true;
  }
  return hints;
}

export async function buildMentionRuntimeAttachments(
  attachments: readonly ComposerAttachment[],
  openDocuments: DocumentSnapshot[],
  options: {
    includeVisionImage?: boolean;
    visionImageFormat?: VisionImageFormat;
    includeMediaContext?: boolean;
    localSttCommand?: string;
    localSttModelPath?: string;
    voiceInputLanguage?: string;
  },
): Promise<AiChatAttachmentInput[]> {
  const mentionAttachments = attachments.filter((attachment): attachment is ComposerMentionAttachment => attachment.kind === "mention");
  const results: AiChatAttachmentInput[] = [];
  for (const mention of mentionAttachments) {
    results.push(await resolveMentionAttachment(mention, openDocuments, options));
  }
  return results;
}

async function resolveMentionAttachment(
  mention: ComposerMentionAttachment,
  openDocuments: DocumentSnapshot[],
  options: Parameters<typeof buildMentionRuntimeAttachments>[2],
): Promise<AiChatAttachmentInput> {
  switch (mention.mentionType) {
    case "codebase":
      return {
        name: mention.name,
        size: 0,
        text: [
          "Mention: @codebase",
          "The user pinned the whole workspace for this turn.",
          "Start with SemanticSearch, FastContext, or RepoMap before broad edits.",
        ].join("\n"),
      };
    case "docs":
      return {
        name: mention.name,
        size: 0,
        text: [
          "Mention: @docs",
          "The user pinned project documentation and rules for this turn.",
          "Read RulesContext and DocsContext before changing code.",
        ].join("\n"),
      };
    case "file": {
      if (!mention.path) {
        return { name: mention.name, size: 0, text: "Mention: @file (path missing)" };
      }
      const open = openDocuments.find((document) => document.path === mention.path);
      if (open) return readEditorDocumentAttachment(open, options);
      const context = await buildPathAttachmentContext(mention.path, `Mentioned file: ${mention.path}`, options);
      return { name: mention.name, size: context.size, text: context.text, visionImageUrl: context.visionImageUrl, visionFrameUrls: context.visionFrameUrls };
    }
    case "folder": {
      if (!mention.path) {
        return { name: mention.name, size: 0, text: "Mention: @folder (path missing)" };
      }
      const entries = await luxCommands.fsReadDir(mention.path);
      const lines = entries.slice(0, 80).map((entry) => `${entry.kind === "directory" ? "dir" : "file"} ${entry.path}`);
      return {
        name: mention.name,
        size: lines.join("\n").length,
        text: truncateText([
          `Mentioned folder: ${mention.path}`,
          `entries=${entries.length}`,
          "",
          ...lines,
          entries.length > 80 ? `... ${entries.length - 80} more entries` : "",
        ].filter(Boolean).join("\n"), 12_000),
      };
    }
    case "symbol": {
      if (!mention.path || !mention.symbolName) {
        return { name: mention.name, size: 0, text: "Mention: @symbol (incomplete)" };
      }
      const response = await luxCommands.aiSymbolContext(
        mention.symbolName,
        mention.path,
        mention.line ?? null,
        mention.column ?? null,
        12,
      );
      return {
        name: mention.name,
        size: JSON.stringify(response).length,
        text: truncateText([
          `Mentioned symbol: ${mention.symbolName}`,
          `path=${mention.path}`,
          mention.line ? `line=${mention.line}` : "",
          "",
          JSON.stringify(response, null, 2),
        ].filter(Boolean).join("\n"), 16_000),
      };
    }
    default:
      return { name: mention.name, size: 0, text: mention.name };
  }
}