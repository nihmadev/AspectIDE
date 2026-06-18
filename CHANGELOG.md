# Changelog

All notable changes to Lux IDE will be documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and this project uses semantic versioning after the first stable release.

## [Unreleased]

### Added

- Public GitHub project packaging: polished README, screenshots, contribution guide, issue templates, PR template, security policy, support notes, and dependency update config.
- Brand assets under `docs/assets` for repository and app icon generation.

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
