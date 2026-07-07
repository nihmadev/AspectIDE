import {
  createRelatedFileDescriptor,
  isDocsContextPath,
  isEntrypointFile,
  isImportantProjectFile,
  isLowSignalRelatedPath,
  isMemoryContextPath,
  isRulesContextPath,
  isSourcePath,
  isTestFile,
  languageForPath,
  scorePath,
  topDirectory,
} from "./aiRuntimeFileContext";
import { normalizePathSlashes } from "./aiRuntimeShared";
import type { FileTreeDirectories } from "./fileTree";
import type { FsEntry } from "./types";

export type AiProjectIndexQuality = "empty" | "limited" | "good" | "excellent";

export type AiProjectIndexSource = "file-tree" | "workspace-scan";

export type AiProjectIndexBucket = {
  label: string;
  count: number;
};

export type AiProjectIndexFile = {
  path: string;
  relativePath: string;
  language: string;
  size: number | null;
  modifiedAt: string | null;
  score: number;
};

export type AiProjectIndexSnapshot = {
  workspaceRoot: string | null;
  indexedFiles: number;
  totalFiles: number;
  ignoredFiles: number;
  truncatedFiles: number;
  totalBytes: number;
  sourceFiles: number;
  testFiles: number;
  configFiles: number;
  rulesFiles: number;
  docsFiles: number;
  memoryFiles: number;
  durationMs: number;
  scanLimit: number | null;
  scanTruncated: boolean;
  source: AiProjectIndexSource;
  quality: AiProjectIndexQuality;
  languageCounts: AiProjectIndexBucket[];
  topDirectories: AiProjectIndexBucket[];
  importantFiles: AiProjectIndexFile[];
};

export type BuildAiProjectIndexOptions = {
  finishedAtMs: number;
  includeImages: boolean;
  maxIndexedFiles: number;
  scanLimit?: number | null;
  source?: AiProjectIndexSource;
  startedAtMs: number;
  workspaceRoot: string;
};

type ScoredIndexFile = AiProjectIndexFile & {
  descriptor: ReturnType<typeof createRelatedFileDescriptor>;
  docs: boolean;
  entrypoint: boolean;
  important: boolean;
  memory: boolean;
  rules: boolean;
  source: boolean;
  test: boolean;
};

const INDEX_LANGUAGE_LIMIT = 8;
const INDEX_DIRECTORY_LIMIT = 8;
const INDEX_IMPORTANT_FILE_LIMIT = 12;

export function collectAiProjectFileEntries(directories: FileTreeDirectories): FsEntry[] {
  const byPath = new Map<string, FsEntry>();
  for (const entries of Object.values(directories)) {
    for (const entry of entries) {
      if (entry.kind !== "file") continue;
      byPath.set(normalizePathSlashes(entry.path), entry);
    }
  }
  return Array.from(byPath.values());
}

export function buildAiProjectIndexSnapshot(entries: FsEntry[], options: BuildAiProjectIndexOptions, getLanguage?: (relativePath: string) => string | null): AiProjectIndexSnapshot {
  const maxIndexedFiles = Math.max(1, Math.floor(options.maxIndexedFiles));
  const eligible: ScoredIndexFile[] = [];
  let totalFiles = 0;
  let ignoredFiles = 0;

  for (const entry of entries) {
    if (entry.kind !== "file") continue;
    totalFiles += 1;
    const image = isIndexImagePath(entry.path);
    if ((!options.includeImages && image) || (isLowSignalRelatedPath(entry.path) && !(options.includeImages && image))) {
      ignoredFiles += 1;
      continue;
    }
    eligible.push(scoreIndexFile(entry, options.workspaceRoot, image, getLanguage));
  }

  eligible.sort(compareScoredIndexFiles);
  const indexed = eligible.slice(0, maxIndexedFiles);
  const truncatedFiles = Math.max(0, eligible.length - indexed.length);
  const languageCounts = countBuckets(indexed.map((file) => file.language), INDEX_LANGUAGE_LIMIT);
  const topDirectories = countBuckets(indexed.map((file) => topDirectory(file.descriptor.relativePath)), INDEX_DIRECTORY_LIMIT);
  const importantFiles = indexed
    .filter((file) => file.important || file.rules || file.docs || file.memory || file.entrypoint)
    .slice(0, INDEX_IMPORTANT_FILE_LIMIT)
    .map(stripScoredInternals);
  const rulesFiles = indexed.filter((file) => file.rules).length;
  const docsFiles = indexed.filter((file) => file.docs).length;
  const memoryFiles = indexed.filter((file) => file.memory).length;
  const sourceFiles = indexed.filter((file) => file.source).length;
  const testFiles = indexed.filter((file) => file.test).length;
  const configFiles = indexed.filter((file) => file.important && !file.rules && !file.docs && !file.memory).length;
  const totalBytes = indexed.reduce((total, file) => total + (file.size ?? 0), 0);

  return {
    workspaceRoot: options.workspaceRoot,
    indexedFiles: indexed.length,
    totalFiles,
    ignoredFiles,
    truncatedFiles,
    totalBytes,
    sourceFiles,
    testFiles,
    configFiles,
    rulesFiles,
    docsFiles,
    memoryFiles,
    durationMs: Math.max(0, Math.round(options.finishedAtMs - options.startedAtMs)),
    scanLimit: options.scanLimit ?? null,
    scanTruncated: typeof options.scanLimit === "number" && totalFiles >= options.scanLimit,
    source: options.source ?? "file-tree",
    quality: indexQuality({ docsFiles, importantFiles: importantFiles.length, indexedFiles: indexed.length, memoryFiles, rulesFiles, sourceFiles, truncatedFiles }),
    languageCounts,
    topDirectories,
    importantFiles,
  };
}

function scoreIndexFile(entry: FsEntry, workspaceRoot: string, image: boolean, getLanguage?: (relativePath: string) => string | null): ScoredIndexFile {
  const descriptor = createRelatedFileDescriptor(entry, workspaceRoot);
  const rules = isRulesContextPath(entry.path, workspaceRoot);
  const docs = isDocsContextPath(entry.path, workspaceRoot);
  const memory = isMemoryContextPath(entry.path, workspaceRoot);
  const important = isImportantProjectFile(descriptor);
  const entrypoint = isEntrypointFile(descriptor);
  const source = isSourcePath(descriptor);
  const test = isTestFile(descriptor);
  let score = scorePath(descriptor.relativePath);

  if (rules) score += 260;
  if (memory) score += 210;
  if (docs) score += 170;
  if (important) score += 150;
  if (entrypoint) score += 130;
  if (source) score += 45;
  if (test) score += 12;
  if (image) score -= 30;
  if (entry.is_hidden) score -= 18;
  if (entry.size > 1_000_000) score -= 24;

  return {
    descriptor,
    docs,
    entrypoint,
    important,
    memory,
    rules,
    source,
    test,
    path: descriptor.path,
    relativePath: descriptor.relativePath,
    language: image ? "images" : (getLanguage?.(descriptor.relativePath) ?? languageForPath(descriptor.relativePath)),
    size: Number.isFinite(entry.size) ? entry.size : null,
    modifiedAt: entry.modified_at,
    score,
  };
}

function compareScoredIndexFiles(left: ScoredIndexFile, right: ScoredIndexFile) {
  return right.score - left.score || left.relativePath.localeCompare(right.relativePath);
}

function countBuckets(labels: string[], limit: number): AiProjectIndexBucket[] {
  const counts = new Map<string, number>();
  for (const label of labels) counts.set(label, (counts.get(label) ?? 0) + 1);
  return Array.from(counts, ([label, count]) => ({ label, count }))
    .sort((left, right) => right.count - left.count || left.label.localeCompare(right.label))
    .slice(0, limit);
}

function stripScoredInternals(file: ScoredIndexFile): AiProjectIndexFile {
  return {
    language: file.language,
    modifiedAt: file.modifiedAt,
    path: file.path,
    relativePath: file.relativePath,
    score: file.score,
    size: file.size,
  };
}

function indexQuality(input: { docsFiles: number; importantFiles: number; indexedFiles: number; memoryFiles: number; rulesFiles: number; sourceFiles: number; truncatedFiles: number }): AiProjectIndexQuality {
  if (input.indexedFiles === 0) return "empty";
  const signalScore = (input.rulesFiles > 0 ? 3 : 0)
    + (input.docsFiles > 0 ? 2 : 0)
    + (input.memoryFiles > 0 ? 2 : 0)
    + (input.sourceFiles > 0 ? 2 : 0)
    + (input.importantFiles >= 4 ? 2 : input.importantFiles > 0 ? 1 : 0)
    + (input.indexedFiles >= 300 ? 1 : 0);
  if (signalScore >= 8 && input.truncatedFiles === 0) return "excellent";
  if (signalScore >= 6) return "good";
  return "limited";
}

function isIndexImagePath(path: string) {
  return /\.(avif|gif|ico|jpeg|jpg|png|svg|webp)$/i.test(path);
}
