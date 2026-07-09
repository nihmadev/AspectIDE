export type FileDiffHunk = {
  id: string;
  kind: "change" | "insert" | "delete";
  beforeStartLine: number;
  beforeLineCount: number;
  afterStartLine: number;
  afterLineCount: number;
  beforeLines: string[];
  afterLines: string[];
};

export function buildFileDiffHunks(beforeText: string, afterText: string): FileDiffHunk[] {
  const beforeLines = splitLines(beforeText);
  const afterLines = splitLines(afterText);
  const hunks: FileDiffHunk[] = [];
  let index = 0;
  let hunkIndex = 0;

  while (index < Math.max(beforeLines.length, afterLines.length)) {
    const beforeLine = beforeLines[index];
    const afterLine = afterLines[index];
    if (beforeLine === afterLine) {
      index += 1;
      continue;
    }

    const start = index;
    const beforeChunk: string[] = [];
    const afterChunk: string[] = [];
    while (index < Math.max(beforeLines.length, afterLines.length)) {
      const left = beforeLines[index];
      const right = afterLines[index];
      if (left === right && beforeChunk.length > 0 && afterChunk.length > 0) break;
      if (left !== undefined && left !== right) beforeChunk.push(left);
      if (right !== undefined && left !== right) afterChunk.push(right);
      if (left === undefined && right === undefined) break;
      index += 1;
      if (beforeChunk.length >= 240 || afterChunk.length >= 240) break;
    }

    const kind: FileDiffHunk["kind"] = beforeChunk.length === 0
      ? "insert"
      : afterChunk.length === 0
        ? "delete"
        : "change";

    hunks.push({
      id: `hunk-${hunkIndex += 1}`,
      kind,
      beforeStartLine: start + 1,
      beforeLineCount: beforeChunk.length,
      afterStartLine: start + 1,
      afterLineCount: afterChunk.length,
      beforeLines: beforeChunk,
      afterLines: afterChunk,
    });
  }

  return hunks;
}

export function applyAcceptedHunks(beforeText: string, afterText: string, acceptedHunkIds: ReadonlySet<string>): string {
  const hunks = buildFileDiffHunks(beforeText, afterText);
  if (acceptedHunkIds.size === 0) return beforeText;
  if (hunks.every((hunk) => acceptedHunkIds.has(hunk.id))) return afterText;

  const beforeLines = splitLines(beforeText);
  const output: string[] = [];
  let lineIndex = 0;

  for (const hunk of hunks) {
    while (lineIndex < hunk.beforeStartLine - 1) {
      output.push(beforeLines[lineIndex] ?? "");
      lineIndex += 1;
    }
    if (acceptedHunkIds.has(hunk.id)) {
      output.push(...hunk.afterLines);
      lineIndex += hunk.beforeLineCount;
    } else {
      output.push(...hunk.beforeLines);
      lineIndex += hunk.beforeLineCount;
    }
  }

  while (lineIndex < beforeLines.length) {
    output.push(beforeLines[lineIndex] ?? "");
    lineIndex += 1;
  }

  return joinLines(output, beforeText, afterText);
}

function splitLines(text: string) {
  if (!text) return [""];
  const parts = text.split(/\r?\n/);
  if (text.endsWith("\n")) parts.push("");
  return parts;
}

function joinLines(lines: string[], beforeText: string, afterText: string) {
  const newline = beforeText.includes("\r\n") || afterText.includes("\r\n") ? "\r\n" : "\n";
  return lines.join(newline);
}