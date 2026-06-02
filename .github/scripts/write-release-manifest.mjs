import { createHash } from "node:crypto";
import { readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, extname, join } from "node:path";

const assetsRoot = "release-assets";
const artifacts = readdirSync(assetsRoot, { withFileTypes: true })
  .filter((entry) => entry.isFile())
  .map((entry) => describeArtifact(join(assetsRoot, entry.name)))
  .sort((left, right) => left.name.localeCompare(right.name));

if (artifacts.length === 0) {
  throw new Error("No release assets were downloaded.");
}

writeFileSync(
  "release-manifest.json",
  `${JSON.stringify({ artifacts }, null, 2)}\n`,
  "utf8",
);

writeFileSync(
  join(assetsRoot, "SHA256SUMS.txt"),
  `${artifacts.map((artifact) => `${artifact.sha256}  ${artifact.name}`).join("\n")}\n`,
  "utf8",
);

function describeArtifact(path) {
  const bytes = readFileSync(path);
  const name = basename(path);
  return {
    name,
    path,
    platform: platformFor(name),
    size: statSync(path).size,
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}

function platformFor(name) {
  const extension = extname(name).toLowerCase();

  if (extension === ".exe" || extension === ".msi") {
    return "windows";
  }

  if (extension === ".dmg" || extension === ".pkg") {
    return "macos";
  }

  if (extension === ".appimage" || extension === ".deb" || extension === ".rpm") {
    return "linux";
  }

  return "unknown";
}
