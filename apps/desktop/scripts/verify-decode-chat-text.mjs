const namedHtmlEntities = {
  quot: '"',
  amp: "&",
  lt: "<",
  gt: ">",
  apos: "'",
  nbsp: "\u00a0",
  ldquo: "\u201C",
  rdquo: "\u201D",
  lsquo: "\u2018",
  rsquo: "\u2019",
};
const fullwidthAmpersandPattern = /\uFF06/g;
const htmlEntityPattern = /&(?:#x([0-9a-f]+)|#(\d+)|([a-z]+));/gi;
const htmlEntityNoSemicolonPattern = /&(?:quot|amp|lt|gt|apos|nbsp)(?![a-z0-9;])/gi;

function decodeHtmlEntitiesOnce(text) {
  const withSemicolon = text.replace(htmlEntityPattern, (entity, hex, decimal, name) => {
    if (hex) return codePointToString(Number.parseInt(hex, 16), entity);
    if (decimal) return codePointToString(Number.parseInt(decimal, 10), entity);
    if (name) return namedHtmlEntities[name.toLowerCase()] ?? entity;
    return entity;
  });
  return withSemicolon.replace(htmlEntityNoSemicolonPattern, (entity) => {
    const name = entity.slice(1).toLowerCase();
    return namedHtmlEntities[name] ?? entity;
  });
}

function codePointToString(codePoint, fallback) {
  if (!Number.isFinite(codePoint) || codePoint < 0) return fallback;
  try {
    return String.fromCodePoint(codePoint);
  } catch {
    return fallback;
  }
}

const goalOrchestrationMarkerLine = /^\s*(?:\[goal:(?:complete|blocked)\]|goal:(?:complete|blocked))\s*$/gim;

function stripGoalOrchestrationMarkers(text) {
  if (!text) return text;
  const stripped = text.replace(goalOrchestrationMarkerLine, "").replace(/\n{3,}/g, "\n\n");
  return stripped.replace(/\s+$/, "");
}

function decodeChatDisplayText(text) {
  let decoded = text;
  if (text.includes("&") || text.includes("\uFF06")) {
    decoded = text.replace(fullwidthAmpersandPattern, "&");
    for (let pass = 0; pass < 4 && decoded.includes("&"); pass += 1) {
      const next = decodeHtmlEntitiesOnce(decoded);
      if (next === decoded) break;
      decoded = next;
    }
  }
  return stripGoalOrchestrationMarkers(decoded);
}

const mustNotContain = ["&quot;", "&#34;", "&#x22;"];
const cases = [
  ["plain", "hello", null],
  ["quot", "Что касается &quot;67&quot; —", "Что касается \"67\" —"],
  ["amp quot", "&amp;quot;67&amp;quot;", "\"67\""],
  ["numeric", "&#34;67&#34;", "\"67\""],
  ["hex", "&#x22;67&#x22;", "\"67\""],
  ["no semi", "test &quot;67&quot end", null],
  ["fullwidth", "Что \uFF06quot;67\uFF06quot;", null],
  ["goal complete", "Summary here.\n[goal:complete]", "Summary here."],
  ["goal blocked", "Need input.\n[goal:blocked]", "Need input."],
  ["goal marker only", "[goal:complete]", ""],
];

let failed = 0;
for (const [name, input, expected] of cases) {
  const out = decodeChatDisplayText(input);
  if (mustNotContain.some((needle) => out.includes(needle))) {
    console.error(`FAIL ${name} still has entity:`, JSON.stringify(out));
    failed += 1;
    continue;
  }
  if (expected !== null && out !== expected) {
    console.error(`FAIL ${name} expected`, JSON.stringify(expected), "got", JSON.stringify(out));
    failed += 1;
    continue;
  }
  console.log(`ok ${name}`);
}
process.exit(failed > 0 ? 1 : 0);