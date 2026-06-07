import { readdir, stat } from "node:fs/promises";
import { join } from "node:path";

const distAssetsDir = join("dist", "assets");
// Default ceiling for eager / feature chunks — the figure that actually governs
// cold-start cost, kept strict.
const maxAnyChunkBytes = 450 * 1024;
const maxEntryChunkBytes = 300 * 1024;
// Lazily-loaded heavy vendor chunks (diagram + math rendering) are split out and
// only fetched when a Mermaid/KaTeX preview is opened — never on startup. They
// carry an irreducible third-party footprint (Mermaid core alone is ~1.9 MB), so
// they get an explicit higher ceiling instead of inflating the global budget.
// Each prefix is matched against chunk names; anything not listed stays strict.
const lazyVendorBudgets = [
  { prefix: "vendor-mermaid-", maxBytes: 2_000 * 1024 },
  { prefix: "vendor-graph-cytoscape-", maxBytes: 512 * 1024 },
  { prefix: "vendor-graph-dagre-", maxBytes: 512 * 1024 },
  { prefix: "vendor-graph-elk-", maxBytes: 768 * 1024 },
  { prefix: "vendor-katex-", maxBytes: 512 * 1024 },
  { prefix: "vendor-d3-", maxBytes: 512 * 1024 },
];
const requiredChunkPrefixes = [
  "AiChatPanel-",
  "BottomPanel-",
  "CommandPalette-",
  "EditorArea-",
  "SettingsDialog-",
  "Sidebar-",
  "vendor-react-",
  "vendor-terminal-",
];

function budgetForChunk(name) {
  const lazy = lazyVendorBudgets.find((entry) => name.startsWith(entry.prefix));
  return lazy ? lazy.maxBytes : maxAnyChunkBytes;
}

const entries = await readdir(distAssetsDir);
const jsAssets = entries.filter((entry) => entry.endsWith(".js"));
if (jsAssets.length === 0) {
  throw new Error("Bundle budget verification failed: no JavaScript assets found in dist/assets.");
}

const sizes = await Promise.all(
  jsAssets.map(async (name) => ({ name, bytes: (await stat(join(distAssetsDir, name))).size })),
);
const errors = [];
for (const asset of sizes) {
  const budget = budgetForChunk(asset.name);
  if (asset.bytes > budget) {
    errors.push(`${asset.name} is ${formatBytes(asset.bytes)}, above ${formatBytes(budget)}.`);
  }
}

const entryChunk = sizes.find((asset) => /^index-[\w-]+\.js$/.test(asset.name));
if (!entryChunk) {
  errors.push("Entry chunk index-*.js was not found.");
} else if (entryChunk.bytes > maxEntryChunkBytes) {
  errors.push(`${entryChunk.name} is ${formatBytes(entryChunk.bytes)}, above entry budget ${formatBytes(maxEntryChunkBytes)}.`);
}

for (const prefix of requiredChunkPrefixes) {
  if (!sizes.some((asset) => asset.name.startsWith(prefix))) {
    errors.push(`Expected split chunk ${prefix}*.js was not emitted.`);
  }
}

if (errors.length > 0) {
  throw new Error(`Bundle budget verification failed:\n- ${errors.join("\n- ")}`);
}

const largest = [...sizes].sort((left, right) => right.bytes - left.bytes).slice(0, 5);
console.log(`Bundle budget verification passed (${sizes.length} JS chunks, largest: ${largest.map((asset) => `${asset.name} ${formatBytes(asset.bytes)}`).join(", ")}).`);

function formatBytes(bytes) {
  return `${(bytes / 1024).toFixed(1)} KiB`;
}
