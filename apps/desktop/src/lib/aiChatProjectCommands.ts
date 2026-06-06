import { luxCommands } from "./tauri";

const commandPathPattern = /(?:^|\/)\.lux\/commands\/([^/]+)\.md$/i;
const maxProjectCommands = 16;
const maxCommandFileBytes = 12_000;
const maxTemplateChars = 4_000;
const maxDescriptionChars = 160;

export type ProjectSlashCommand = {
  kind: "project";
  id: string;
  name: string;
  description: string;
  template: string;
  sourcePath: string;
  keywords: string[];
};

const frontMatterPattern = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/;

export function parseProjectCommandMarkdown(fileName: string, raw: string): ProjectSlashCommand | null {
  const name = sanitizeCommandName(fileName.replace(/\.md$/i, ""));
  if (!name) return null;

  const trimmed = raw.trim();
  if (!trimmed) return null;

  const matched = trimmed.match(frontMatterPattern);
  const metaBlock = matched?.[1] ?? "";
  const body = (matched?.[2] ?? trimmed).trim();
  const description = extractDescription(metaBlock) || truncateText(body.split("\n").find((line) => line.trim()) ?? name, maxDescriptionChars);
  const template = truncateText(body, maxTemplateChars);
  if (!template) return null;

  return {
    kind: "project",
    id: `project:${name}`,
    name,
    description: truncateText(description, maxDescriptionChars),
    template,
    sourcePath: fileName,
    keywords: [name, description.toLowerCase()],
  };
}

function extractDescription(metaBlock: string) {
  for (const line of metaBlock.split("\n")) {
    const match = line.match(/^\s*description\s*:\s*(.+)\s*$/i);
    if (match?.[1]) return match[1].trim().replace(/^["']|["']$/g, "");
  }
  return "";
}

function sanitizeCommandName(value: string) {
  const normalized = value.trim().toLowerCase().replace(/[^a-z0-9-]+/g, "-").replace(/^-+|-+$/g, "");
  if (!normalized || normalized.length > 32) return null;
  return normalized;
}

function truncateText(value: string, max: number) {
  const trimmed = value.trim();
  if (trimmed.length <= max) return trimmed;
  return `${trimmed.slice(0, max).trimEnd()}...`;
}

function isReservedSlashName(name: string) {
  return new Set([
    "compact",
    "clear",
    "undo",
    "help",
    "goal",
    "model",
    "agent",
    "settings",
    "index",
  ]).has(name);
}

export async function loadProjectSlashCommands(workspaceRoot: string | null | undefined): Promise<ProjectSlashCommand[]> {
  if (!workspaceRoot) return [];
  try {
    const entries = await luxCommands.fsListFiles(4_000);
    const paths = entries
      .filter((entry) => entry.kind === "file")
      .map((entry) => entry.path.replace(/\\/g, "/"))
      .filter((path) => commandPathPattern.test(path))
      .slice(0, maxProjectCommands * 2);

    const commands: ProjectSlashCommand[] = [];
    for (const path of paths) {
      if (commands.length >= maxProjectCommands) break;
      try {
        const file = await luxCommands.fsReadText(path, maxCommandFileBytes);
        const baseName = path.match(commandPathPattern)?.[1] ?? path;
        const parsed = parseProjectCommandMarkdown(baseName, file.text);
        if (!parsed || isReservedSlashName(parsed.name)) continue;
        commands.push({ ...parsed, sourcePath: path });
      } catch {
        // skip unreadable command file
      }
    }
    return commands.sort((left, right) => left.name.localeCompare(right.name));
  } catch {
    return [];
  }
}