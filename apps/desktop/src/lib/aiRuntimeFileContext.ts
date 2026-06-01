import { normalizePathSlashes } from "./aiRuntimeShared";
import type { FsEntry, LspWorkspaceSymbol } from "./types";

export type RelatedFileRelation = "same-directory" | "test" | "style" | "type-definition" | "route" | "schema" | "config" | "entrypoint" | "story" | "barrel" | "nearby-name" | "query-match";

export type RelatedFileDescriptor = {
  entry?: FsEntry;
  path: string;
  relativePath: string;
  lower: string;
  relativeLower: string;
  dir: string;
  relativeDir: string;
  basename: string;
  basenameLower: string;
  extension: string;
  stem: string;
  stemLower: string;
  familyStem: string;
  familyStemLower: string;
};

export type RelatedFileMatch = {
  descriptor: RelatedFileDescriptor;
  score: number;
  relations: Set<RelatedFileRelation>;
  queryHits: string[];
};

export type SemanticSearchResult = {
  type: "symbol" | "text" | "file";
  score: number;
  path: string;
  relativePath?: string;
  line?: number;
  column?: number;
  name?: string;
  kind?: string;
  containerName?: string | null;
  preview?: string;
  matchText?: string;
  source: string;
};

const relatedIgnoredPathPattern = /(^|\/)(node_modules|target|dist|build|out|coverage|\.git|\.next|\.turbo|vendor|venv|\.venv|__pycache__)(\/|$)/;
const relatedBinaryFilePattern = /\.(7z|avi|bmp|class|db|dll|dmg|exe|gif|gz|ico|jar|jpeg|jpg|lockb|mov|mp3|mp4|o|obj|pdf|png|rar|so|tar|ttf|webm|webp|woff2?|zip)$/;
const relatedSourceFilePattern = /\.(astro|c|cc|cpp|cs|css|cxx|go|graphql|gql|h|hpp|html|java|js|json|jsx|kt|kts|less|md|mdx|mjs|mts|php|proto|py|rb|rs|sass|scss|sql|svelte|swift|toml|ts|tsx|vue|xml|ya?ml)$/;
const relatedStopWords = new Set([
  "about", "after", "also", "and", "any", "are", "bug", "can", "code", "create", "default", "edit", "file", "files", "fix", "for", "from", "get", "has", "have", "into", "make", "need", "new", "not", "now", "please", "set", "that", "the", "this", "tool", "tools", "use", "with", "work",
]);
const relatedShortUsefulTokens = new Set(["ai", "api", "ci", "db", "fs", "gh", "ui", "ux"]);
const rulesContextFileNames = new Set(["agents.md", "claude.md", ".cursorrules", "cursor_rules.md", "cursor-rules.md", "codex.md"]);
const docsContextFilePattern = /(^|\/)(readme|contributing|changelog|architecture|docs?|package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle|vite\.config\.|tsconfig\.)/i;
const memoryContextFileNames = new Set(["memory.md", "memories.md", "project-memory.md", "decisions.md", "decision-log.md", "preferences.md", "notes.md", "todo.md", "todos.md", "roadmap.md"]);

export function scoreRelatedFile(descriptor: RelatedFileDescriptor, target: RelatedFileDescriptor | null, queryTokens: string[]): RelatedFileMatch {
  const relations = new Set<RelatedFileRelation>();
  const queryHits: string[] = [];
  let score = 0;

  if (target) {
    const sameDirectory = descriptor.dir === target.dir;
    const sameFamily = Boolean(descriptor.familyStemLower && target.familyStemLower) && descriptor.familyStemLower === target.familyStemLower;
    const siblingFamily = Boolean(descriptor.familyStemLower && target.familyStemLower) && (
      descriptor.stemLower === target.familyStemLower ||
      descriptor.familyStemLower.includes(target.familyStemLower) ||
      target.familyStemLower.includes(descriptor.familyStemLower)
    );

    if (sameDirectory) {
      relations.add("same-directory");
      score += 16;
    }
    if (sameFamily) score += 42;
    else if (sameDirectory && siblingFamily) score += 24;

    const directoryDistance = relatedDirectoryDistance(target.relativeDir, descriptor.relativeDir);
    score += Math.max(0, 18 - directoryDistance * 4);

    if (sameDirectory && isBarrelFile(descriptor)) {
      relations.add("barrel");
      score += 25;
    }
    if (sameDirectory && target.familyStemLower && descriptor.stemLower.includes(target.familyStemLower) && descriptor.familyStemLower !== target.familyStemLower) {
      relations.add("nearby-name");
      score += 12;
    }
    if (sameFamily || sameDirectory || directoryDistance <= 2) {
      const kindScore = addRelatedKindRelations(descriptor, relations);
      score += kindScore;
    }
    if (sameFamily && isSourceCounterpart(descriptor, target)) {
      relations.add("nearby-name");
      score += 18;
    }
  } else {
    const kindScore = addRelatedKindRelations(descriptor, relations);
    score += kindScore > 0 ? Math.min(kindScore, 20) : 0;
    if (isImportantProjectFile(descriptor)) score += 35;
  }

  for (const token of queryTokens) {
    if (descriptor.relativeLower.includes(token)) {
      queryHits.push(token);
      relations.add("query-match");
      score += token.length >= 6 ? 18 : 12;
      if (descriptor.basenameLower.includes(token)) score += 10;
    }
  }

  if (isImportantProjectFile(descriptor)) {
    addImportantFileRelation(descriptor, relations);
    score += target ? 14 : 30;
  }
  if (target && queryHits.length === 0 && relations.size === 0) {
    return { descriptor, score: 0, relations, queryHits };
  }
  if (descriptor.relativeLower.includes("/src/") || descriptor.relativeLower.startsWith("src/")) score += 4;
  if (descriptor.relativeLower.includes("/test") || descriptor.relativeLower.includes("/spec")) score += 4;
  if (descriptor.basenameLower.endsWith(".lock")) score -= 20;

  return { descriptor, score, relations, queryHits };
}

export function isRulesContextPath(path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const lower = file.relativeLower;
  return rulesContextFileNames.has(file.basenameLower) || lower.startsWith(".cursor/rules/") || lower.includes("/.cursor/rules/") || lower.includes("/rules/") && /\.(md|mdx|txt)$/.test(lower);
}

export function isDocsContextPath(path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  return docsContextFilePattern.test(file.relativeLower) && !isLowSignalRelatedPath(path);
}

export function isMemoryContextPath(path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const lower = file.relativeLower;
  if (isLowSignalRelatedPath(path)) return false;
  const isKnownMemoryName = memoryContextFileNames.has(file.basenameLower) || /^(agents\.md|claude\.md|codex\.md|\.cursorrules)$/.test(file.basenameLower);
  if (!/\.(md|mdx|txt|json|ya?ml|toml)$/.test(file.extension.toLowerCase()) && !isKnownExtensionlessProjectFile(lower) && !isKnownMemoryName) return false;
  return isKnownMemoryName ||
    /(^|\/)(adr|adrs|decisions?|memory|notes|roadmap|todos?|\.codex|\.cursor)(\/|$)/.test(lower) ||
    /(^|\/)(agents\.md|claude\.md|codex\.md|\.cursorrules)$/.test(lower);
}

export function scoreRulesFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  if (file.basenameLower === "agents.md") score += 120;
  if (file.basenameLower === ".cursorrules") score += 115;
  if (file.basenameLower === "claude.md") score += 100;
  if (file.relativeLower.startsWith(".cursor/rules/")) score += 90;
  if (file.relativeDir === "" || file.relativeDir === ".") score += 45;
  if (file.relativeLower.includes("rules")) score += 20;
  for (const token of queryTokens) {
    if (file.relativeLower.includes(token)) score += token.length >= 6 ? 18 : 10;
  }
  return score;
}

export function scoreMemoryFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  if (memoryContextFileNames.has(file.basenameLower)) score += 110;
  if (/adr|decision/.test(file.relativeLower)) score += 90;
  if (/memory|preference|notes/.test(file.relativeLower)) score += 85;
  if (/roadmap|todo/.test(file.relativeLower)) score += 60;
  if (/agents\.md|claude\.md|codex\.md|\.cursorrules/.test(file.basenameLower)) score += 72;
  if (file.relativeLower.startsWith(".codex/") || file.relativeLower.startsWith(".cursor/")) score += 58;
  if (file.relativeDir === "" || file.relativeDir === ".") score += 22;
  for (const token of queryTokens) {
    if (file.relativeLower.includes(token)) score += token.length >= 6 ? 22 : 12;
  }
  return score;
}

export function scoreDocsFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  if (/readme/i.test(file.basenameLower)) score += 80;
  if (/(package\.json|cargo\.toml|pyproject\.toml|go\.mod)$/.test(file.basenameLower)) score += 70;
  if (file.relativeLower.startsWith("docs/") || file.relativeLower.includes("/docs/")) score += 45;
  if (/architecture|contributing|changelog/i.test(file.basenameLower)) score += 30;
  if (file.relativeDir === "" || file.relativeDir === ".") score += 20;
  for (const token of queryTokens) {
    if (file.relativeLower.includes(token)) score += token.length >= 6 ? 22 : 12;
  }
  return score;
}

export function passesSemanticPathFilter(path: string, pathFilter: string) {
  return !pathFilter || normalizePathSlashes(path).toLowerCase().includes(pathFilter);
}

export function scoreSemanticSymbol(symbol: LspWorkspaceSymbol, query: string, queryTokens: string[], path: string, workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const name = symbol.name.toLowerCase();
  const container = symbol.container_name?.toLowerCase() ?? "";
  const normalizedQuery = query.toLowerCase();
  let score = 80 + scorePath(file.relativePath);
  if (name === normalizedQuery) score += 90;
  else if (name.includes(normalizedQuery)) score += 55;
  if (container.includes(normalizedQuery)) score += 25;
  for (const token of queryTokens) {
    if (name.includes(token)) score += token.length >= 6 ? 24 : 16;
    if (container.includes(token)) score += 12;
    if (file.relativeLower.includes(token)) score += 10;
  }
  if (isTestFile(file)) score -= 10;
  if (isImportantProjectFile(file)) score += 8;
  return score;
}

export function scoreSemanticTextHit(path: string, preview: string, matchText: string, queryTokens: string[], workspaceRoot: string) {
  const file = createRelatedFileDescriptor({ path }, workspaceRoot);
  const haystack = `${file.relativeLower}\n${preview}\n${matchText}`.toLowerCase();
  let score = 50 + scorePath(file.relativePath);
  for (const token of queryTokens) {
    if (haystack.includes(token)) score += token.length >= 6 ? 18 : 11;
    if (file.basenameLower.includes(token)) score += 10;
  }
  if (/function|class|interface|type|struct|enum|impl|export|const|async/i.test(preview)) score += 12;
  if (isTestFile(file)) score -= 8;
  if (isImportantProjectFile(file)) score += 6;
  return score;
}

export function scoreSemanticFile(file: RelatedFileDescriptor, queryTokens: string[]) {
  let score = 0;
  for (const token of queryTokens) {
    if (file.basenameLower.includes(token)) score += token.length >= 6 ? 34 : 22;
    if (file.familyStemLower.includes(token)) score += 16;
    if (file.relativeLower.includes(token)) score += 10;
  }
  if (score === 0) return 0;
  score += Math.min(scorePath(file.relativePath), 30);
  if (isImportantProjectFile(file)) score += 16;
  if (isTestFile(file)) score -= 6;
  return score;
}

export function upsertSemanticResult(results: Map<string, SemanticSearchResult>, result: SemanticSearchResult) {
  const key = `${result.type}:${normalizePathSlashes(result.path).toLowerCase()}:${result.line ?? 0}:${(result.name ?? result.matchText ?? "").toLowerCase()}`;
  const existing = results.get(key);
  if (!existing || result.score > existing.score) results.set(key, result);
}

export function createRelatedFileDescriptor(entry: Pick<FsEntry, "path"> & Partial<FsEntry>, workspaceRoot: string): RelatedFileDescriptor {
  const path = normalizePathSlashes(entry.path);
  const root = normalizePathSlashes(workspaceRoot).replace(/\/+$/, "");
  const relativePath = root && path.toLowerCase().startsWith(`${root.toLowerCase()}/`)
    ? path.slice(root.length + 1)
    : path;
  const basename = path.split("/").pop() ?? path;
  const dir = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  const relativeDir = relativePath.includes("/") ? relativePath.slice(0, relativePath.lastIndexOf("/")) : "";
  const extension = fileExtension(basename);
  const stem = basename.slice(0, basename.length - extension.length);
  const familyStem = familyStemFromBasename(basename);
  return {
    entry: entry.kind ? entry as FsEntry : undefined,
    path,
    relativePath,
    lower: path.toLowerCase(),
    relativeLower: relativePath.toLowerCase(),
    dir,
    relativeDir,
    basename,
    basenameLower: basename.toLowerCase(),
    extension,
    stem,
    stemLower: stem.toLowerCase(),
    familyStem,
    familyStemLower: familyStem.toLowerCase(),
  };
}

export function tokenizeRelatedQuery(query: string) {
  const tokens = new Set<string>();
  query
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .toLowerCase()
    .split(/[^a-z0-9_-]+/i)
    .map((token) => token.trim().replace(/^[-_]+|[-_]+$/g, ""))
    .filter(Boolean)
    .forEach((token) => {
      if (token.length < 3 && !relatedShortUsefulTokens.has(token)) return;
      if (relatedStopWords.has(token)) return;
      tokens.add(token);
    });
  return Array.from(tokens).slice(0, 12);
}

export function resolveWorkspacePath(path: string, workspaceRoot: string) {
  const normalized = normalizePathSlashes(path.trim());
  if (!workspaceRoot || /^[a-z]:\//i.test(normalized) || normalized.startsWith("/")) return normalized;
  return `${normalizePathSlashes(workspaceRoot).replace(/\/+$/, "")}/${normalized.replace(/^\/+/, "")}`;
}

export function isPathInsideWorkspace(path: string, workspaceRoot: string) {
  const root = normalizePathSlashes(workspaceRoot).replace(/\/+$/, "").toLowerCase();
  const normalized = normalizePathSlashes(path).replace(/\/+$/, "").toLowerCase();
  return Boolean(root) && (normalized === root || normalized.startsWith(`${root}/`));
}

export function isLowSignalRelatedPath(path: string) {
  const lower = normalizePathSlashes(path).toLowerCase();
  return relatedIgnoredPathPattern.test(lower) || relatedBinaryFilePattern.test(lower) || (!relatedSourceFilePattern.test(lower) && !isKnownExtensionlessProjectFile(lower));
}

export function isTestFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(test|spec|tests|specs)([._-]|$)/.test(file.basenameLower) || /(^|\/)(__tests__|tests?|specs?)(\/|$)/.test(file.relativeLower);
}

export function isImportantProjectFile(file: RelatedFileDescriptor) {
  return /(^|\/)(package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle|vite\.config\.|tsconfig\.|jsconfig\.|readme|dockerfile|makefile|\.env\.example)/.test(file.relativeLower);
}

export function isEntrypointFile(file: RelatedFileDescriptor) {
  return /(^|\/)(main|index|app|lib|mod)\.(ts|tsx|js|jsx|rs|go|py|java|cs|kt|swift)$/.test(file.relativeLower) || /(^|\/)(src\/main\.rs|src-tauri\/src\/lib\.rs)$/.test(file.relativeLower);
}

export function isSourcePath(file: RelatedFileDescriptor) {
  return file.relativeLower.includes("/src/") || file.relativeLower.startsWith("src/") || /\.(ts|tsx|js|jsx|rs|py|go|java|kt|cs|vue|svelte|astro)$/.test(file.extension.toLowerCase());
}

export function topDirectory(path: string) {
  const parts = normalizePathSlashes(path).split("/").filter(Boolean);
  if (parts.length === 0) return ".";
  if (parts[0].startsWith(".")) return parts[0];
  return parts.length > 1 && ["apps", "crates", "packages", "src"].includes(parts[0]) ? `${parts[0]}/${parts[1]}` : parts[0];
}

export function languageForPath(path: string) {
  const lower = path.toLowerCase();
  if (lower.endsWith(".tsx") || lower.endsWith(".ts") || lower.endsWith(".mts") || lower.endsWith(".cts")) return "typescript";
  if (lower.endsWith(".jsx") || lower.endsWith(".js") || lower.endsWith(".mjs") || lower.endsWith(".cjs")) return "javascript";
  if (lower.endsWith(".rs")) return "rust";
  if (lower.endsWith(".py")) return "python";
  if (lower.endsWith(".go")) return "go";
  if (lower.endsWith(".java") || lower.endsWith(".kt") || lower.endsWith(".kts")) return "jvm";
  if (lower.endsWith(".cs")) return "csharp";
  if (/\.(css|scss|sass|less)$/.test(lower)) return "styles";
  if (/\.(json|ya?ml|toml|xml)$/.test(lower)) return "config-data";
  if (/\.(md|mdx)$/.test(lower) || /readme|license|notice/.test(lower)) return "docs";
  if (/\.(html|vue|svelte|astro)$/.test(lower)) return "web";
  if (/\.(sql|graphql|gql|proto)$/.test(lower)) return "schema";
  return "other";
}

export function compareRelatedDescriptors(left: RelatedFileDescriptor, right: RelatedFileDescriptor) {
  return scorePath(right.relativePath) - scorePath(left.relativePath) || left.relativeLower.localeCompare(right.relativeLower);
}

export function compactIndexedFile(file: RelatedFileDescriptor) {
  return {
    path: file.path,
    relativePath: file.relativePath,
    language: languageForPath(file.basenameLower),
    size: file.entry?.size ?? null,
    modifiedAt: file.entry?.modified_at ?? null,
  };
}

export function scorePath(path: string) {
  const lower = path.toLowerCase().replaceAll("\\", "/");
  let score = 0;
  if (/package\.json$|cargo\.toml$|vite\.config\.|tsconfig\.|readme|src\/app\.|src\/main\.|src-tauri\/src\/lib\.rs/.test(lower)) score += 100;
  if (lower.includes("/src/")) score += 25;
  if (lower.includes("/components/")) score += 10;
  if (lower.includes("/node_modules/") || lower.includes("/target/") || lower.includes("/dist/")) score -= 200;
  return score;
}

function addRelatedKindRelations(descriptor: RelatedFileDescriptor, relations: Set<RelatedFileRelation>) {
  let score = 0;
  if (isTestFile(descriptor)) {
    relations.add("test");
    score += 35;
  }
  if (isStyleFile(descriptor)) {
    relations.add("style");
    score += 30;
  }
  if (isTypeDefinitionFile(descriptor)) {
    relations.add("type-definition");
    score += 28;
  }
  if (isRouteFile(descriptor)) {
    relations.add("route");
    score += 24;
  }
  if (isSchemaFile(descriptor)) {
    relations.add("schema");
    score += 24;
  }
  if (isConfigFile(descriptor)) {
    relations.add("config");
    score += 18;
  }
  if (isEntrypointFile(descriptor)) {
    relations.add("entrypoint");
    score += 18;
  }
  if (isStoryFile(descriptor)) {
    relations.add("story");
    score += 22;
  }
  if (isBarrelFile(descriptor)) {
    relations.add("barrel");
    score += 14;
  }
  return score;
}

function familyStemFromBasename(basename: string) {
  return basename
    .replace(/(\.d)?\.[^.]+$/, "")
    .replace(/\.(test|spec|stories|story|module|types|schema|route|routes|model|models|entity|entities|service|controller|view|styles?|style|component|page|layout|hook|hooks|util|utils|helper|helpers)$/i, "")
    .replace(/[-_.](test|spec|stories|story|module|types|schema|route|routes|model|models|entity|entities|service|controller|view|styles?|style|component|page|layout|hook|hooks|util|utils|helper|helpers)$/i, "");
}

function fileExtension(basename: string) {
  const lowered = basename.toLowerCase();
  if (lowered.endsWith(".d.ts")) return ".d.ts";
  if (lowered.endsWith(".d.mts")) return ".d.mts";
  if (lowered.endsWith(".d.cts")) return ".d.cts";
  const dot = basename.lastIndexOf(".");
  return dot > 0 ? basename.slice(dot) : "";
}

function relatedDirectoryDistance(left: string, right: string) {
  if (left === right) return 0;
  const leftParts = left.split("/").filter(Boolean);
  const rightParts = right.split("/").filter(Boolean);
  let common = 0;
  while (leftParts[common] && leftParts[common] === rightParts[common]) common += 1;
  return (leftParts.length - common) + (rightParts.length - common);
}

function isKnownExtensionlessProjectFile(lowerPath: string) {
  const basename = lowerPath.split("/").pop() ?? lowerPath;
  return /^(dockerfile|makefile|readme|license|notice|procfile|gemfile|rakefile)$/.test(basename);
}

function isStyleFile(file: RelatedFileDescriptor) {
  return /\.(css|scss|sass|less)$/.test(file.extension.toLowerCase()) || /(^|[._-])(styles?|theme|tokens)([._-]|$)/.test(file.basenameLower);
}

function isTypeDefinitionFile(file: RelatedFileDescriptor) {
  return /\.d\.(ts|mts|cts)$/.test(file.basenameLower) || /(^|[._-])(types?|interfaces?|dto|defs)([._-]|$)/.test(file.basenameLower);
}

function isRouteFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(route|routes|router|page|layout)([._-]|$)/.test(file.basenameLower) || /(^|\/)(app|pages|routes?)(\/|$)/.test(file.relativeLower);
}

function isSchemaFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(schema|schemas|model|models|entity|entities|migration|prisma|graphql|proto)([._-]|$)/.test(file.basenameLower) || /\.(graphql|gql|proto|sql)$/.test(file.extension.toLowerCase());
}

function isConfigFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(config|conf|rc|settings|eslint|prettier|vite|webpack|rollup|tsconfig|jsconfig|cargo|package|pyproject)([._-]|$)/.test(file.basenameLower) || /(^|\/)(package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle|vite\.config\.)/.test(file.relativeLower);
}

function isStoryFile(file: RelatedFileDescriptor) {
  return /(^|[._-])(stories|story)([._-]|$)/.test(file.basenameLower);
}

function isBarrelFile(file: RelatedFileDescriptor) {
  return /^(index|mod|lib)\.(ts|tsx|js|jsx|rs)$/.test(file.basenameLower);
}

function addImportantFileRelation(file: RelatedFileDescriptor, relations: Set<RelatedFileRelation>) {
  if (isConfigFile(file)) relations.add("config");
  if (isEntrypointFile(file)) relations.add("entrypoint");
  if (/readme|license|notice/.test(file.basenameLower)) relations.add("nearby-name");
}

function isSourceCounterpart(file: RelatedFileDescriptor, target: RelatedFileDescriptor) {
  if (file.extension.toLowerCase() === target.extension.toLowerCase()) return false;
  const relatedExtensions = new Set([".ts", ".tsx", ".js", ".jsx", ".css", ".scss", ".sass", ".less", ".d.ts"]);
  return relatedExtensions.has(file.extension.toLowerCase()) && relatedExtensions.has(target.extension.toLowerCase());
}
