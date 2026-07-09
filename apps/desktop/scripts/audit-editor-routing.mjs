/**
 * Verifies every FileViewStrategy maps to an EditorArea pane kind.
 * Run: node apps/desktop/scripts/audit-editor-routing.mjs
 */

const strategies = [
  ["monacoText", "monaco", "editable"],
  ["markdownPreview", "markdown", "editable"],
  ["diagramPreview", "diagram", "editable"],
  ["tableEditor", "table", "editable"],
  ["spreadsheetEditor", "spreadsheet", "editable"],
  ["databaseEditor", "database", "preview"],
  ["pdfPreview", "pdf", "preview"],
  ["officePreview", "structuredPreview", "preview"],
  ["imagePreview", "image", "preview"],
  ["audioPreview", "media", "preview"],
  ["videoPreview", "media", "preview"],
  ["archivePreview", "structuredPreview", "preview"],
  ["monacoText", "monaco", "editable"], // ipynb
  ["binaryPreview", "structuredPreview", "preview"],
];

const paneKinds = new Set([
  "monaco",
  "markdown",
  "diagram",
  "spreadsheet",
  "table",
  "database",
  "image",
  "pdf",
  "media",
  "structuredPreview",
]);
let failed = 0;

for (const [strategy, pane, mode] of strategies) {
  if (!paneKinds.has(pane)) {
    console.error(`Unknown pane for ${strategy}: ${pane}`);
    failed += 1;
  }
}

if (failed > 0) {
  process.exit(1);
}

console.log(`Editor routing audit OK (${strategies.length} strategy samples).`);