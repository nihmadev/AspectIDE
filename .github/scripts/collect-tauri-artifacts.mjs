import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { basename, dirname, extname, join } from "node:path";

// This is a Cargo workspace, so build output lands in the workspace-root `target/`,
// not under the crate. The macOS universal build adds a target-triple segment.
// Collect from whichever known locations exist.
const bundleRootCandidates = [
  "target/release/bundle",
  "target/universal-apple-darwin/release/bundle",
  "apps/desktop/src-tauri/target/release/bundle",
  "apps/desktop/src-tauri/target/universal-apple-darwin/release/bundle",
];
const outputRoot = "release-assets";

// User-facing installers, by platform.
const platformExtensions = new Map([
  ["win32", new Set([".exe", ".msi"])],
  ["darwin", new Set([".dmg", ".pkg"])],
  ["linux", new Set([".appimage", ".deb", ".rpm"])],
]);

// Updater artifacts (the signed bundle the updater downloads + its detached
// signature) use compound suffixes that `extname` cannot match. The updater
// manifest (latest.json) is generated later from these. Collecting them here is
// what makes the GitHub-Releases updater endpoint actually resolve.
// Tauri v2 signs the installer artifacts directly (e.g. `*-setup.exe.sig`,
// `*.AppImage.sig`), so the updater bundle IS the installer plus its detached
// `.sig`. We also keep the legacy compound suffixes (`.nsis.zip[.sig]`,
// `.appimage.tar.gz[.sig]`) for older Tauri output, in case it reappears.
const platformUpdaterSuffixes = new Map([
  ["win32", [".exe.sig", ".msi.sig", ".nsis.zip", ".nsis.zip.sig", ".msi.zip", ".msi.zip.sig"]],
  ["darwin", [".app.tar.gz", ".app.tar.gz.sig", ".dmg.sig"]],
  ["linux", [".appimage.sig", ".deb.sig", ".rpm.sig", ".appimage.tar.gz", ".appimage.tar.gz.sig"]],
]);

const allowedExtensions = platformExtensions.get(process.platform);
const updaterSuffixes = platformUpdaterSuffixes.get(process.platform);

if (!allowedExtensions || !updaterSuffixes) {
  throw new Error(`Unsupported release platform: ${process.platform}`);
}

const bundleRoots = bundleRootCandidates.filter((root) => existsSync(root));
if (bundleRoots.length === 0) {
  throw new Error(
    `Tauri bundle output was not found in any known location: ${bundleRootCandidates.join(", ")}`,
  );
}

rmSync(outputRoot, { force: true, recursive: true });
mkdirSync(outputRoot, { recursive: true });

const matchesUpdaterSuffix = (file) => {
  const lower = file.toLowerCase();
  return updaterSuffixes.some((suffix) => lower.endsWith(suffix));
};

const files = bundleRoots
  .flatMap((root) => walk(root))
  .filter((file) => allowedExtensions.has(extname(file).toLowerCase()) || matchesUpdaterSuffix(file))
  .sort((left, right) => left.localeCompare(right));

if (files.length === 0) {
  throw new Error(
    `No release artifacts found for ${process.platform} in ${bundleRoots.join(", ")}.`,
  );
}

const usedNames = new Set();

for (const file of files) {
  const name = uniqueAssetName(file, usedNames);
  copyFileSync(file, join(outputRoot, name));
  console.log(`Collected ${file} -> ${join(outputRoot, name)}`);
}

function walk(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);

    if (entry.isDirectory()) {
      return walk(path);
    }

    if (entry.isFile() || entry.isSymbolicLink()) {
      return [path];
    }

    return [];
  });
}

function uniqueAssetName(file, usedNames) {
  const preferred = basename(file);

  if (!usedNames.has(preferred)) {
    usedNames.add(preferred);
    return preferred;
  }

  const parent = basename(dirname(file));
  const fallback = `${parent}-${preferred}`;

  if (!usedNames.has(fallback)) {
    usedNames.add(fallback);
    return fallback;
  }

  const suffix = usedNames.size + 1;
  const extension = extname(preferred);
  const stem = preferred.slice(0, -extension.length);
  const numbered = `${parent}-${stem}-${suffix}${extension}`;
  usedNames.add(numbered);
  return numbered;
}
