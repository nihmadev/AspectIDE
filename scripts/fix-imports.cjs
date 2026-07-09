const fs = require('fs');
const path = require('path');

const SRC = path.resolve(__dirname, '..', 'apps', 'desktop', 'src');

const MOVE_MAP = {
  'aiChatRuntime': 'aspector/chat/runtime',
  'aiChatTransport': 'aspector/chat/transport',
  'aiChatTimeline': 'aspector/chat/timeline',
  'aiChatTypes': 'aspector/chat/types',
  'aiChatHistory': 'aspector/chat/history',
  'aiChatErrors': 'aspector/chat/errors',
  'aiChatDisplayText': 'aspector/chat/display-text',
  'aiChatQueue': 'aspector/chat/queue',
  'aiChatReasoning': 'aspector/chat/reasoning',
  'aiChatMentions': 'aspector/chat/mentions',
  'aiChatSlashCommands': 'aspector/chat/slash-commands',
  'aiChatSessionLifecycle': 'aspector/chat/session-lifecycle',
  'aiChatSessionTitle': 'aspector/chat/session-title',
  'aiChatSessionExtras': 'aspector/chat/session-extras',
  'aiChatCheckpointInput': 'aspector/chat/checkpoint-input',
  'aiChatCheckpointStore': 'aspector/chat/checkpoint-store',
  'aiChatExport': 'aspector/chat/export',
  'aiChatPresentation': 'aspector/chat/presentation',
  'aiChatPendingApproval': 'aspector/chat/pending-approval',
  'aiChatPlanHandoff': 'aspector/chat/plan-handoff',
  'aiChatDocumentAttachment': 'aspector/chat/document-attachment',
  'aiChatComposerSession': 'aspector/chat/composer-session',
  'aiChatGoalOrchestration': 'aspector/chat/goal-orchestration',
  'aiChatMentionAttachments': 'aspector/chat/mention-attachments',
  'aiChatPanelTurnHelpers': 'aspector/chat/panel-turn-helpers',
  'aiChatPathEvidence': 'aspector/chat/path-evidence',
  'aiChatProjectCommands': 'aspector/chat/project-commands',
  'aiChatTurnCheckpoints': 'aspector/chat/turn-checkpoints',
  'aiChatTurnRestore': 'aspector/chat/turn-restore',
  'aiChatTurnRuntime': 'aspector/chat/turn-runtime',
  'aiActiveTurns': 'aspector/chat/active-turns',
  'aiChatComposerAttachments': 'aspector/chat/composer-attachments',
  'aiChatComposerInlineMentions': 'aspector/chat/composer-inline-mentions',
  'aiChatContextCompaction': 'aspector/chat/context-compaction',
  'aiChatContextReport': 'aspector/chat/context-report',
  'aiChatContextUsage': 'aspector/chat/context-usage',
  'aiChatErrorHistory': 'aspector/chat/error-history',
  'decodeChatDisplayText': 'aspector/chat/decode-display-text',

  'aiRuntimeApprovals': 'aspector/runtime/approvals',
  'aiRuntimeBrowser': 'aspector/runtime/browser',
  'aiRuntimeCheckpoints': 'aspector/runtime/checkpoints',
  'aiRuntimeContextBudget': 'aspector/runtime/context-budget',
  'aiRuntimeContextSources': 'aspector/runtime/context-sources',
  'aiRuntimeContextTools': 'aspector/runtime/context-tools',
  'aiRuntimeDiagnostics': 'aspector/runtime/diagnostics',
  'aiRuntimeExploreTools': 'aspector/runtime/explore-tools',
  'aiRuntimeFileContext': 'aspector/runtime/file-context',
  'aiRuntimeFileTools': 'aspector/runtime/file-tools',
  'aiRuntimePatch': 'aspector/runtime/patch',
  'aiRuntimePrompt': 'aspector/runtime/prompt',
  'aiRuntimeSecretGuard': 'aspector/runtime/secret-guard',
  'aiRuntimeSemanticSearch': 'aspector/runtime/semantic-search',
  'aiRuntimeShared': 'aspector/runtime/shared',
  'aiRuntimeShellTools': 'aspector/runtime/shell-tools',
  'aiRuntimeSshTools': 'aspector/runtime/ssh-tools',
  'aiRuntimeTerminal': 'aspector/runtime/terminal',
  'aiRuntimeToolApproval': 'aspector/runtime/tool-approval',
  'aiRuntimeToolBridge': 'aspector/runtime/tool-bridge',
  'aiRuntimeToolDispatch': 'aspector/runtime/tool-dispatch',
  'aiRuntimeTools': 'aspector/runtime/tools',
  'aiRuntimeToolSession': 'aspector/runtime/tool-session',
  'aiFileContext': 'aspector/utils/file-context',

  'aiSessionGoal': 'aspector/session/goal/session-goal',
  'aiSessionGoalRun': 'aspector/session/goal/session-goal-run',
  'aiSessionOrchestrationSanitize': 'aspector/session/orchestration/sanitize',
  'aiSessionTodos': 'aspector/session/todos',
  'aiGoalEvaluator': 'aspector/session/goal/evaluator',
  'aiGoalRunLimits': 'aspector/session/goal/run-limits',
  'aiGoalRunPrompt': 'aspector/session/goal/run-prompt',
  'aiGoalRunPromptBlocks': 'aspector/session/goal/run-prompt-blocks',
  'aiGoalRunRefusalGuard': 'aspector/session/goal/run-refusal-guard',

  'aiAutomaticModeEnforcement': 'aspector/automatic/mode-enforcement',
  'aiAutomaticModeInstructions': 'aspector/automatic/mode-instructions',
  'aiAutomaticRetry': 'aspector/automatic/retry',
  'aiAutomaticSocialMessage': 'aspector/automatic/social-message',
  'aiAutomaticVerification': 'aspector/automatic/verification',
  'aiLoopDetector': 'aspector/automatic/loop-detector',
  'aiOutputNormalizer': 'aspector/automatic/output-normalizer',

  'aiSubagentErrorFormat': 'aspector/subagents/error-format',
  'aiSubagentPolicy': 'aspector/subagents/policy',
  'aiSubagentRuns': 'aspector/subagents/runs',
  'aiSubagents': 'aspector/subagents/subagents',
  'aiNativeFileReview': 'aspector/subagents/native-file-review',
  'aiNativeOrchestrationBridge': 'aspector/subagents/native-orchestration-bridge',
  'aiNativeSubagentEvents': 'aspector/subagents/native-subagent-events',
  'aiNativeTurn': 'aspector/subagents/native-turn',

  'aiPreferences': 'aspector/utils/preferences',
  'aiProviderModels': 'aspector/utils/provider-models',
  'aiSystemPrompt': 'aspector/utils/system-prompt',
  'aiFileDiffHunks': 'aspector/utils/file-diff-hunks',
  'aiFileReviewBridge': 'aspector/utils/file-review/bridge',
  'aiFileReviewPolicy': 'aspector/utils/file-review/policy',
  'aiPendingFileReview': 'aspector/utils/pending-file-review',
  'aiPendingPlan': 'aspector/utils/pending-plan',
  'aiPendingQuestion': 'aspector/utils/pending-question',
  'aiModelContext': 'aspector/utils/model-context',
  'aiModelOverrides': 'aspector/utils/model-overrides',
  'aiProjectAgentsSnip': 'aspector/utils/project-agents-snip',
  'aiProjectAgentsWalkUp': 'aspector/utils/project-agents-walk-up',
  'aiProjectIndex': 'aspector/utils/project-index',
  'aiProjectIndexPolicy': 'aspector/utils/project-index-policy',
  'aiRetryNotice': 'aspector/utils/retry-notice',
  'aiShellLiveOutput': 'aspector/utils/shell-live-output',
  'aiTurnActivity': 'aspector/utils/turn-activity',
  'aiTurnFileSummary': 'aspector/utils/turn-file-summary',
  'aiTurnUsage': 'aspector/utils/usage/turn-usage',
  'aiUsageLog': 'aspector/utils/usage/usage-log',
  'aiVisionFormat': 'aspector/utils/vision-format',

  'agentBrowser': 'agent-browser/core',
  'agentBrowserAutoUpdate': 'agent-browser/auto-update',
  'agentBrowserCommandCatalog': 'agent-browser/command-catalog',
  'agentBrowserCommandReference': 'agent-browser/command-reference',
  'agentBrowserPreviewDocument': 'agent-browser/preview-document',
  'agentBrowserSkillsCache': 'agent-browser/skills-cache',
  'agentBrowserStream': 'agent-browser/stream',

  'aspectEnroll': 'aspect/enroll',
  'aspectLinkStore': 'aspect/link-store',
  'aspectModelSync': 'aspect/model-sync',
  'aspectProvider': 'aspect/provider',
  'aspectUsageStore': 'aspect/usage-store',

  'editorChatBridge': 'editor/chat-bridge',
  'editorCloseTargets': 'editor/close-targets',
  'editorPreferenceCommands': 'editor/preference-commands',
  'editorPreferences': 'editor/preferences',
  'editorSelectionBridge': 'editor/selection-bridge',
  'documentEdits': 'editor/documents/document-edits',
  'documents': 'editor/documents/documents',
  'documentViewRouting': 'editor/documents/view-routing',
  'monacoAiEditDecorations': 'editor/monaco/ai-edit-decorations',
  'monacoDebugAdapters': 'editor/monaco/debug-adapters',
  'monacoLspAdapters': 'editor/monaco/lsp-adapters',
  'languageLabels': 'editor/language-labels',
  'lspAutoInstall': 'editor/lsp-auto-install',
  'lspInstallStore': 'editor/lsp-install-store',
  'openWorkspaceEditorPath': 'editor/open-workspace-editor-path',
  'spreadsheetDocument': 'editor/documents/spreadsheet-document',
  'tableDocument': 'editor/documents/table-document',

  'terminalOutput': 'terminal/output',
  'terminalSpawn': 'terminal/spawn',
  'terminalTypes': 'terminal/types',

  'tauri': 'tauri/commands',
  'tauriDecode': 'tauri/decode',
  'tauriEvents': 'tauri/events',
  'tauriRuntime': 'tauri/runtime',

  'store': 'store/index',
  'storeEquality': 'store/equality',

  'types': 'types/index',

  'keybindings': 'keyboard/keybindings',
  'workspaceActions': 'workspace/actions',

  'projectLoadPresentation': 'init/project-load-presentation',
  'runtimeBootstrap': 'init/runtime-bootstrap',
  'runtimeProvisionStore': 'init/runtime-provision-store',

  'sanitizeHtml': 'preview/sanitize-html',
  'htmlArtifactLang': 'preview/html-artifact-lang',
  'diagramPreview': 'preview/diagram-preview',

  'codeGraphStore': 'visualization/code-graph-store',

  'fileTree': 'explorer/file-tree',
  'fileIconMap': 'explorer/file-icon-map',
  'fileIcons': 'explorer/file-icons',
  'explorerImport': 'explorer/import',
  'gitDecorations': 'explorer/git-decorations',

  'useDebouncedValue': 'hooks/use-debounced-value',
  'useElapsedSeconds': 'hooks/use-elapsed-seconds',
  'useFileAssetUrl': 'hooks/use-file-asset-url',
  'useUpdater': 'hooks/use-updater',
  'useLiveTokenSpeed': 'hooks/use-live-token-speed',
  'useVoiceInput': 'hooks/use-voice-input',
  'useComposerSessionDraft': 'hooks/use-composer-session-draft',
  'useAiChatComposerAttachments': 'hooks/use-ai-chat-composer-attachments',
  'useAiChatScroll': 'hooks/use-ai-chat-scroll',
};

const LIB_DIR = path.join(SRC, 'lib');

function collectAllFiles(dir) {
  const results = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name !== 'node_modules') results.push(...collectAllFiles(full));
    } else if (/\.(ts|tsx)$/.test(entry.name)) {
      results.push(full);
    }
  }
  return results;
}

/** Get the lib-relative directory of a file (empty string for lib/ itself, null for outside lib/) */
function fileDirRel(filePath) {
  const rel = path.relative(LIB_DIR, filePath).replace(/\\/g, '/');
  if (rel.startsWith('..')) return null;
  const idx = rel.lastIndexOf('/');
  return idx === -1 ? '' : rel.slice(0, idx);
}

const importRegex = /from\s+['"]([^'"]+)['"]|require\(['"]([^'"]+)['"]\)/g;

function fixFile(filePath) {
  const oldContent = fs.readFileSync(filePath, 'utf-8');
  const dirRel = fileDirRel(filePath);
  const isInLib = dirRel !== null;

  let newContent = oldContent;
  const replacements = [];

  while ((match = importRegex.exec(oldContent)) !== null) {
    const importPath = match[1] || match[2];
    const fullMatch = match[0];
    if (!importPath) continue;

    // Case 1: from "./lib/<oldname>" (App.tsx, main.tsx)
    let m = importPath.match(/^\.\/lib\/(.+)$/);
    if (m && MOVE_MAP[m[1]]) {
      const newPath = `./lib/${MOVE_MAP[m[1]]}`;
      replacements.push([fullMatch, `from "${newPath}"`]);
      continue;
    }

    // Case 2: from "../../lib/<oldname>" (components/*.tsx, etc.)
    // Also matches "../../lib/subdir/file" patterns
    // Find the LAST '/lib/' and split there
    const lastLib = importPath.lastIndexOf('/lib/');
    if (lastLib !== -1 && importPath.startsWith('..')) {
      const prefix = importPath.slice(0, lastLib + 5); // includes '/lib/'
      const oldMod = importPath.slice(lastLib + 5);    // after '/lib/'
      if (MOVE_MAP[oldMod]) {
        const newPath = prefix + MOVE_MAP[oldMod];
        replacements.push([fullMatch, `from "${newPath}"`]);
      }
      continue;
    }

    // Case 3: from "./<oldname>" — internal lib/ cross-reference
    if (isInLib) {
      const localRef = importPath.replace(/^\.\//, '');
      if (MOVE_MAP[localRef]) {
        const toRel = MOVE_MAP[localRef];
        // Compute relative path from dirRel to toRel
        if (!dirRel) {
          replacements.push([fullMatch, `from "./${toRel}"`]);
        } else {
          const dParts = dirRel.split('/');
          const tParts = toRel.split('/');
          let i = 0;
          while (i < dParts.length && i < tParts.length && dParts[i] === tParts[i]) i++;
          const up = dParts.length - i;
          const down = tParts.slice(i);
          const segs = [];
          for (let j = 0; j < up; j++) segs.push('..');
          segs.push(...down);
          replacements.push([fullMatch, `from "./${segs.join('/')}"`]);
        }
      }
    }
  }

  if (replacements.length > 0) {
    for (const [oldStr, newStr] of replacements) {
      newContent = newContent.replace(oldStr, newStr);
    }
  }

  if (newContent !== oldContent) {
    fs.writeFileSync(filePath, newContent, 'utf-8');
    const relPath = path.relative(SRC, filePath);
    console.log(`  ✓ ${relPath.replace(/\\/g, '/')} (${replacements.length})`);
    return replacements.length;
  }
  return 0;
}

// First, restore modified files back to original (before any import changes)
// File moves via git mv are preserved in the index
const { execSync } = require('child_process');
const root = path.resolve(__dirname, '..');
console.log('Restoring working tree files to index state...');
execSync('git checkout -- apps/desktop/src/', { cwd: root });
console.log('Done.\n');

// Now apply correct import fixes
console.log('Scanning files...');
const files = collectAllFiles(SRC).filter(f => !f.endsWith('.d.ts'));
console.log(`Found ${files.length} .ts/.tsx files`);

let totalChanges = 0;
let changedFiles = 0;
for (const file of files) {
  const changes = fixFile(file);
  if (changes > 0) { changedFiles++; totalChanges += changes; }
}
console.log(`\nDone! Updated ${totalChanges} imports across ${changedFiles} files.`);
