const fs = require('fs');
const path = require('path');

const LIB = path.resolve(__dirname, '..', 'apps', 'desktop', 'src', 'lib');

function collectFiles(dir) {
  const results = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory() && entry.name !== 'node_modules') results.push(...collectFiles(full));
    else if (/\.(ts|tsx)$/.test(entry.name) && !full.includes('node_modules')) results.push(full);
  }
  return results;
}

function fixFile(filePath) {
  const rel = path.relative(LIB, filePath).replace(/\\/g, '/');
  const dirs = rel.split('/').length - 1;
  if (dirs === 0) return false;
  const correct = '../'.repeat(dirs) + 'i18n';

  let content = fs.readFileSync(filePath, 'utf-8');
  const orig = content;

  // Replace `from "./i18n"` / `from "./i18n/xxx"` → correct relative path
  content = content.replace(/from\s+['"]\.\/i18n(\/[^'"]*)?['"]/g, (match, suffix) => {
    return `from "${correct}${suffix || ''}"`;
  });

  // Replace `from "../i18n"` / `from "../i18n/xxx"` when the depth doesn't match
  // These should have exactly `dirs` × `../` prefix
  content = content.replace(/from\s+['"]((?:\.\.\/)+)(i18n(?:\/[^'"]*)?)['"]/g, (match, prefix, tail) => {
    const expected = '../'.repeat(dirs);
    if (prefix === expected) return match; // already correct
    const rest = tail.startsWith('i18n') ? tail.slice(4) : tail; // remove leading "i18n"
    return `from "${correct}${rest}"`;
  });

  if (content !== orig) {
    fs.writeFileSync(filePath, content, 'utf-8');
    console.log(`  ✓ ${rel}`);
    return true;
  }
  return false;
}

console.log('Fixing i18n import paths in lib/ subdirectories...');
const files = collectFiles(LIB).filter(f => !f.startsWith(path.join(LIB, 'i18n')));
let count = 0;
for (const f of files) {
  if (fixFile(f)) count++;
}
console.log(`\nFixed i18n paths in ${count} files.`);
