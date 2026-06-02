import { appendFileSync, readFileSync } from "node:fs";

const rootPackage = JSON.parse(readFileSync("package.json", "utf8"));
const tauriConfig = JSON.parse(readFileSync("apps/desktop/src-tauri/tauri.conf.json", "utf8"));

const version = String(rootPackage.version ?? "").trim();
const tauriVersion = String(tauriConfig.version ?? "").trim();

if (!version) {
  throw new Error("Root package.json must define a release version.");
}

if (version !== tauriVersion) {
  throw new Error(
    `Release version mismatch: package.json=${version}, tauri.conf.json=${tauriVersion}`,
  );
}

const inputTag = String(process.env.INPUT_TAG_NAME ?? "").trim();
const draft = String(process.env.INPUT_DRAFT ?? "false").trim() === "true";
const refType = String(process.env.GITHUB_REF_TYPE ?? "").trim();
const refName = String(process.env.GITHUB_REF_NAME ?? "").trim();
const tagName = refType === "tag" ? refName : inputTag || `v${version}`;
const expectedTag = `v${version}`;

if (tagName !== expectedTag) {
  throw new Error(`Release tag ${tagName} must match app version ${expectedTag}.`);
}

const prerelease = /[-+]/u.test(version);
const releaseName = `Lux IDE ${version}`;
const output = process.env.GITHUB_OUTPUT;

if (!output) {
  throw new Error("GITHUB_OUTPUT is not available.");
}

const lines = [
  `version=${version}`,
  `tag_name=${tagName}`,
  `release_name=${releaseName}`,
  `draft=${String(draft)}`,
  `prerelease=${String(prerelease)}`,
];

appendFileSync(output, `${lines.join("\n")}\n`, "utf8");
