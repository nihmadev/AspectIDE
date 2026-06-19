# Changelog

All notable changes to Lux IDE will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses semantic versioning after the first stable release.

## [Unreleased]

### Added

- Public GitHub project packaging: polished README, screenshots, contribution guide, issue templates, PR template, security policy, support notes, and dependency update config.
- Brand assets under `docs/assets` for repository and app icon generation.

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
