import fs from "node:fs";
import path from "node:path";

const srcRoot = path.resolve("src");
const used = new Set();
const keyPattern = /\bt\(\s*["']([^"']+)["']/g;

function walk(dir, files = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "node_modules" || entry.name === "dist") continue;
      walk(full, files);
    } else if (/\.(tsx?)$/.test(entry.name)) {
      files.push(full);
    }
  }
  return files;
}

for (const file of walk(srcRoot)) {
  const source = fs.readFileSync(file, "utf8");
  let match;
  while ((match = keyPattern.exec(source))) used.add(match[1]);
}

function dictionaryKeys(filePath) {
  const source = fs.readFileSync(filePath, "utf8");
  return new Set([...source.matchAll(/"([^"]+)":/g)].map((m) => m[1]));
}

const enKeys = dictionaryKeys(path.join(srcRoot, "lib/i18n/messages-en.ts"));
const ruKeys = dictionaryKeys(path.join(srcRoot, "lib/i18n/messages-ru.ts"));

const missingEn = [...used].filter((key) => !enKeys.has(key)).sort();
const missingRu = [...used].filter((key) => enKeys.has(key) && !ruKeys.has(key)).sort();
const extraRu = [...ruKeys].filter((key) => !enKeys.has(key)).sort();

console.log(JSON.stringify({ used: used.size, missingEn, missingRu, extraRuCount: extraRu.length }, null, 2));