# Rust Migration Roadmap — AI runtime & non-visual logic → Rust

> **Goal:** the full Lux IDE backbone runs in **Rust**. The frontend (Tauri + React)
> keeps **only** the visual layer: components, rendering, view routing, editor (Monaco)
> integration, and visual state. Everything else — the AI chat runtime, orchestration,
> tools, prompt assembly, context/search engines, session/state management — moves to
> native Rust for speed, correctness, and lower overhead.
>
> **Hard rule: do not worsen behavior.** Each stage is shippable, behavior-preserving,
> and guarded by characterization (golden) tests that pin current TypeScript output and
> assert the Rust implementation matches. No silent quality regressions.

## Why

- **Speed:** native loop + tool dispatch, no per-tool JS↔Rust IPC round-trips, no
  webview CPU contention with rendering.
- **Quality:** one authoritative implementation of security, ranking, and orchestration
  in a typed, tested, memory-safe language.
- **Economy:** less duplicated logic, smaller hot-path overhead, accurate token/cost
  accounting in one place.

## Target architecture

```
┌─ React + Tauri (VISUAL ONLY) ──────────────────────────────┐
│  components/*.tsx · view routing · Monaco · visual state     │
│  subscribes to events, renders timeline, answers approvals   │
└───────────────▲───────────────────────────┬────────────────┘
        events (lux://ai-turn:*)     commands (ai_run_turn, ai_resolve_approval)
┌───────────────┴───────────────────────────▼────────────────┐
│  RUST CORE (lux-desktop + crates)                            │
│  turn loop · prompt assembly · tool dispatch · context/      │
│  search engines · sessions/goals/todos/subagents/A2A ·       │
│  compaction · permissions · shell-safety · transport+retry   │
└─────────────────────────────────────────────────────────────┘
```

**Bridge contract (the only things that cross to the frontend):**
- Rust → UI **events** (`lux://ai-turn`): assistant text delta, reasoning delta,
  tool-call start/status/result, `approval-required {requestId, preview}`, turn-usage,
  done, error.
- UI → Rust **commands**: `ai_run_turn(...)`, `ai_resolve_approval(requestId, decision)`,
  `ai_cancel_turn(...)`. Approvals and Monaco diff review stay in the visual layer; the
  Rust loop awaits the decision over a channel.

## Scope

- **Migrates to Rust:** `apps/desktop/src/lib/ai*.ts` (84 files, ~14.7k LOC) + non-visual
  helpers (path, mentions, slash commands, presentation logic that is pure).
- **Stays TypeScript/React (visual layer):** `apps/desktop/src/components/**/*.tsx`
  (59 files), view routing, Monaco/editor glue, visual stores, i18n message catalogs,
  thin view-model adapters that map Rust events → React state.
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

### Stage 0 — Security & resilience foundation (DONE ✅)
Already in Rust this cycle: workspace path-scope, shell command safety (`ai_shell_safety`),
declarative permission engine (`ai_permissions`), A2A blackboard (`ai_a2a`), transient
auto-retry in the transport. File ops, shell exec, HTTP/SSE transport, history were
already Rust.

### Stage 1 — Pure-logic tool engines → Rust (no UI coupling)
Self-contained, read-only, highest ROI/lowest risk. Each becomes a native command that
composes `lux-search`/`lux-lsp`/`lux-fs`/`lux-git`; the TS tool delegates, then is removed.
- Semantic search (compose symbols+text+files, rank natively) — replaces 3 IPC calls + TS ranking with one native call.
- RelatedFiles, RepoMap, WorkspaceIndex, ImpactAnalysis, ContextBudgeter ranking.
- DiagnosticsContext / ReadLints / ReviewDiff formatting; token estimation & compaction math.
- **Golden tests:** pin current TS ranking/formatting output, assert Rust parity.

### Stage 2 — Prompt assembly → Rust
Move `buildInitialMessages`, system-prompt assembly, context-source gathering, history
compaction into Rust. Rust returns the full provider message array. Keep `verify-ai-prompt`
budgets enforced in Rust (port the length guards).

### Stage 3 — Turn loop → Rust (the core)
`ai_run_turn` drives model↔tool rounds natively; tool dispatch in Rust; streaming,
tool-call timeline, approvals, and Monaco diff review bridged via events/commands. The
React side becomes a thin renderer + approval responder. This is the centerpiece that
makes the AI chat "fast/native".

### Stage 4 — Session & orchestration state → Rust
Sessions, goals, todos, subagents, checkpoints, compaction state, goal-runs managed and
persisted in Rust (chat history already is). Frontend reads via events/queries.

### Stage 5 — Retire the TS runtime
Delete migrated `ai*.ts`; keep only view-model adapters and the optional browser fallback.

## Done criteria (overall)
- No non-visual business logic remains in `apps/desktop/src/lib` except thin adapters.
- AI chat turn runs entirely in Rust; React only renders and answers approvals.
- All gates green; measured latency/CPU improvement on a real turn; zero behavior regressions
  versus the golden fixtures.

## Progress log
- 2026-06-06 — Stage 0 complete (security/resilience foundation in Rust). Roadmap created.
- 2026-06-06 — Stage 1 started: native `ai_semantic_search` (`ai_semantic.rs`) composes
  lux-lsp + lux-search + lux-fs and ranks in Rust (1 IPC call replaces 3 + TS ranking).
  Scoring/tokenizer ported faithfully with parity unit tests; TS `semanticSearch` now
  delegates to the native command in the desktop runtime (TS kept only as browser fallback).
  Also recorded the Rust-first Language Policy in `AGENTS.md`.
