import fs from "node:fs";

const panel = fs.readFileSync("src/components/AiToolsPanel.tsx", "utf8");
const en = fs.readFileSync("src/lib/i18n/messages-en.ts", "utf8");
const ru = fs.readFileSync("src/lib/i18n/messages-ru.ts", "utf8");

const categories = [...panel.matchAll(/id: "([^"]+)"/g)].map((m) => m[1]);
const categoryIds = categories.slice(0, 3);
const toolIds = [...panel.matchAll(/\{ id: "([^"]+)"/g)].map((m) => m[1]);

const missing = { en: [], ru: [] };
for (const id of categoryIds) {
  for (const dict of ["en", "ru"]) {
    const source = dict === "en" ? en : ru;
    for (const suffix of ["title", "subtitle"]) {
      const key = `aiTools.category.${id}.${suffix}`;
      if (!source.includes(`"${key}"`)) missing[dict].push(key);
    }
  }
}
for (const id of toolIds) {
  for (const dict of ["en", "ru"]) {
    const source = dict === "en" ? en : ru;
    const key = `aiTools.tool.${id}.description`;
    if (!source.includes(`"${key}"`)) missing[dict].push(key);
  }
}

console.log(JSON.stringify({ toolIds: toolIds.length, missing }, null, 2));