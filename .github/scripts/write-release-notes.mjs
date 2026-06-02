import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";

const version = String(process.env.RELEASE_VERSION ?? "").trim();
const tagName = String(process.env.RELEASE_TAG_NAME ?? "").trim();
const output = process.env.GITHUB_OUTPUT;

if (!version) {
  throw new Error("RELEASE_VERSION is required.");
}

if (!tagName) {
  throw new Error("RELEASE_TAG_NAME is required.");
}

if (!output) {
  throw new Error("GITHUB_OUTPUT is not available.");
}

mkdirSync("release", { recursive: true });

const manifest = JSON.parse(readFileSync("release-manifest.json", "utf8"));
const groups = groupByPlatform(manifest.artifacts ?? []);
const checksums = readFileSync("release-assets/SHA256SUMS.txt", "utf8").trim();

const body = [
  `# Lux IDE ${version}`,
  "",
  "Production desktop installers built by GitHub Actions from a clean tag.",
  "",
  "## Downloads",
  "",
  ...platformLines(groups),
  "",
  "## Verification",
  "",
  "Every platform build runs dependency install, frontend verification, Rust checks, and native Tauri bundling before assets are attached to this release.",
  "",
  "```text",
  checksums,
  "```",
  "",
].join("\n");

const notesPath = "release/notes.md";
writeFileSync(notesPath, body, "utf8");
appendFileSync(output, `notes_path=${notesPath}\n`, "utf8");

function groupByPlatform(artifacts) {
  return artifacts.reduce((groups, artifact) => {
    const platform = String(artifact.platform ?? "unknown");
    const list = groups.get(platform) ?? [];
    list.push(artifact.name);
    groups.set(platform, list);
    return groups;
  }, new Map());
}

function platformLines(groups) {
  const labels = [
    ["windows", "Windows"],
    ["macos", "macOS"],
    ["linux", "Linux"],
  ];

  return labels.flatMap(([key, label]) => {
    const names = groups.get(key) ?? [];

    if (names.length === 0) {
      return [`- ${label}: no artifact produced`];
    }

    return [`- ${label}: ${names.join(", ")}`];
  });
}
