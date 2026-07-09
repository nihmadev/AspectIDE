const PATH_VERIFICATION_TOOLS = new Set(["Read", "Glob", "Grep", "InspectFile", "RepoMap"]);
const filePathPattern = /(?:^|[\s"'`(])([\w./\\-]+\.(?:ts|tsx|js|jsx|rs|py|md))(?:$|[\s"'`,.)])/gi;
const dirPathPattern = /(?:^|[\s*`#>\-])([a-zA-Z][\w.-]*\/)/g;

function normalizeEvidencePath(path) {
  return path.replace(/\\/g, "/").replace(/^\.\//, "").toLowerCase();
}

function extractFilePaths(text) {
  const paths = [];
  filePathPattern.lastIndex = 0;
  for (const match of text.matchAll(filePathPattern)) {
    const path = match[1]?.replace(/\\/g, "/");
    if (path) paths.push(path);
  }
  return paths;
}

function extractDirPaths(text) {
  const paths = [];
  dirPathPattern.lastIndex = 0;
  for (const match of text.matchAll(dirPathPattern)) {
    const path = match[1]?.replace(/\\/g, "/");
    if (path) paths.push(path);
  }
  return paths;
}

function listUnverified(message) {
  const verified = new Set();
  for (const call of message.toolCalls ?? []) {
    if (!PATH_VERIFICATION_TOOLS.has(call.tool)) continue;
    for (const text of [call.input, call.output]) {
      if (!text) continue;
      for (const p of [...extractFilePaths(text), ...extractDirPaths(text)]) {
        verified.add(normalizeEvidencePath(p));
      }
    }
  }
  const cited = [...new Set([...extractFilePaths(message.content), ...extractDirPaths(message.content)])];
  return cited.filter((path) => {
    const n = normalizeEvidencePath(path);
    if (verified.has(n)) return false;
    for (const v of verified) {
      if (v.startsWith(n) || n.startsWith(v)) return false;
    }
    return true;
  });
}

const message = {
  role: "assistant",
  content: "See docs/readme.md and archives/ for samples.",
  toolCalls: [{ tool: "Read", input: '{"path":"docs/readme.md"}', output: "ok", status: "success" }],
};

const unverified = listUnverified(message);
if (!unverified.includes("archives/")) {
  console.error("expected archives/ unverified, got", unverified);
  process.exit(1);
}
if (unverified.includes("docs/readme.md")) {
  console.error("readme should be verified");
  process.exit(1);
}

console.log("path evidence verification passed");