// Output normalization — defends against runaway repetition in a model's final
// answer (a "flood": the same paragraph, block, or line emitted many times). This
// is a safety net, not a style filter: it only collapses near-verbatim repetition
// that is almost certainly a degeneration artifact, never normal prose.

/** Minimum trimmed length for a block to be considered for de-duplication. */
const MIN_BLOCK_CHARS = 120;
/** Keep at most this many copies of an identical block. */
const MAX_BLOCK_REPEATS = 2;
/** Minimum trimmed length for a single line to count toward a line flood. */
const MIN_LINE_CHARS = 24;
/** Consecutive identical lines beyond this are collapsed. */
const MAX_CONSECUTIVE_LINES = 2;

export const OUTPUT_TRIM_MARKER = "[Lux · trimmed repeated output]";

export type OutputNormalizationResult = {
  text: string;
  /** True when repetition was collapsed (caller may nudge the model). */
  collapsed: boolean;
  /** Number of repeated units removed. */
  removed: number;
};

/**
 * Collapse runaway repetition in an assistant message. Returns the original text
 * unchanged when nothing looks like a flood.
 */
export function normalizeRepeatedOutput(content: string): OutputNormalizationResult {
  if (!content || content.length < MIN_BLOCK_CHARS * 2) {
    return { text: content, collapsed: false, removed: 0 };
  }

  let removed = 0;

  // Pass 1 — collapse blocks (paragraphs / fenced sections) separated by blank lines.
  const blocks = content.split(/\n{2,}/);
  if (blocks.length >= 3) {
    const counts = new Map<string, number>();
    const kept: string[] = [];
    for (const block of blocks) {
      const key = block.trim();
      if (key.length >= MIN_BLOCK_CHARS) {
        const next = (counts.get(key) ?? 0) + 1;
        counts.set(key, next);
        if (next > MAX_BLOCK_REPEATS) {
          removed += 1;
          continue;
        }
      }
      kept.push(block);
    }
    content = kept.join("\n\n");
  }

  // Pass 2 — collapse long runs of identical consecutive lines (single-line floods).
  const lines = content.split("\n");
  const dedupedLines: string[] = [];
  let runKey = "";
  let runCount = 0;
  for (const line of lines) {
    const key = line.trim();
    if (key.length >= MIN_LINE_CHARS && key === runKey) {
      runCount += 1;
      if (runCount > MAX_CONSECUTIVE_LINES) {
        removed += 1;
        continue;
      }
    } else {
      runKey = key.length >= MIN_LINE_CHARS ? key : "";
      runCount = key === runKey ? 1 : 0;
    }
    dedupedLines.push(line);
  }
  content = dedupedLines.join("\n");

  if (removed > 0) {
    content = `${content.trimEnd()}\n\n${OUTPUT_TRIM_MARKER}`;
  }
  return { text: content, collapsed: removed > 0, removed };
}
