const fs = require('fs');
const path = require('path');

const SRC = path.resolve(__dirname, '..', 'apps', 'desktop', 'src');

const fixes = [
  // AiChatPanel.tsx — old luxide* imports need to match restored aspect/* export names
  {
    file: 'components/AiChatPanel.tsx',
    from: 'import { luxideAvailability, luxideWeeklyBadge, useLuxideModelSync, useLuxideUsagePoller } from "../lib/aspect/model-sync";',
    to: 'import { aspectAvailability, aspectWeeklyBadge, useAspectModelSync, useAspectUsagePoller } from "../lib/aspect/model-sync";',
  },
  {
    file: 'components/AiChatPanel.tsx',
    from: 'import { useLuxideUsageStore } from "../lib/aspect/usage-store";',
    to: 'import { useAspectUsageStore } from "../lib/aspect/usage-store";',
  },
  {
    file: 'components/AiChatPanel.tsx',
    from: 'import { isLuxideProvider, relinkLuxide } from "../lib/aspect/enroll";',
    to: 'import { isAspectProvider, relinkAspect } from "../lib/aspect/enroll";',
  },
  // AiChatComposer.tsx
  {
    file: 'components/ai-chat/AiChatComposer.tsx',
    from: 'import { formatLuxideUsageLabel, useLuxideSelectedModelUsage } from "../../lib/aspect/model-sync";',
    to: 'import { formatAspectUsageLabel, useAspectSelectedModelUsage } from "../../lib/aspect/model-sync";',
  },
  // AspectLinkModal.tsx — aspectCommands -> luxCommands
  {
    file: 'components/AspectLink/AspectLinkModal.tsx',
    from: 'import { aspectCommands } from "../lib/tauri/commands";',
    to: 'import { luxCommands } from "../lib/tauri/commands";',
  },
];

let count = 0;
for (const fix of fixes) {
  const fullPath = path.join(SRC, fix.file);
  if (!fs.existsSync(fullPath)) {
    console.log(`  ✗ ${fix.file} — not found`);
    continue;
  }
  let content = fs.readFileSync(fullPath, 'utf-8');
  if (!content.includes(fix.from)) {
    console.log(`  ✗ ${fix.file} — pattern not found`);
    continue;
  }
  content = content.replace(fix.from, fix.to);
  fs.writeFileSync(fullPath, content, 'utf-8');
  console.log(`  ✓ ${fix.file}`);
  count++;
}

// Also replace usage references in AiChatPanel.tsx body (not just imports)
const aiChatPanelPath = path.join(SRC, 'components/AiChatPanel.tsx');
let aicp = fs.readFileSync(aiChatPanelPath, 'utf-8');
const bodyFixes = [
  { from: 'useLuxideModelSync', to: 'useAspectModelSync' },
  { from: 'useLuxideUsagePoller', to: 'useAspectUsagePoller' },
  { from: 'useLuxideUsageStore', to: 'useAspectUsageStore' },
  { from: 'isLuxideProvider', to: 'isAspectProvider' },
  { from: 'relinkLuxide', to: 'relinkAspect' },
];
let changed = false;
for (const fix of bodyFixes) {
  if (aicp.includes(fix.from)) {
    aicp = aicp.replace(new RegExp(fix.from, 'g'), fix.to);
    changed = true;
  }
}
if (changed) {
  fs.writeFileSync(aiChatPanelPath, aicp, 'utf-8');
  console.log('  ✓ components/AiChatPanel.tsx (body)');
  count++;
}

// Fix aspectCommands -> luxCommands in AspectLinkModal.tsx body
const aspectLinkPath = path.join(SRC, 'components/AspectLink/AspectLinkModal.tsx');
let alm = fs.readFileSync(aspectLinkPath, 'utf-8');
if (alm.includes('aspectCommands')) {
  alm = alm.replace(/aspectCommands/g, 'luxCommands');
  fs.writeFileSync(aspectLinkPath, alm, 'utf-8');
  console.log('  ✓ components/AspectLink/AspectLinkModal.tsx (body)');
  count++;
}

console.log(`\nFixed ${count} files.`);
