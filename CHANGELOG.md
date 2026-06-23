# Changelog

All notable changes to Lux IDE will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses semantic versioning after the first stable release.

## [Unreleased]

### Added

- Public GitHub project packaging: polished README, screenshots, contribution guide, issue templates, PR template, security policy, support notes, and dependency update config.
- Brand assets under `docs/assets` for repository and app icon generation.

## [1.0.7] - 2026-06-23

### Added

- Seamless mid-work message injection: a message staged while the agent is running can now be folded into the **live** turn at the next round boundary (native `ai_inject_message`), so a recommendation lands during the work — in the gap between the agent's requests — instead of waiting for the whole turn to finish. It renders as a user bubble in order, and never gets lost (a failed inject re-queues as a follow-up turn).
- Live MCP self-service for the agent: a single `McpManage` tool lists / adds / connects / restarts / disconnects / enables / disables / removes MCP servers, so the model can install a capability it lacks (`add` installs + connects, then tools call as `mcp__<id>__<tool>`).
- Queue plate redesign: per-message index badges, mode pills (queued vs recommendation), an accent rail, hover-revealed actions, and larger action buttons — full-width and compact.

### Changed

- Retry backoff is now a gentle linear ladder (1s, 3s, 6s, 9s, 12s, … up to ~10 attempts) instead of an exponential jump to the 30s cap on the third try; a server `Retry-After` still wins when present.
- The system prompt drives the full force-multiplier toolset harder: RecallMemory at the start of non-trivial work, RememberMemory the moment something durable is learned, and WebResearch for anything external or uncertain — framed as a strength signal, not a fallback.
- The integrated terminal was reworked to keep a live per-session xterm instance (no garbled raw-byte replay on session switch), auto-spawn the first session, keep the PTY alive across tab/panel toggles, and toggle with Ctrl+` (Ctrl+Ё). Cleaner full-bleed surface, themed scrollbar, and corrected icons (broom = clear, trash = close).

### Fixed

- Native turns no longer finish as "The turn produced no answer" when a provider ignores `stream:true` and returns a single non-SSE JSON body, or when a reasoning model completes with only thinking text and empty content — both are now parsed and surfaced (Anthropic `content:[{text}]` and OpenAI `choices` shapes).
- Opening another project while one is already open no longer strands the loading plate: the folder is picked first (no overlay on click or cancel), and the `WorkspaceChanged` event is the single source of truth for the load stages.
- Chat markdown no longer glues `**bold**` / `` `code` `` to adjacent words — the trailing-whitespace trim that ran per inline token is now applied only to the whole message.
- Default WebView2 right-click menu is suppressed in the desktop runtime (copy/paste preserved in editable fields).
- Shell and terminal now strip the verbatim `\\?\` Windows path prefix before launching, so `cmd.exe` opens in the workspace folder instead of falling back to `C:\Windows`.

### Security

- Mutating `McpManage` actions are gated through tool approval and the tool is agent/automatic-only; the Shell tool's anti-hang grace window prevents a backgrounded grandchild process from holding the pipe open for the full timeout.

## [1.0.4] - 2026-06-19

### Added

- AI progress narration: the agent now streams short play-by-play notes before tool calls and after notable results (Claude Code / Codex style) instead of working silently. Honored on both the native Rust prompt and the TS fallback, and preserved (just terser) under token-economy mode.
- Live auto-retry UX: transient provider failures retry automatically with a visible amber "Retrying — {reason} · attempt n/m" notice and a live countdown. Retries are now smart and per-failure-budgeted — rate limits / 5xx get up to 10 attempts (longer backoff, honoring `Retry-After`), network/timeout get a few.
- Cursor-style AI file review on the desktop turn loop: edits now register a pending review so changed files show a green/red diff and an Accept/Reject bar (previously only the browser-preview path did this).
- Inline **Update now** button in Settings → Updates that downloads and installs without waiting for the corner notice; the updater became a shared singleton so the corner notice and Settings stay in sync.
- Full **Source Control** panel: stage/unstage, discard (with confirmation), commit and commit-all, push/pull with ahead/behind counts, branch switch/create, per-file `+/−` counts, and a side-by-side HEAD↔working diff view.
- Git change decorations across the workspace — file explorer, editor tabs, and a status-bar branch chip (`⎇ branch ↑↓`) — sharing one decoration source, with folder roll-up tints.
- Workspace AI chat history reached parity with the agent surface: delete (with confirmation), archive, and restore.
- The AI tool map now advertises the built-in CodeGraph tools so the model prefers them over grep for tracing definitions, callers, and dependencies.

### Changed

- New Rust git commands (`stage`/`unstage`/`discard`/`commit`/`push`/`pull`/`branches`/`checkout`/`create`/`file_diff`) back the panel; each busts the status/diff cache so every surface refreshes immediately.

### Fixed

- Rate-limit (429) failures now show a calm, recognizable message with retry instead of a generic "AI request failed"; rate-limit and timeout errors render in amber rather than alarming red.
- Intermediate narration text no longer disappears when the model writes a line and then calls multiple tools in one round (the turn timeline no longer splices a committed text segment on a later empty commit).
- The project-loading overlay only appears on the first workspace open; incremental file-tree refreshes (delete/create/rename/move) now run in the background instead of re-raising the full-screen splash.

## [1.0.3] - 2026-06-19

### Fixed

- Git polling no longer spawns a flood of duplicate `find.exe`/helper processes on Windows: `status`/`diff` now disable `gc.auto`, maintenance, and the `core.fsmonitor` hook, run with `--no-optional-locks`, and are coalesced (single-flight + short TTL) so bursts collapse into one invocation per repo.
- AI requests now retry transient `403` responses (edge/CDN challenges) alongside the existing 429/5xx/network retries.
- Opaque "error decoding response body" stream failures are rewritten into an actionable "stream interrupted" message and classified as retryable.
- The chat **Retry request** action now resumes the failed turn with a `Continue` message, preserving the AI's prior reasoning and tool calls instead of wiping them and replaying the prompt.

## [0.1.0] - 2026-05-31

### Added

- Tauri 2 desktop shell with React workbench, Monaco editor, xterm.js terminal surface, and Rust service crates.
- Rust-first workspace, filesystem, document, search, Git, LSP, terminal, settings, extension metadata, and DAP discovery foundations.
- AI chat and agent workspace surfaces with provider/runtime settings and guarded tool workflows.
- Installer and distribution policy for future signed desktop releases.
