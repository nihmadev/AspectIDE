import { readdir, stat } from "node:fs/promises";
import { join } from "node:path";

const distAssetsDir = join("dist", "assets");
const maxAnyChunkBytes = 450 * 1024;
const maxEntryChunkBytes = 300 * 1024;
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

const entries = await readdir(distAssetsDir);
const jsAssets = entries.filter((entry) => entry.endsWith(".js"));
if (jsAssets.length === 0) {
  throw new Error("Bundle budget verification failed: no JavaScript assets found in dist/assets.");
}

const sizes = await Promise.all(
  jsAssets.map(async (name) => ({ name, bytes: (await stat(join(distAssetsDir, name))).size })),
);
const errors = [];
const oversized = sizes.filter((asset) => asset.bytes > maxAnyChunkBytes);
for (const asset of oversized) {
  errors.push(`${asset.name} is ${formatBytes(asset.bytes)}, above ${formatBytes(maxAnyChunkBytes)}.`);
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
