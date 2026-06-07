// Generates `latest.json` — the Tauri v2 updater manifest — from the signed
// updater bundles collected into `release-assets/`, and writes it there so the
// publish step uploads it as a release asset. The app's `TAURI_UPDATER_ENDPOINTS`
// points at this file's download URL on the GitHub release, so a running install
// resolves it, compares versions, and downloads + verifies the matching bundle.
//
// Each `*.sig` is a detached signature whose sibling (same name without `.sig`)
// is the bundle the updater downloads. We embed the signature content inline and
// point `url` at the bundle's GitHub release download URL.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const assetsRoot = "release-assets";
const version = String(process.env.RELEASE_VERSION ?? "").trim();
const tagName = String(process.env.RELEASE_TAG_NAME ?? "").trim();
const repository = String(process.env.GITHUB_REPOSITORY ?? "").trim();
const pubDate = String(process.env.RELEASE_PUB_DATE ?? "").trim() || new Date().toISOString();
const notes = String(process.env.RELEASE_NOTES ?? "").trim() || `Lux IDE ${version}`;

if (!version) throw new Error("RELEASE_VERSION is required.");
if (!tagName) throw new Error("RELEASE_TAG_NAME is required.");
if (!repository) throw new Error("GITHUB_REPOSITORY is required.");

// Maps a collected bundle filename to its Tauri updater target key. Tauri keys
// platforms as `<os>-<arch>`; the universal macOS build serves both arches.
function targetsForAsset(name) {
  const lower = name.toLowerCase();
  if (lower.endsWith(".nsis.zip") || lower.endsWith(".msi.zip")) {
    return ["windows-x86_64"];
  }
  if (lower.endsWith(".app.tar.gz")) {
    // Universal-darwin bundle is valid for both Apple Silicon and Intel.
    return ["darwin-x86_64", "darwin-aarch64"];
  }
  if (lower.endsWith(".appimage.tar.gz")) {
    return ["linux-x86_64"];
  }
  return [];
}

const downloadUrl = (assetName) =>
  `https://github.com/${repository}/releases/download/${tagName}/${encodeURIComponent(assetName)}`;

const entries = readdirSync(assetsRoot, { withFileTypes: true })
  .filter((entry) => entry.isFile() && entry.name.toLowerCase().endsWith(".sig"));

const platforms = {};

for (const entry of entries) {
  const bundleName = entry.name.slice(0, -".sig".length);
  const targets = targetsForAsset(bundleName);
  if (targets.length === 0) {
    console.warn(`Skipping signature with unrecognized target: ${entry.name}`);
    continue;
  }
  const signature = readFileSync(join(assetsRoot, entry.name), "utf8").trim();
  const url = downloadUrl(bundleName);
  for (const target of targets) {
    if (platforms[target]) {
      console.warn(`Duplicate updater target ${target}; keeping first (${platforms[target].url}).`);
      continue;
    }
    platforms[target] = { signature, url };
  }
}

if (Object.keys(platforms).length === 0) {
  throw new Error(
    "No updater signatures found in release-assets. Ensure createUpdaterArtifacts is enabled and TAURI_SIGNING_PRIVATE_KEY is set so *.sig bundles are produced.",
  );
}

const manifest = {
  version,
  notes,
  pub_date: pubDate,
  platforms,
};

const manifestPath = join(assetsRoot, "latest.json");
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
console.log(`Wrote ${manifestPath} for ${Object.keys(platforms).length} target(s):`);
for (const [target, info] of Object.entries(platforms)) {
  console.log(`  ${target} -> ${info.url}`);
}
