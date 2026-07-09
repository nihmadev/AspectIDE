const frontMatterPattern = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/;

function sanitizeCommandName(value) {
  const normalized = value.trim().toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
  if (!normalized || normalized.length > 32) return null;
  return normalized;
}

function parseProjectCommandMarkdown(fileName, raw) {
  const name = sanitizeCommandName(fileName.replace(/\.md$/i, ""));
  if (!name) return null;
  const trimmed = raw.trim();
  const matched = trimmed.match(frontMatterPattern);
  const metaBlock = matched?.[1] ?? "";
  const body = (matched?.[2] ?? trimmed).trim();
  const descMatch = metaBlock.match(/^\s*description\s*:\s*(.+)\s*$/im);
  const description = descMatch?.[1]?.trim().replace(/^["']|["']$/g, "") || name;
  return { name, description, template: body };
}

const parsed = parseProjectCommandMarkdown("review", `---
description: Review recent changes
---
Review the diff and list findings by severity.`);
if (parsed?.name !== "review" || !parsed.template.includes("findings")) {
  console.error("parse failed", parsed);
  process.exit(1);
}

const commandPathPattern = /(?:^|\/)\.aspect\/commands\/([^/]+)\.md$/i;
if (!commandPathPattern.test("proj/.aspect/commands/review.md")) {
  console.error("path pattern failed");
  process.exit(1);
}

console.log("project commands verification passed");