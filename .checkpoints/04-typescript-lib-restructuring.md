# Checkpoint 04: TypeScript Library Restructuring

## Summary
~150 files moved from flat `src/lib/` layout to domain-organized subdirectories. 7 old `luxide*` modules replaced by `aspect/` equivalents.

## Restructured modules

### From flat to subdirectories
| Old (flat) | New (domain) |
|---|---|
| `types.ts` | `types/index.ts` |
| `store.ts` | `store/index.ts` |
| `storeEquality.ts` | `store/equality.ts` |
| `tauri.ts` | `tauri/commands.ts` |
| `tauriDecode.ts` | `tauri/decode.ts` |
| `tauriEvents.ts` | `tauri/events.ts` |
| `tauriRuntime.ts` | `tauri/runtime.ts` |
| `terminalOutput.ts` | `terminal/output.ts` |
| `terminalSpawn.ts` | `terminal/spawn.ts` |
| `terminalTypes.ts` | `terminal/types.ts` |
| `codeGraphStore.ts` | `visualization/code-graph-store.ts` |
| `workspaceActions.ts` | `workspace/actions.ts` |
| `diagramPreview.ts` | `preview/diagram-preview.ts` |
| `sanitizeHtml.ts` | `preview/sanitize-html.ts` |
| `htmlArtifactLang.test.ts` | `preview/html-artifact-lang.test.ts` |
| `fileIconMap.ts` | `explorer/file-icon-map.ts` |
| `fileIcons.tsx` | `explorer/file-icons.tsx` |
| `fileTree.ts` | `explorer/file-tree.ts` |
| `explorerImport.ts` | `explorer/import.ts` |
| `gitDecorations.ts` | `explorer/git-decorations.ts` |
| `keybindings.ts` | `keyboard/keybindings.ts` |
| `editorChatBridge.ts` | `editor/chat-bridge.ts` |
| `documentEdits.ts` | `editor/documents/document-edits.ts` |
| `documents.ts` | `editor/documents/documents.ts` |
| `editorPreferences.ts` | `editor/preferences.ts` |
| `editorPreferenceCommands.ts` | `editor/preference-commands.ts` |
| `editorSelectionBridge.ts` | `editor/selection-bridge.ts` |
| `documentViewRouting.ts` | `editor/documents/view-routing.ts` |
| `spreadsheetDocument.ts` | `editor/documents/spreadsheet-document.ts` |
| `tableDocument.ts` | `editor/documents/table-document.ts` |
| `languageLabels.ts` | `editor/language-labels.ts` |
| `lspAutoInstall.ts` | `editor/lsp-auto-install.ts` |
| `lspInstallStore.ts` | `editor/lsp-install-store.ts` |
| `monacoAiEditDecorations.ts` | `editor/monaco/ai-edit-decorations.ts` |
| `monacoDebugAdapters.ts` | `editor/monaco/debug-adapters.ts` |
| `monacoLspAdapters.ts` | `editor/monaco/lsp-adapters.ts` |
| `openWorkspaceEditorPath.ts` | `editor/open-workspace-editor-path.ts` |
| `editorCloseTargets.ts` | `editor/close-targets.ts` |
| `useAiChatComposerAttachments.ts` | `hooks/use-ai-chat-composer-attachments.ts` |
| `useAiChatScroll.ts` | `hooks/use-ai-chat-scroll.ts` |
| `useComposerSessionDraft.ts` | `hooks/use-composer-session-draft.ts` |
| `useDebouncedValue.ts` | `hooks/use-debounced-value.ts` |
| `useElapsedSeconds.ts` | `hooks/use-elapsed-seconds.ts` |
| `useFileAssetUrl.ts` | `hooks/use-file-asset-url.ts` |
| `useLiveTokenSpeed.ts` | `hooks/use-live-token-speed.ts` |
| `useUpdater.ts` | `hooks/use-updater.ts` |
| `useVoiceInput.ts` | `hooks/use-voice-input.ts` |
| `projectLoadPresentation.ts` | `init/project-load-presentation.ts` |
| `runtimeBootstrap.ts` | `init/runtime-bootstrap.ts` |
| `runtimeProvisionStore.ts` | `init/runtime-provision-store.ts` |

### AI modules reorganization (into `aspector/`)
All `aiChat*`, `aiRuntime*`, `aiSession*`, `aiSubagent*`, `aiGoal*` flat modules moved to:
- `aspector/chat/` — chat types, transport, runtime, history, errors, timeline, etc.
- `aspector/runtime/` — tools, approvals, context, prompts, diagnostics, etc.
- `aspector/session/` — goals, todos, orchestration sanitize
- `aspector/subagents/` — native turn, bridge, file review, policy
- `aspector/utils/` — file context, system prompt, preferences, usage, etc.
- `aspector/automatic/` — loop detector, retry, verification, mode enforcement

### `luxide*` -> `aspect/` replacement
- `luxideEnroll.ts` -> `aspect/enroll.ts`
- `luxideLinkStore.ts` -> `aspect/link-store.ts`
- `luxideModelSync.ts` -> `aspect/model-sync.ts`
- `luxideProvider.test.ts` -> `aspect/provider.test.ts`
- `luxideUsageStore.ts` -> `aspect/usage-store.ts`

### Deleted files
- `aiChatSessionLifecycle.ts` — replaced by `aspector/chat/session-lifecycle.ts`
- `decodeChatDisplayText.ts` — replaced by `aspector/chat/decode-display-text.ts`
- `luxideModelSync.test.ts` — replaced by `aspect/model-sync.test.ts`
