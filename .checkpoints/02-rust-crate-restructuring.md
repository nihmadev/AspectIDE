# Checkpoint 02: Rust Crate Restructuring

## Summary
Complete rewrite of the Rust workspace: 17 `lux-*` crates removed, 22 `aspect-*` crates created (of which 6 are entirely new).

## Deleted crates (17 crates, ~27k lines removed)

| Crate | Modules |
|---|---|
| `lux-core` | concurrency, file_view, 126 TS bindings |
| `lux-workspace` | lib.rs |
| `lux-fs` | lib.rs |
| `lux-file-intel` | archive_list, database_edit, delimited_edit, office_extract, spreadsheet_edit |
| `lux-editor` | lib.rs |
| `lux-search` | lib.rs |
| `lux-lsp` | discovery, protocol, results, transport |
| `lux-codegraph` | cache, community, detect, export, graph, index, lang, metrics, parse, query, resolve |
| `lux-memory` | model, search, store |
| `lux-skills` | model, parse, store |
| `lux-research` | model, provider, rerank |
| `lux-dap` | protocol, session, workspace |
| `lux-terminal` | lib.rs |
| `lux-ssh` | args, config, model |
| `lux-git` | lib.rs |
| `lux-settings` | lib.rs |
| `lux-extensions` | activation, commands, discovery, manifest, plan, registry, runtime, wasm_preflight |
| `lux-bench` | main.rs |

## Replacement crates (17 crates, expanded modules)
- `aspect-core`: lib.rs, concurrency, debug, error, events, extension, file_view (formats+preview), fs, git, lsp, search, settings, terminal, workspace, 101 TS bindings
- `aspect-workspace`, `aspect-fs`, `aspect-file-intel`, `aspect-editor`, `aspect-search` (expanded — classify, filter, glob, graph, matcher, orchestrate, path, rank, scoring, tokenize, types)
- `aspect-lsp` (expanded — discovery, helpers, manager, protocol/, results/, session, transport, types)
- `aspect-codegraph`, `aspect-memory` (expanded — store/crud, maintenance, relations, retrieval, schema, streams)
- `aspect-skills`, `aspect-research` (expanded — provider/parse+url, rerank/passage+util)
- `aspect-dap` (expanded — session/breakpoints, commands, events, handshake, internal, io, messages, spawn)
- `aspect-terminal`, `aspect-ssh`, `aspect-git` (expanded — branch, command, diff, ops, repo, status)
- `aspect-settings` (expanded — io, keybindings, store)
- `aspect-extensions`, `aspect-bench`

## New crates (5 entirely new)
- **`aspect-agent-tools`**: approval, atomic_write, browser/, console_decode, definitions/ (agent, browser, context, edit, plan, remote, schema, shell), diff_stats, eol, file_read, glob_utils, json_utils, output_truncate, patch/ (apply, prepare, rollback), process_kill, secret_scan, shell_command, subagent, symbol_utils, tool_names/ (aliases, normalize, parallel, rejection, suggest), types/ (file_patch, file_result, glob_result, read_result, shell, symbol)
- **`aspect-aspector-core`**: approval, browser, json_helpers, parallel, plan_quality, protocol/ (request, response, tools), registry, response, secrets, subagent, tool_names, transport/ (anthropic_acc, auth, completion, diagnostic, endpoints, history, inline_think, models, race, reasoning, retry, sse, stream_acc, stream_feed, stream_mode, stream_types, streaming, types), turn/ (constants, env), types, usage
- **`aspect-mcp`**: client, naming, protocol, tool_result, types
- **`aspect-runtimes`**: archive, command, crypto, error, fs, io, lsp/ (github, go, manage, npm, pip, recipes, rustup), platform, resolve, runtime/types
- **`aspect-agent-browser`**: install, invoke, probes, process, read_image, resolver, status, stream, types, validate, version
- **`aspect-security`**: block_device, catastrophic, interpreter, launcher, normalize, read_only, report, risky, rm_detect, splitter

## Dependency changes (Cargo.lock)
- `tauri`: 2.11.2 -> 2.11.5
- `serde_with`: 3.20.0 -> 3.21.0
- `uuid`: 1.23.1 -> 1.23.4
- `wasmtime`: 45.0.0 -> 45.0.3
- `tray-icon`: 0.23.1 -> 0.24.1
- `bitflags`: 2.11.1 -> 2.13.0
- `shlex`: 1.3.0 -> 2.0.1
- `time`: 0.3.47 -> 0.3.53
- Removed: `pathdiff`, `prettyplease`, `unicode-xid`, `wasip3`, `wit-bindgen`, `wasm-encoder`, `wasmparser`
- New: `rand 0.10.2`, `rand_pcg`, `quick-xml 0.41.0`, `serde_core`, `block2`
