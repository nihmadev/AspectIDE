import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const desktopRoot = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");

async function bundleModule(relativePath) {
  const result = await esbuild.build({
    entryPoints: [resolve(desktopRoot, relativePath)],
    bundle: true,
    write: false,
    format: "esm",
    platform: "node",
    target: "es2022",
  });
  const code = result.outputFiles[0]?.text;
  if (!code) throw new Error(`failed to bundle ${relativePath}`);
  return import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
}

const slash = await bundleModule("src/lib/aiChatSlashCommands.ts");
const { composerTextAfterSlashPick, parseGoalSlashCommand } = slash;

if (parseGoalSlashCommand("/goal")?.kind !== "incomplete") throw new Error("/goal without text must be incomplete");
if (parseGoalSlashCommand("/goal clear")?.kind !== "clear") throw new Error("/goal clear failed");
if (parseGoalSlashCommand("/goal stop")?.kind !== "clear") throw new Error("/goal stop alias failed");
if (parseGoalSlashCommand("/goal status")?.kind !== "status") throw new Error("/goal status failed");
if (parseGoalSlashCommand("/goal pause")?.kind !== "pause") throw new Error("/goal pause failed");
const set = parseGoalSlashCommand("/goal проверка goal");
if (set?.kind !== "set" || set.goal !== "проверка goal") throw new Error("/goal set failed");
const flagged = parseGoalSlashCommand("/goal fix tests --max-turns 12");
if (flagged?.kind !== "set" || flagged?.limits?.maxRounds !== 12) {
  throw new Error(`/goal flags failed: ${JSON.stringify(flagged)}`);
}
const split = parseGoalSlashCommand("/goal Ship feature :: add tests");
if (split?.kind !== "set" || split.extraMessage !== "add tests") throw new Error("/goal :: split failed");

if (composerTextAfterSlashPick("goal") !== "/goal ") throw new Error("goal pick without draft failed");
if (composerTextAfterSlashPick("goal", "/goal fix bug") !== "/goal fix bug") {
  throw new Error("goal pick must preserve typed args");
}

console.log("goal slash verification passed");