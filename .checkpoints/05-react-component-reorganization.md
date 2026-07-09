# Checkpoint 05: React Component Reorganization

## Summary
~40 old `ai-chat/` components renamed and moved to `Aspector/` directory. 50+ new components created for Editor, Shell, Sidebar, Settings, Terminal, Welcome screens, and more.

## Renamed components: `ai-chat/` -> `Aspector/`
All old `Ai*` prefixed components renamed to `Aspector*`:

| Old (ai-chat/) | New (Aspector/) |
|---|---|
| `AiChatPanel.tsx` | `AspectorChatPanel.tsx` (+ major rewrite, 2555 lines) |
| `AiChatMessages.tsx` | `AspectorChatMessages.tsx` |
| `AiChatComposer.tsx` | `AspectorChatComposer.tsx` |
| `AiAgentNowBar.tsx` | `AspectorAgentNowBar.tsx` |
| `AiAgentNowPlaque.tsx` | `AspectorAgentNowPlaque.tsx` |
| `AiAgentOrchestrationRail.tsx` | `AspectorAgentOrchestrationRail.tsx` |
| `AiChatHistoryPopover.tsx` | `AspectorChatHistoryPopover.tsx` |
| `AiChatErrorNotice.tsx` | `AspectorChatErrorNotice.tsx` |
| `AiChatGlobalApprovalBanner.tsx` | `AspectorChatGlobalApprovalBanner.tsx` |
| `AiFileReviewBar.tsx` | `AspectorFileReviewBar.tsx` |
| `AiSessionReviewBar.tsx` | `AspectorSessionReviewBar.tsx` |
| `AiPlanCard.tsx` | `AspectorPlanCard.tsx` |
| `AiPlanRunCard.tsx` | `AspectorPlanRunCard.tsx` |
| `AiQuestionCard.tsx` | `AspectorQuestionCard.tsx` |
| `AiRetryBanner.tsx` | `AspectorRetryBanner.tsx` |
| `AiTurnDiagnostics.tsx` | `AspectorTurnDiagnostics.tsx` |
| `AiTurnSummaryCard.tsx` | `AspectorTurnSummaryCard.tsx` |
| `AiSubagentPanel.tsx` | `AspectorSubagentPanel.tsx` |
| `AiToolCall.tsx` | `AspectorToolCall.tsx` (+ rewrite, 424 lines) |
| `AiToolsPanel.tsx` | `AspectorToolsPanel.tsx` (+ rewrite, 380 lines) |
| `AiThinkingIndicator.tsx` | `AspectorThinkingIndicator.tsx` |
| `AiContextIndicator.tsx` | `AspectorContextIndicator.tsx` |
| `AiIndexStatusBanner.tsx` | `AspectorIndexStatusBanner.tsx` |
| `AiAutomaticChecklist.tsx` | `AspectorAutomaticChecklist.tsx` |
| `AiPathEvidenceNotice.tsx` | `AspectorPathEvidenceNotice.tsx` |
| `AiChatClosedNotice.tsx` | `AspectorChatClosedNotice.tsx` |
| `HtmlArtifact.tsx` | `Aspector/HtmlArtifact.tsx` |
| And more... (MentionMenu, SlashMenu, ComposerAttachments, etc.) |

## New components (50+)

### Editor panes
- `EditorArea.tsx` (864 lines), `EditorBreadcrumb.tsx`, `EditorCloseGuard.tsx`
- `DatabaseEditorPane.tsx`, `DiagramEditorPane.tsx`
- `FilePreviewPane.tsx`, `ImageEditorPane.tsx`
- `MarkdownEditorPane.tsx`, `MediaEditorPane.tsx`
- `PdfEditorPane.tsx`, `SpreadsheetEditorPane.tsx`, `TableEditorPane.tsx`

### Shell chrome
- `ActivityBar.tsx`, `BottomPanel.tsx` (457 lines)
- `StatusBar.tsx`, `TitleBar.tsx` (358 lines)

### Sidebar panels
- `ExplorerPanel.tsx` (1160 lines), `ExplorerHelpers.ts`, `ExplorerTypes.ts`
- `SearchPanel.tsx`, `GitPanel.tsx` (488 lines), `GitDiffModal.tsx`
- `RunDebugPanel.tsx` (825 lines), `ExtensionsPanel.tsx`
- `Sidebar.tsx`, `SidebarShared.tsx`

### Settings
- `SettingsDialog.tsx` (1395 lines), `SettingsControls.tsx` (301 lines)
- `AiProvidersSection.tsx` (685 lines), `AiUsageSection.tsx`
- `McpSection.tsx`, `MemorySection.tsx` (434 lines)
- `AgentBrowserSection.tsx`, `SkillsSection.tsx` (441 lines)
- `SshSection.tsx`

### Other
- `XtermTerminal.tsx`, `UpdateNotice.tsx`, `UpdateNoticeHost.tsx`
- `WelcomeScreen.tsx`, `ProjectLoadingStatus.tsx`, `WorkspaceSkeleton.tsx`
- `CommandPalette.tsx` (713 lines)
- `CompactDropdown.tsx` (454 lines)
- `AgentWorkspace.tsx` (356 lines)
- `AspectorLazyChatPanel.tsx`, `AspectorMonacoDiffReview.tsx`
- `MediaAssetView.tsx`
- `App.tsx` — rewired imports for all renames

## Deleted component files
- `SettingsDialog.tsx` (old `ai-chat/` versions)
- `AiComposerInputArea.tsx`, `AiComposerTypes.ts`
- All old `ai-chat/` components (40 files)
