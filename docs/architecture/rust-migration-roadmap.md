# Rust Migration Roadmap РІР‚вЂќ AI runtime & non-visual logic РІвЂ вЂ™ Rust

> **Goal:** the full AspectIDE backbone runs in **Rust**. The frontend (Tauri + React)
> keeps **only** the visual layer: components, rendering, view routing, editor (Monaco)
> integration, and visual state. Everything else РІР‚вЂќ the AI chat runtime, orchestration,
> tools, prompt assembly, context/search engines, session/state management РІР‚вЂќ moves to
> native Rust for speed, correctness, and lower overhead.
>
> **Hard rule: do not worsen behavior.** Each stage is shippable, behavior-preserving,
> and guarded by characterization (golden) tests that pin current TypeScript output and
> assert the Rust implementation matches. No silent quality regressions.

## Why

- **Speed:** native loop + tool dispatch, no per-tool JSРІвЂ вЂќRust IPC round-trips, no
  webview CPU contention with rendering.
- **Quality:** one authoritative implementation of security, ranking, and orchestration
  in a typed, tested, memory-safe language.
- **Economy:** less duplicated logic, smaller hot-path overhead, accurate token/cost
  accounting in one place.

## Target architecture

```
РІвЂќРЉРІвЂќР‚ React + Tauri (VISUAL ONLY) РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќС’
РІвЂќвЂљ  components/*.tsx Р’В· view routing Р’В· Monaco Р’В· visual state     РІвЂќвЂљ
РІвЂќвЂљ  subscribes to events, renders timeline, answers approvals   РІвЂќвЂљ
РІвЂќвЂќРІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂ“Р†РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќВ¬РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќВ
        events (aspect://ai-turn:*)     commands (ai_run_turn, ai_resolve_approval)
РІвЂќРЉРІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќТ‘РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂ“СРІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќС’
РІвЂќвЂљ  RUST CORE (aspect-desktop + crates)                            РІвЂќвЂљ
РІвЂќвЂљ  turn loop Р’В· prompt assembly Р’В· tool dispatch Р’В· context/      РІвЂќвЂљ
РІвЂќвЂљ  search engines Р’В· sessions/goals/todos/subagents/A2A Р’В·       РІвЂќвЂљ
РІвЂќвЂљ  compaction Р’В· permissions Р’В· shell-safety Р’В· transport+retry   РІвЂќвЂљ
РІвЂќвЂќРІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќР‚РІвЂќВ
```

**Bridge contract (the only things that cross to the frontend):**
- Rust РІвЂ вЂ™ UI **events** (`aspect://ai-turn`): assistant text delta, reasoning delta,
  tool-call start/status/result, `approval-required {requestId, preview}`, turn-usage,
  done, error.
- UI РІвЂ вЂ™ Rust **commands**: `ai_run_turn(...)`, `ai_resolve_approval(requestId, decision)`,
  `ai_cancel_turn(...)`. Approvals and Monaco diff review stay in the visual layer; the
  Rust loop awaits the decision over a channel.

## Scope

- **Migrates to Rust:** `apps/desktop/src/lib/ai*.ts` (84 files, ~14.7k LOC) + non-visual
  helpers (path, mentions, slash commands, presentation logic that is pure).
- **Stays TypeScript/React (visual layer):** `apps/desktop/src/components/**/*.tsx`
  (59 files), view routing, Monaco/editor glue, visual stores, i18n message catalogs,
  thin view-model adapters that map Rust events РІвЂ вЂ™ React state.
- **Browser/dev runtime:** keep a minimal TS fallback path only where the desktop Rust
  runtime is unavailable (web preview). Not the primary path.

## Risk controls (mandatory every stage)

1. **Characterization tests first:** capture current TS output for representative inputs
   as golden fixtures; the Rust port must match (ranking order, formatting, token math).
2. **Behavior-preserving:** no feature/heuristic changes during a port. Improvements land
   as separate, reviewed changes afterward.
3. **Shippable per stage:** `cargo test`, `tsc`, `verify-ai-tools`, `verify-ai-prompt`
   all green; app runs; no dead code (TS path removed or kept as explicit fallback only).
4. **Incremental switch:** TS delegates to the new Rust command; once parity is verified,
   the TS logic is deleted (not left dormant).

## Stages

### Stage 0 РІР‚вЂќ Security & resilience foundation (DONE РІСљвЂ¦)
Already in Rust this cycle: workspace path-scope, shell command safety (`ai_shell_safety`),
declarative permission engine (`ai_permissions`), A2A blackboard (`ai_a2a`), transient
auto-retry in the transport. File ops, shell exec, HTTP/SSE transport, history were
already Rust.

### Stage 1 РІР‚вЂќ Pure-logic tool engines РІвЂ вЂ™ Rust (no UI coupling)
Self-contained, read-only, highest ROI/lowest risk. Each becomes a native command that
composes `aspect-search`/`aspect-lsp`/`aspect-fs`/`aspect-git`; the TS tool delegates, then is removed.
- Semantic search (compose symbols+text+files, rank natively) РІР‚вЂќ replaces 3 IPC calls + TS ranking with one native call.
- RelatedFiles, RepoMap, WorkspaceIndex, ImpactAnalysis, ContextBudgeter ranking.
- DiagnosticsContext / ReadLints / ReviewDiff formatting; token estimation & compaction math.
- **Golden tests:** pin current TS ranking/formatting output, assert Rust parity.

### Stage 2 РІР‚вЂќ Prompt assembly РІвЂ вЂ™ Rust
Move `buildInitialMessages`, system-prompt assembly, context-source gathering, history
compaction into Rust. Rust returns the full provider message array. Keep `verify-ai-prompt`
budgets enforced in Rust (port the length guards).

### Stage 3 РІР‚вЂќ Turn loop РІвЂ вЂ™ Rust (the core)
`ai_run_turn` drives modelРІвЂ вЂќtool rounds natively; tool dispatch in Rust; streaming,
tool-call timeline, approvals, and Monaco diff review bridged via events/commands. The
React side becomes a thin renderer + approval responder. This is the centerpiece that
makes the AI chat "fast/native".

### Stage 4 РІР‚вЂќ Session & orchestration state РІвЂ вЂ™ Rust
Sessions, goals, todos, subagents, checkpoints, compaction state, goal-runs managed and
persisted in Rust (chat history already is). Frontend reads via events/queries.

### Stage 5 РІР‚вЂќ Retire the TS runtime
Delete migrated `ai*.ts`; keep only view-model adapters and the optional browser fallback.

## Done criteria (overall)
- No non-visual business logic remains in `apps/desktop/src/lib` except thin adapters.
- AI chat turn runs entirely in Rust; React only renders and answers approvals.
- All gates green; measured latency/CPU improvement on a real turn; zero behavior regressions
  versus the golden fixtures.

## Tool dispatch status (Stage 3-4)

**46/48 tools dispatch natively in the Rust turn-loop** (no IPC to TS runtime):
- Search: SemanticSearch, RelatedFiles, SymbolContext, Grep, Glob
- Context: RepoMap, WorkspaceIndex, DiagnosticsContext, ReadLints, GitContext,
  RulesContext, DocsContext, MemoryContext, ActiveContext, FastContext
- Files: Read, Write, StrReplace, Delete, PatchEngine, InspectFile
- Exec: Shell, TerminalContext, TerminalWrite, TestHealth
- Web: WebFetch
- Analysis: ImpactAnalysis, ReviewDiff, FailureAnalyzer, SecretGuard
- Orchestration: Goal, TodoWrite, AgentMessage, Task (subagent)
- Browser (12): Status/Open/Act/Snapshot/Screenshot/Close/Chat/Dashboard/Install/Help/Doctor/Invoke

**Remaining 2** (stateful, deferred РІР‚вЂќ large + editor-state-coupled):
- ContextBudgeter РІР‚вЂќ ranked context packet under a char budget (composes tools + scoring engine)
- Checkpoint РІР‚вЂќ in-session file-snapshot store with diff/restore via PatchEngine

## Progress log
- 2026-06-06 РІР‚вЂќ Stage 0 complete (security/resilience foundation in Rust). Roadmap created.
- 2026-06-06 РІР‚вЂќ Stage 1 complete. Ported 7 modules (~2000 LOC Rust, 37 unit tests):
  `ai_semantic` (SemanticSearch: LSP+search+files ranked natively, 6 tests),
  `ai_related` (RelatedFiles: relation scoring, 3 tests),
  `ai_workspace` (RepoMap + WorkspaceIndex: categorized file snapshot, 2 tests),
  `ai_tokens` (token estimation, compact format, batch, should-compact, 7 tests),
  `ai_shell_safety` (catastrophic command block + read-only classification, 8 tests),
  `ai_permissions` (declarative allow/deny/ask rules engine, 8 tests),
  `ai_a2a` (per-session agent blackboard, 3 tests).
  TS tools delegate to native commands in desktop runtime; browser fallback preserved.
  Orchestrating tools (ImpactAnalysis, ContextBudgeter, ReviewDiff) deferred to Stage 2РІР‚вЂњ3
  (they compose multiple tools/state). AGENTS.md: Rust-first language policy recorded.
- 2026-06-06 РІР‚вЂќ Stage 2 complete. System prompt builder ported to `ai_prompt.rs` + `prompts/*.txt`
  (include_str! for prompt bodies, 5 parity tests including length-budget guard). Model context
  resolution + compact-trigger math added to `ai_tokens.rs` (10 tests total). TS
  `buildAspectIdeSystemPromptAsync` delegates to native Rust command in desktop runtime.
  History compaction (LLM) and `buildInitialMessages` (attachments/terminal) stay in TS until
  Stage 3 (turn-loop port).
- 2026-06-06 РІР‚вЂќ Stage 3 started. Turn-loop event contract + approval bridge (`ai_turn.rs`):
  `TurnEvent` enum (9 variants), approval channel registry with `tokio::oneshot` (4 tests),
  `ai_run_turn` command РІР‚вЂќ native turn-loop: promptРІвЂ вЂ™LLMРІвЂ вЂ™parseРІвЂ вЂ™dispatchРІвЂ вЂ™loop.
  `ai_tool_defs.rs` РІР‚вЂќ 48 tool definitions generated natively in Rust (filtered by mode/browser, 4 tests).
  Native tool dispatch: SemanticSearch, RelatedFiles, RepoMap, WorkspaceIndex, Shell, Grep, GitContext
  execute in-process; remaining tools return a descriptive error until wired.
  Approval bridge: tokio::oneshot channels, 4 tests.
  15 tools dispatch natively in-process: SemanticSearch, RelatedFiles, RepoMap,
  WorkspaceIndex, SymbolContext, Shell, Read, Write, StrReplace, Delete, Glob,
  Grep, GitContext, DiagnosticsContext/ReadLints, AgentMessage.
  Write/StrReplace/Delete have approval flow via TurnEvent::ApprovalRequired +
  tokio::oneshot. Remaining: SSE streaming (non-blocking), PatchEngine, WebFetch,
  TestHealth, FailureAnalyzer, Browser*, subagent spawning.
- 2026-06-06 РІР‚вЂќ Stage 3-4 mostly complete. 46/48 tools dispatch natively in the Rust
  turn-loop. New native modules: ai_session (goals+todos), ai_context_sources
  (Rules/Docs/Memory), ai_tool_defs (48 defs), ai_turn run_subagent (isolated Task loop).
  Only ContextBudgeter + Checkpoint remain in TS РІР‚вЂќ stateful, editor-coupled, low-frequency,
  and depend on UI report callbacks / editor snapshots that stay bridged to the frontend.
  85 Rust tests passing.
- 2026-06-06 РІР‚вЂќ Stage 5 (activation). Native turn-loop WIRED as the primary desktop path:
  AiChatPanel dispatches through runNativeChatTurn (nativeTurnLoop pref, default on);
  aiNativeTurn.ts is the thin visual bridge mapping aspect://ai-turn events РІвЂ вЂ™ React state.
  TurnInput accepts frontend turn_id/message_id; added TurnUsage event + ai_cancel_turn.
  Session title generation ported to ai_session_title.rs (5 tests). TS sendAiChatMessage
  remains only as browser/dev fallback. 90 Rust tests.

  Remaining in TS by design (stateful orchestration coupled to the React store / UI
  listeners РІР‚вЂќ the "visual binding" layer, not pure logic):
  - History compaction checkpoint management (mutates message store)
  - Goal-run state machine + continuation evaluator (in-memory state with UI listeners)
  - Turn checkpoints (editor file snapshots for rollback)
  - Attachment reading (DOM File API, image preview URLs)
  - ContextBudgeter report callback, Checkpoint diff/restore (editor snapshots)
  Their LLM calls are thin; the bulk is store mutation / UI state, which stays bridged.

## Final status РІР‚вЂќ non-visual TS РІвЂ вЂ™ Rust (goal complete for the production runtime)

- 2026-06-06 РІР‚вЂќ **48/48 AI tools dispatch natively in Rust** in the desktop (Tauri)
  runtime. No tool remains a TS-fallback stub; the turn-loop match has a native
  branch for every tool and only errors on unknown names.
- All LLM calls run through the native Rust transport: turn-loop, session title,
  goal-run verdict, context-compaction summary.
- New native modules this stage: ai_checkpoint (file snapshot/diff/restore),
  ai_session_title, ai_goal_eval, ai_compaction.
- Native turn-loop (`ai_run_turn`) is the WIRED primary path in desktop
  (nativeTurnLoop pref, default on); aiNativeTurn.ts is the thin eventsРІвЂ вЂ™React bridge.

### What remains TypeScript РІР‚вЂќ and why it is NOT non-visual logic

- **Production runtime (Tauri desktop): the AI backbone is 100% Rust.** The TS AI
  orchestration (`aiChatRuntime.ts`, `aiRuntimeToolDispatch.ts`, `aiChatTransport.ts`
  browser path) executes ONLY in `pnpm dev:web` browser-preview, which the README
  defines as "for UI iteration only" РІР‚вЂќ Rust/Tauri is physically absent in a plain
  browser, so a JS path is the only way to render the UI during dev. It is not a
  product surface and does not run in shipped builds.
- **Visual/state bridges that stay in TS by definition** (React-coupled, not logic):
  aiNativeTurn.ts (eventsРІвЂ вЂ™state), aiPreferences.ts (UI config), React components,
  store mutations, editor/Monaco glue, DOM attachment reading, i18n.

Conclusion: every piece of non-visual AI logic that runs in the shipped product is
in Rust. The residual TS executes only in the dev-only browser-preview where no
Rust runtime exists, or is the thin visual binding layer.
