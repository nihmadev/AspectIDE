import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const source = readFileSync(join(root, "src/lib/aiRuntimeTools.ts"), "utf8");

const blocked = ["Write", "StrReplace", "PatchEngine", "Delete", "Shell", "Task"];
for (const tool of blocked) {
  if (!source.includes(`"${tool}"`)) {
    console.error(`missing blocked tool name: ${tool}`);
    process.exit(1);
  }
}
if (!source.includes("readOnlyBlockedToolNames")) {
  console.error("readOnlyBlockedToolNames set not found");
  process.exit(1);
}
if (!source.includes("readOnlyAgentModeToolDenyReason")) {
  console.error("readOnlyAgentModeToolDenyReason not found");
  process.exit(1);
}

console.log("readonly agent tools verification passed");