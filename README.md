<p align="center">
  <img src="docs/assets/lux-mark.svg" width="96" height="96" alt="Lux IDE logo">
</p>

<h1 align="center">Lux IDE</h1>

<p align="center">
  Rust-first desktop IDE for fast, typed, AI-native developer workflows.
</p>

<p align="center">
  <a href="https://github.com/GofMan5/lux-ide/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/GofMan5/lux-ide/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="License" src="https://img.shields.io/badge/license-Apache--2.0-0f172a"></a>
  <img alt="Status" src="https://img.shields.io/badge/status-early%20alpha-f59e0b">
  <img alt="Tauri" src="https://img.shields.io/badge/Tauri-2-24c8db">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-native%20engine-b7410e">
  <img alt="React" src="https://img.shields.io/badge/React-workbench-149eca">
  <img alt="pnpm" src="https://img.shields.io/badge/pnpm-10-f69220">
</p>

![Lux IDE welcome screen](docs/assets/lux-welcome.png)

Lux IDE is an open-source desktop IDE built around a native Rust product engine and a React workbench. The goal is a serious Cursor/VS Code-class tool: real workspace operations, typed IPC, native services, polished editor UX, and AI workflows that can inspect and change a project without turning the app into a browser-only shell.

The project is early alpha. The core workbench is real and usable for development, while release signing, updater channels, extension hosting, and full debug session execution are still in progress.

## Why Lux

- **Rust owns the hard work.** Workspaces, filesystem mutations, document persistence, search, Git, LSP, terminal PTY, settings, keybindings, extension metadata, and installer/update policy stay native.
- **React owns the surface.** The frontend focuses on layout, Monaco integration, panels, dialogs, command composition, and fast interaction.
- **Typed boundaries.** Tauri commands and `lux://event` payloads keep frontend/backend contracts explicit instead of stringly typed glue.
- **AI is a product surface.** Chat, agent workspace, provider settings, local voice input, context budgeting, and guarded file/shell tools are treated as first-class IDE workflows.
- **Open by design.** The roadmap, architecture boundaries, contribution rules, and distribution policy are kept in the repo.

## Current Features

- Native folder picker, recent workspaces, recursive explorer, file create/rename/delete/copy/reveal.
- Monaco editor lifecycle backed by Rust `DocumentStore`: open, edit, save, save as, save all, dirty close guard, tabs, split editors, minimap, word wrap, and font zoom.
- File watcher refresh, workspace search with include/exclude, case, regex, and whole-word filters.
- Git status and diff plumbing for workspace files.
- Integrated terminal surface backed by a Rust PTY service and xterm.js.
- LSP manager with diagnostics, hover, definition, references, symbols, folding, inlay hints, semantic tokens, completion, code actions, formatting, signature help, rename, and workspace edits.
- Settings UI for editor, keybindings, themes, profiles, AI providers, model/runtime options, and voice input.
- AI chat and agent workspace with tool approval modes, file read/write/replace/patch/delete tools, web fetch, shell execution, test health, symbol context, and local STT hooks.
- Extension discovery/status with stable contribution-point metadata.
- DAP workspace inspection for `launch.json` and adapter detection. Debug session lifecycle is not wired yet.

## Screenshots

| Editor | AI Chat |
| --- | --- |
| ![Editor](docs/assets/lux-editor.png) | ![AI chat](docs/assets/lux-ai-chat.png) |

| Agent Workspace | Settings |
| --- | --- |
| ![Agent workspace](docs/assets/lux-agent-workspace.png) | ![Settings](docs/assets/lux-settings.png) |

## Architecture

```text
apps/desktop        Tauri 2 shell, React workbench, Monaco, xterm.js
crates/lux-core     shared DTOs, typed events, generated TypeScript bindings
crates/lux-editor   Rust document store and edit/save lifecycle
crates/lux-fs       filesystem/workspace operations
crates/lux-search   workspace search service
crates/lux-terminal Rust PTY service
crates/lux-lsp      language server lifecycle and LSP translation
crates/lux-dap      debug adapter discovery and DAP protocol helpers
crates/lux-settings persisted settings and keybinding profiles
crates/lux-extensions extension manifest discovery and contribution points
```

Read the architecture contract in [docs/architecture/rust-first-boundaries.md](docs/architecture/rust-first-boundaries.md) and the milestone plan in [docs/architecture/milestones.md](docs/architecture/milestones.md).

## Quick Start

Prerequisites:

- Rust stable toolchain
- Node.js 22+
- pnpm 10+
- Platform dependencies required by Tauri 2

```powershell
pnpm install
pnpm dev
```

For web-only frontend iteration:

```powershell
pnpm dev:web
```

For a production desktop bundle:

```powershell
pnpm tauri:build
```

## Quality Gates

Run these before opening a PR:

```powershell
pnpm typecheck
pnpm build
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

UI changes should also be verified in the running app, including console health, panel layout, keyboard flow, and the affected workflow.

## Roadmap

- Harden the real workspace loop: explorer, document lifecycle, search, Git, settings, and terminal reliability.
- Complete LSP ergonomics across common languages and make diagnostics/navigation feel native.
- Wire full DAP debug sessions from detected configurations.
- Build the WASM extension host and stable public contribution API.
- Add signed release channels, installer QA, and updater artifacts.
- Add startup/search/indexing benchmarks and publish repeatable performance targets.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) before opening a change. Good contributions keep Rust as the product engine, preserve typed IPC, avoid placeholder UX, and include focused tests for shared behavior.

If you are new to the codebase, start with docs, reproducible bugs, focused UI polish, Rust unit tests, LSP/DAP adapters, extension manifest validation, and installer QA.

## License

Lux IDE is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
