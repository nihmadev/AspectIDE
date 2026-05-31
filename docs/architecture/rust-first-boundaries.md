# Rust-First Boundaries

Lux IDE treats React as the presentation layer and Rust as the product engine.

## Rules

- React must not crawl workspaces, read arbitrary project files, spawn processes, run Git, host LSP, own terminal PTY sessions, or index source code.
- React may hold viewport/UI state: selected panel, open command palette, active tab, split sizes, transient form values.
- Rust owns persistent IDE state: workspaces, documents, search, filesystem mutations, Git, settings, keybinding profiles, terminal sessions, LSP, extension host, and update/install metadata.
- Commands cross the Tauri boundary as typed request/response calls.
- Long-running Rust work must report progress through events and support cancellation before production release.

## Command/Event Contract

The public command surface is implemented in `apps/desktop/src-tauri/src/lib.rs` and mirrors the v1 architecture plan:

- workspace: open/close
- fs: read/create/rename/delete
- editor: open/update/save
- search: query
- terminal: create/write
- git: status
- settings: get/set

Events use one channel, `lux://event`, with discriminated payloads from `lux-core::LuxEvent`.

## Type Bindings

Rust structs derive `serde` and `ts-rs`. Frontend DTO imports are routed through `apps/desktop/src/lib/types.ts`, which re-exports the checked-in generated bindings from `crates/lux-core/bindings` instead of maintaining a manual mirror. When a cross-boundary Rust type changes, run the `lux-core` binding export tests before frontend typechecking so the TypeScript contract stays generated from the Rust source of truth.
