import type { AiChatSendInput } from "./aiChatTypes";
import {
  agentsFileCandidates,
  isPathInsideWorkspace,
  relativeDirectoryLabel,
  resolveAgentsStartDirectory,
  walkUpAgentsDirectories,
} from "./aiProjectAgentsWalkUp";
import { truncateText } from "./aiRuntimeShared";
import { luxCommands } from "./tauri";

export { walkUpAgentsDirectories } from "./aiProjectAgentsWalkUp";

const maxTotalSnipChars = 3_600;
const maxRootFileChars = 1_400;
const maxNestedFileChars = 900;

export type ProjectAgentsSnipFile = {
  relativeDir: string;
  path: string;
  text: string;
  truncated: boolean;
};

export async function loadProjectAgentsSnip(input: AiChatSendInput): Promise<string> {
  const workspaceRoot = input.workspace?.root?.trim();
  if (!workspaceRoot) return "";

  const startDir = resolveAgentsStartDirectory(workspaceRoot, input.activeDocument?.path ?? null);
  const directories = walkUpAgentsDirectories(workspaceRoot, startDir);
  if (directories.length === 0) return "";

  const files: ProjectAgentsSnipFile[] = [];
  let remainingChars = maxTotalSnipChars;

  for (const directory of directories) {
    if (remainingChars <= 0) break;
    const loaded = await readAgentsFileInDirectory(workspaceRoot, directory);
    if (!loaded) continue;

    const isRoot = relativeDirectoryLabel(workspaceRoot, directory) === ".";
    const perFileBudget = Math.min(isRoot ? maxRootFileChars : maxNestedFileChars, remainingChars);
    const text = truncateText(loaded.text.trim(), perFileBudget);
    if (!text) continue;

    remainingChars -= text.length;
    files.push({
      relativeDir: relativeDirectoryLabel(workspaceRoot, directory),
      path: loaded.path,
      text,
      truncated: loaded.truncated || loaded.text.length > text.length,
    });
  }

  if (files.length === 0) return "";
  return formatProjectAgentsSnip(files);
}

async function readAgentsFileInDirectory(workspaceRoot: string, directory: string) {
  for (const path of agentsFileCandidates(directory)) {
    if (!isPathInsideWorkspace(workspaceRoot, path)) continue;
    try {
      const response = await luxCommands.fsReadText(path, maxRootFileChars + 512);
      if (!response.text.trim()) continue;
      return {
        path: response.path,
        text: response.text,
        truncated: response.truncated,
      };
    } catch {
      continue;
    }
  }
  return null;
}

function formatProjectAgentsSnip(files: ProjectAgentsSnipFile[]) {
  const blocks = files.map((file) => {
    const location = file.relativeDir === "." ? "workspace root" : file.relativeDir;
    const truncatedNote = file.truncated ? " (truncated for context budget)" : "";
    return `### ${location} — ${file.path}${truncatedNote}\n${file.text}`;
  });
  return [
    "Project AGENTS guidance (auto-loaded, read-only)",
    "These snippets come from AGENTS.md files discovered by walking up from the active file to the workspace root. More specific directories appear later and refine broader root rules.",
    "Use RulesContext when you need CLAUDE.md, .cursor/rules, or additional rule files not inlined here.",
    blocks.join("\n\n"),
  ].join("\n\n");
}