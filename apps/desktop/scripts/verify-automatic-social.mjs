import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import * as esbuild from "esbuild";

const entry = resolve("src/lib/aiAutomaticSocialMessage.ts");
const bundle = await esbuild.build({
  entryPoints: [entry],
  bundle: true,
  write: false,
  format: "esm",
  platform: "node",
  target: "es2022",
});
const code = bundle.outputFiles[0]?.text;
if (!code) throw new Error("bundle failed");
const moduleUrl = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const { isAutomaticSocialOnlyMessage } = await import(moduleUrl);

if (!isAutomaticSocialOnlyMessage("Привет бро")) throw new Error("greeting should be social-only");
if (!isAutomaticSocialOnlyMessage("hello")) throw new Error("hello should be social-only");
if (isAutomaticSocialOnlyMessage("fix the aquarium bug")) throw new Error("task message must not be social-only");
if (isAutomaticSocialOnlyMessage("привет, добавь тесты для routing")) throw new Error("mixed task must not be social-only");

console.log("automatic social message verification passed");