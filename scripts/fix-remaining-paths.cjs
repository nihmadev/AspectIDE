const fs = require('fs');
const path = require('path');

const LIB = path.resolve(__dirname, '..', 'apps', 'desktop', 'src', 'lib');

const fixes = [
  // checkpoint-input.ts: AiChatSendInput is in local ./types, DocumentSnapshot etc. are in root types
  {
    file: 'aspector/chat/checkpoint-input.ts',
    from: 'import type { AiChatSendInput } from "./../../types/index";\nimport type { AiModelConfig, AiPreferences, AiProviderConfig } from "./../utils/preferences";\nimport type { Locale } from "../../i18n";\nimport type { DocumentSnapshot, TerminalSessionInfo, WorkspaceInfo } from "./types";',
    to: 'import type { AiChatSendInput } from "./types";\nimport type { AiModelConfig, AiPreferences, AiProviderConfig } from "./../utils/preferences";\nimport type { Locale } from "../../i18n";\nimport type { DocumentSnapshot, TerminalSessionInfo, WorkspaceInfo } from "./../../types/index";',
  },
  // document-attachment.ts: AiChatAttachmentInput is in local ./types
  {
    file: 'aspector/chat/document-attachment.ts',
    from: 'import type { AiChatAttachmentInput } from "./../../types/index";',
    to: 'import type { AiChatAttachmentInput } from "./types";',
  },
  // mention-attachments.ts: AiChatAttachmentInput, AiChatMentionHints are in local ./types  
  {
    file: 'aspector/chat/mention-attachments.ts',
    from: 'import type { AiChatAttachmentInput, AiChatMentionHints } from "./../../types/index";',
    to: 'import type { AiChatAttachmentInput, AiChatMentionHints } from "./types";',
  },
  // browser.ts: ./aiChatTurnRuntime -> ../chat/turn-runtime
  {
    file: 'aspector/runtime/browser.ts',
    from: 'const { bumpBrowserStreamRefresh } = await import("./aiChatTurnRuntime");',
    to: 'const { bumpBrowserStreamRefresh } = await import("./../chat/turn-runtime");',
  },
  // tool-session.ts: ./aiPendingQuestion -> ../utils/pending-question
  {
    file: 'aspector/runtime/tool-session.ts',
    from: 'const { registerPendingQuestion, waitForQuestionAnswer } = await import("./aiPendingQuestion");',
    to: 'const { registerPendingQuestion, waitForQuestionAnswer } = await import("./../utils/pending-question");',
  },
  // tool-session.ts: ./aiPendingPlan -> ../utils/pending-plan
  {
    file: 'aspector/runtime/tool-session.ts',
    from: 'const { registerPendingPlan } = await import("./aiPendingPlan");',
    to: 'const { registerPendingPlan } = await import("./../utils/pending-plan");',
  },
  // tool-session.ts: ./tauri -> ../../tauri/commands
  {
    file: 'aspector/runtime/tool-session.ts',
    from: 'const { isTauriRuntime, luxCommands, MCP_SERVERS_KEY } = await import("./tauri");',
    to: 'const { isTauriRuntime, luxCommands, MCP_SERVERS_KEY } = await import("./../../tauri/commands");',
  },
  // native-turn.ts: ./luxideEnroll -> ../../aspect/enroll
  {
    file: 'aspector/subagents/native-turn.ts',
    from: 'import { resolveProviderApiKey } from "./luxideEnroll";',
    to: 'import { resolveProviderApiKey } from "./../../aspect/enroll";',
  },
  // runs.ts: ./tauri -> ../../tauri/commands
  {
    file: 'aspector/subagents/runs.ts',
    from: 'void import("./tauri")',
    to: 'void import("./../../tauri/commands")',
  },
];

let count = 0;
for (const fix of fixes) {
  const fullPath = path.join(LIB, fix.file);
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

// Fix context-tools.ts inline type imports on line 95
const ctxToolsPath = path.join(LIB, 'aspector/runtime/context-tools.ts');
let ctxTools = fs.readFileSync(ctxToolsPath, 'utf-8');
const oldSig = 'async function resolveIndexLanguages(entries: import("./types").FsEntry[], workspaceRoot: string, options: import("./aiProjectIndex").BuildAiProjectIndexOptions): Promise<import("./aiProjectIndex").AiProjectIndexSnapshot> {';
const newSig = 'async function resolveIndexLanguages(entries: import("./../../types/index").FsEntry[], workspaceRoot: string, options: import("./../utils/project-index").BuildAiProjectIndexOptions): Promise<import("./../utils/project-index").AiProjectIndexSnapshot> {';
if (ctxTools.includes(oldSig)) {
  ctxTools = ctxTools.replace(oldSig, newSig);
  fs.writeFileSync(ctxToolsPath, ctxTools, 'utf-8');
  console.log('  ✓ aspector/runtime/context-tools.ts (inline types)');
  count++;
} else {
  console.log('  ✗ aspector/runtime/context-tools.ts — pattern not found');
}

console.log(`\nFixed ${count} files.`);
