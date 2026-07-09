# Milestones

## M0: Foundation Scaffold

- Rust workspace with separated crates for core IDE services.
- Tauri 2 shell with command/event bridge.
- React IDE workbench with activity bar, sidebar, editor area, bottom panel, status bar, and command palette.
- Native bundle configuration for Windows, macOS, and Linux.

## M1: Real Workspace Loop

- Native folder picker and recent workspaces.
- Recursive explorer with lazy directory expansion.
- Monaco document lifecycle backed by `aspect-editor`.
- File watcher events batched through Rust.
- Git status refresh and search over the active workspace.

## M2: Developer Workflows

- Real PTY integration with xterm.js.
- LSP process management and diagnostics.
- Settings UI, keybindings, themes, and persisted profiles.
- First DAP integration.

## M3: Extensibility and Release

- WASM extension host and stable contribution points.
- Signed updater manifests.
- Installer QA matrix across Windows, macOS, and Linux.
- Performance benchmarks for indexing, search, startup, and event batching.
