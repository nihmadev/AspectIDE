# Contributing to Lux IDE

Lux IDE is a Rust-first desktop IDE. Contributions should make the editor more real, faster, clearer, or easier to maintain. Avoid decorative rewrites, mock-only features, and placeholder product flows.

## Setup

Install:

- Rust stable
- Node.js 22+
- pnpm 10+
- Tauri 2 system dependencies for your OS

Then run:

```powershell
pnpm install
pnpm dev
```

Use `pnpm dev:web` only when the change does not need native Tauri commands.

## Engineering Rules

- Rust is the product engine. React must not crawl workspaces, mutate arbitrary files, spawn processes, run Git, host LSP, own terminal PTY sessions, index code, or manage updater/install metadata.
- React owns presentation state: layout, selected panels, open dialogs, transient form values, editor chrome, and command composition.
- IPC must stay typed through Tauri commands, generated bindings, and `lux://event` payloads.
- User-visible workflows must work end to end. Disabled controls are acceptable only when they honestly expose an unfinished subsystem, as with the current debug session lifecycle.
- Keep changes scoped. Refactor when it directly improves the requested behavior, removes real duplication, or protects a shared contract.
- Match existing UX patterns unless the change includes a clear product reason.

## Pull Requests

- Keep PRs focused on one feature, fix, or cleanup theme.
- Include screenshots or short recordings for visible UI changes.
- Mention affected Rust crates, Tauri commands, events, or generated TypeScript bindings.
- Add tests for shared Rust behavior, document lifecycle, file operations, search, settings, LSP/DAP translation, extension metadata, and IPC contracts.
- Do not commit build outputs, caches, local logs, generated app bundles, or personal environment files.

## Quality Gates

Run before submitting:

```powershell
pnpm typecheck
pnpm build
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
```

For UI changes, also open the app and verify the actual workflow, keyboard path, empty/error states, console health, and responsive panel layout.

## Generated Bindings

Frontend DTOs are generated from Rust types in `crates/lux-core`. When a cross-boundary type changes, regenerate/check bindings through the crate tests before frontend typechecking. Do not manually mirror DTOs in React.

## Issue Quality

Good bug reports include:

- OS and architecture
- Lux IDE version or commit
- exact steps to reproduce
- expected vs actual behavior
- logs/screenshots when relevant
- whether the bug affects desktop, web-only dev mode, or both

Good feature requests explain the workflow, the user problem, expected interaction, and why it belongs in the Rust engine, React surface, or extension layer.

## Licensing

Unless explicitly marked otherwise, contributions are submitted under the Apache License, Version 2.0, matching this repository.
