import { resolve } from "node:path";
import { build } from "esbuild";

const entryPath = resolve("src/lib/aiProjectAgentsWalkUp.ts");
const bundle = await build({
  entryPoints: [entryPath],
  bundle: true,
  write: false,
  format: "esm",
  platform: "node",
  target: "es2022",
});
const moduleUrl = `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`;
const { walkUpAgentsDirectories } = await import(moduleUrl);

const root = "E:/Projects/demo-repo";
const nested = `${root}/apps/desktop/src/components`;

const dirs = walkUpAgentsDirectories(root, nested);
const expected = [
  root,
  `${root}/apps`,
  `${root}/apps/desktop`,
  `${root}/apps/desktop/src`,
  `${root}/apps/desktop/src/components`,
];

if (dirs.length !== expected.length) {
  throw new Error(`Expected ${expected.length} directories, got ${dirs.length}: ${dirs.join(" | ")}`);
}
for (let index = 0; index < expected.length; index += 1) {
  if (dirs[index] !== expected[index]) {
    throw new Error(`Directory mismatch at ${index}: expected ${expected[index]}, got ${dirs[index]}`);
  }
}

const outside = walkUpAgentsDirectories(root, "D:/outside/other");
if (outside.length !== 1 || outside[0] !== root) {
  throw new Error(`Paths outside workspace must fall back to root only, got: ${outside.join(" | ")}`);
}

console.log("Project AGENTS snip verification passed.");