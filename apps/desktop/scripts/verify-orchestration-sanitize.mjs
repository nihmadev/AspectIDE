import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { transform } from "esbuild";

const sourcePath = resolve("src/lib/aiSessionOrchestrationSanitize.ts");
const source = await readFile(sourcePath, "utf8");
const { code } = await transform(source, { loader: "ts", format: "esm", target: "es2022" });
const moduleUrl = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const { sanitizeSessionGoal, sanitizeSessionTodos } = await import(moduleUrl);

const badGoal = sanitizeSessionGoal("All issues / bugs / gaps found (quoted exactly)");
if (badGoal.ok) throw new Error("review heading should be rejected as goal");

const goodGoal = sanitizeSessionGoal("Ship file preview routing for PDF and spreadsheets.");
if (!goodGoal.ok) throw new Error("valid goal should be accepted");

const badTodos = sanitizeSessionTodos([
  { id: "1", content: "No obsessive review performed.", status: "pending", priority: "medium" },
  { id: "2", content: "Missing required three-section structure.", status: "pending", priority: "medium" },
]);
if (badTodos.ok) throw new Error("review finding todos should be rejected");

const goodTodos = sanitizeSessionTodos([
  { id: "1", content: "Inspect editor routing for PDF tabs.", status: "in_progress", priority: "high" },
  { id: "2", content: "Add regression test for documentViewRouting.", status: "pending", priority: "medium" },
]);
if (!goodTodos.ok) throw new Error("engineering todos should be accepted");

console.log("orchestration sanitize verification passed");