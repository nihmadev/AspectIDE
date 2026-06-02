import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { basename, dirname, extname, join } from "node:path";

const bundleRoot = "apps/desktop/src-tauri/target/release/bundle";
const outputRoot = "release-assets";

const platformExtensions = new Map([
  ["win32", new Set([".exe", ".msi"])],
  ["darwin", new Set([".dmg", ".pkg"])],
  ["linux", new Set([".appimage", ".deb", ".rpm"])],
]);

const allowedExtensions = platformExtensions.get(process.platform);

if (!allowedExtensions) {
  throw new Error(`Unsupported release platform: ${process.platform}`);
}

if (!existsSync(bundleRoot)) {
  throw new Error(`Tauri bundle output was not found: ${bundleRoot}`);
}

rmSync(outputRoot, { force: true, recursive: true });
mkdirSync(outputRoot, { recursive: true });

const files = walk(bundleRoot)
  .filter((file) => allowedExtensions.has(extname(file).toLowerCase()))
  .sort((left, right) => left.localeCompare(right));

if (files.length === 0) {
  throw new Error(
    `No release artifacts found for ${process.platform} in ${bundleRoot}.`,
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
