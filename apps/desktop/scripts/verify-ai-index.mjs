import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { build } from "esbuild";

const { buildAiProjectIndexSnapshot, collectAiProjectFileEntries } = await importSource("src/lib/aiProjectIndex.ts");

const workspaceRoot = "C:/work/lux";
const fileTreeDirectories = {
  [workspaceRoot]: [
    entry("AGENTS.md", 1_600),
    entry("README.md", 5_000),
    entry("package.json", 1_200),
    entry("node_modules/react/index.js", 100_000),
    entry("src/main.tsx", 4_500),
    entry("src/main.tsx", 4_500),
    entry("src/App.tsx", 20_000),
    entry("src/App.test.tsx", 8_000),
    entry("docs/architecture/runtime.md", 6_000),
    entry(".codex/memory.md", 2_000),
    entry("assets/splash.png", 90_000),
  ],
  [`${workspaceRoot}/src`]: [
    entry("src/lib/store.ts", 11_000),
    entry("src/generated/huge.ts", 2_500_000),
    directory("src/nested"),
  ],
};

const collected = collectAiProjectFileEntries(fileTreeDirectories);
assertEqual(collected.length, 12, "file tree collection deduplicates file entries and ignores directories");

const snapshot = buildAiProjectIndexSnapshot(collected, {
  finishedAtMs: 150,
  includeImages: false,
  maxIndexedFiles: 20,
  startedAtMs: 100,
  workspaceRoot,
});

assertEqual(snapshot.workspaceRoot, workspaceRoot, "snapshot keeps workspace root");
assertEqual(snapshot.totalFiles, 12, "snapshot counts all file entries before filtering");
assertEqual(snapshot.durationMs, 50, "snapshot reports build duration");
assertEqual(snapshot.source, "file-tree", "snapshot defaults to file-tree source");
assertEqual(snapshot.scanLimit, null, "snapshot defaults to no scan limit");
assertEqual(snapshot.scanTruncated, false, "snapshot is not scan-truncated without a scan limit");
assert(snapshot.ignoredFiles >= 2, "snapshot ignores low-signal dependency and image files");
assertEqual(snapshot.truncatedFiles, 0, "snapshot is not truncated when limit covers eligible files");
assert(snapshot.sourceFiles >= 3, "snapshot classifies source files");
assert(snapshot.testFiles >= 1, "snapshot classifies test files");
assert(snapshot.rulesFiles >= 1, "snapshot classifies project rule files");
assert(snapshot.docsFiles >= 2, "snapshot classifies docs and manifest files");
assert(snapshot.memoryFiles >= 1, "snapshot classifies memory files");
assert(["good", "excellent"].includes(snapshot.quality), "rich project snapshot reaches good or excellent quality");
assertIncludes(snapshot.languageCounts.map((bucket) => bucket.label), "typescript", "language mix includes TypeScript");
assertIncludes(snapshot.topDirectories.map((bucket) => bucket.label), "src", "top directories include source folder");

const anchors = snapshot.importantFiles.map((file) => file.relativePath);
assertEqual(anchors[0], "AGENTS.md", "project rules are the highest-priority context anchor");
assertIncludes(anchors, "README.md", "docs are included as context anchors");
assertIncludes(anchors, "package.json", "manifest files are included as context anchors");
assert(!anchors.includes("node_modules/react/index.js"), "low-signal dependency paths are excluded from anchors");

const limitedSnapshot = buildAiProjectIndexSnapshot(collected, {
  finishedAtMs: 200,
  includeImages: true,
  maxIndexedFiles: 3,
  scanLimit: 12,
  source: "workspace-scan",
  startedAtMs: 100,
  workspaceRoot,
});

assertEqual(limitedSnapshot.indexedFiles, 3, "index respects configured max files");
assertEqual(limitedSnapshot.source, "workspace-scan", "workspace scan source is preserved");
assertEqual(limitedSnapshot.scanLimit, 12, "workspace scan limit is preserved");
assertEqual(limitedSnapshot.scanTruncated, true, "workspace scan reports capped scans when file count reaches scan limit");
assert(limitedSnapshot.truncatedFiles > 0, "index reports files dropped by max file limit");
assertIncludes(limitedSnapshot.languageCounts.map((bucket) => bucket.label), "docs", "limited snapshot still keeps high-value docs/rules first");

console.log("AI project index verification passed.");

function entry(relativePath, size = 1_000) {
  const normalized = relativePath.replaceAll("\\", "/");
  return {
    name: normalized.split("/").pop() ?? normalized,
    path: `${workspaceRoot}/${normalized}`,
    kind: "file",
    size,
    modified_at: "2026-06-02T00:00:00.000Z",
    is_hidden: normalized.startsWith("."),
  };
}

function directory(relativePath) {
  const normalized = relativePath.replaceAll("\\", "/");
  return {
    name: normalized.split("/").pop() ?? normalized,
    path: `${workspaceRoot}/${normalized}`,
    kind: "directory",
    size: 0,
    modified_at: null,
    is_hidden: normalized.startsWith("."),
  };
}

function assert(value, message) {
  if (!value) throw new Error(`AI project index verification failed: ${message}.`);
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`AI project index verification failed: ${message}. Expected ${expected}, got ${actual}.`);
  }
}

function assertIncludes(values, expected, message) {
  if (!values.includes(expected)) {
    throw new Error(`AI project index verification failed: ${message}. Missing ${expected}; got ${values.join(", ")}.`);
  }
}

async function importSource(path) {
  await readFile(resolve(path), "utf8");
  const bundled = await build({
    bundle: true,
    entryPoints: [resolve(path)],
    format: "esm",
    platform: "browser",
    target: "es2022",
    write: false,
  });
  const code = bundled.outputFiles[0]?.text;
  if (!code) throw new Error(`Failed to bundle ${path}.`);
  return import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
}
