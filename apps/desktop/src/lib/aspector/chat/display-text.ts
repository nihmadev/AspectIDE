/** Keeps this module linked in split chunks (not tree-shaken away from chat UI). */
export const AI_CHAT_DISPLAY_TEXT_MODULE = "aiChatDisplayText.v1";

const namedHtmlEntities: Record<string, string> = {
  quot: '"',
  amp: "&",
  lt: "<",
  gt: ">",
  apos: "'",
  nbsp: "\u00a0",
  ldquo: "\u201C",
  rdquo: "\u201D",
  lsquo: "\u2018",
  rsquo: "\u2019",
};

const fullwidthAmpersandPattern = /\uFF06/g;
const htmlEntityPattern = /&(?:#x([0-9a-f]+)|#(\d+)|([a-z]+));/gi;
const htmlEntityNoSemicolonPattern = /&(?:quot|amp|lt|gt|apos|nbsp)(?![a-z0-9;])/gi;

/** Internal goal orchestration markers — detected by runtime, never shown in chat UI. */
const goalOrchestrationMarkerLine = /^\s*(?:\[goal:(?:complete|blocked)\]|goal:(?:complete|blocked))\s*$/gim;

/** Strip silent goal markers before rendering assistant text. */
export function stripGoalOrchestrationMarkers(text: string): string {
  if (!text) return text;
  // NOTE: must NOT trim trailing whitespace here. decodeChatDisplayText runs this on
  // every individual inline markdown token (see AiChatMessages.textFromToken); trimming
  // would eat the space between a word and an adjacent **bold**/`code`/[link] span,
  // gluing them together ("через **PowerShell**" → "черезPowerShell"). The cosmetic
  // end-of-message trim lives at the message level (trimChatMessageEnd).
  return text.replace(goalOrchestrationMarkerLine, "").replace(/\n{3,}/g, "\n\n");
}

/** Trim trailing whitespace from a WHOLE assistant message (never a single token). */
export function trimChatMessageEnd(text: string): string {
  return text.replace(/\s+$/, "");
}

/** Decode `&quot;`, `&#34;`, etc. from model/provider text for safe React text rendering. */
export function decodeChatDisplayText(text: string | null | undefined): string {
  if (typeof text !== "string" || text.length === 0) return typeof text === "string" ? text : "";
  let decoded = text;
  if (text.includes("&") || text.includes("\uFF06")) {
    decoded = text.replace(fullwidthAmpersandPattern, "&");
    for (let pass = 0; pass < 4 && decoded.includes("&"); pass += 1) {
      const next = decodeHtmlEntitiesOnce(decoded);
      if (next === decoded) break;
      decoded = next;
    }
    if (typeof document !== "undefined" && /&(?:#x?[0-9a-f]+|#\d+|[a-z]+)/i.test(decoded)) {
      try {
        decoded = decodeWithTextarea(decoded);
      } catch {
        // keep partially decoded text
      }
    }
  }
  return stripGoalOrchestrationMarkers(decoded);
}

/** Coerce persisted or streamed message fields before markdown/entity decoding. */
export function coerceChatMessageText(value: unknown): string {
  return typeof value === "string" ? value : value == null ? "" : String(value);
}

function decodeHtmlEntitiesOnce(text: string) {
  htmlEntityPattern.lastIndex = 0;
  htmlEntityNoSemicolonPattern.lastIndex = 0;
  const withSemicolon = text.replace(htmlEntityPattern, (entity, hex, decimal, name) => {
    if (hex) return codePointToString(Number.parseInt(hex, 16), entity);
    if (decimal) return codePointToString(Number.parseInt(decimal, 10), entity);
    if (name) return namedHtmlEntities[name.toLowerCase()] ?? entity;
    return entity;
  });
  htmlEntityNoSemicolonPattern.lastIndex = 0;
  return withSemicolon.replace(htmlEntityNoSemicolonPattern, (entity) => {
    const name = entity.slice(1).toLowerCase();
    return namedHtmlEntities[name] ?? entity;
  });
}

function decodeWithTextarea(text: string) {
  const textarea = document.createElement("textarea");
  textarea.innerHTML = text;
  return textarea.value;
}

function codePointToString(codePoint: number, fallback: string) {
  if (!Number.isFinite(codePoint) || codePoint < 0) return fallback;
  try {
    return String.fromCodePoint(codePoint);
  } catch {
    return fallback;
  }
}