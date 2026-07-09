<p align="center">
  <img src="docs/assets/aspect-mark.svg" width="110" height="110" alt="AspectIDE Р Р†Р вЂљРІР‚Сњ AI-native code editor logo">
</p>

<h1 align="center">AspectIDE</h1>

<p align="center">
  <b>The AI-native desktop IDE with a real Rust engine.</b><br>
  An autonomous coding agent with 45+ native tools, a tree-sitter code graph, persistent project memory,<br>
  parallel subagents, and built-in web research Р Р†Р вЂљРІР‚Сњ inside a fast, polished editor. No Electron. No cloud lock-in.
</p>

<p align="center">
  <a href="https://github.com/nihmadev/AspectIDE/releases/latest"><img alt="Latest release" src="https://img.shields.io/github/v/release/GofMan5/aspect-ide?label=release&color=6d5cff"></a>
  <a href="https://github.com/nihmadev/AspectIDE/releases"><img alt="Downloads" src="https://img.shields.io/github/downloads/GofMan5/aspect-ide/total?color=22c55e"></a>
  <a href="https://github.com/nihmadev/AspectIDE/actions/workflows/ci.yml"><img alt="CI status" src="https://github.com/nihmadev/AspectIDE.git/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT" src="https://img.shields.io/badge/license-MIT-0f172a"></a>
  <a href="https://github.com/nihmadev/AspectIDE/stargazers"><img alt="GitHub stars" src="https://img.shields.io/github.com/nihmadev/AspectIDE?style=flat&color=f59e0b"></a>
</p>

<p align="center">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-24c8db?logo=tauri&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-native%20engine-b7410e?logo=rust&logoColor=white">
  <img alt="React" src="https://img.shields.io/badge/React-workbench-149eca?logo=react&logoColor=white">
  <img alt="TypeScript" src="https://img.shields.io/badge/TypeScript-typed%20IPC-3178c6?logo=typescript&logoColor=white">
  <img alt="Monaco Editor" src="https://img.shields.io/badge/Monaco-editor-0078d4">
</p>

<p align="center">
  <a href="https://github.com/nihmadev/AspectIDE/releases/latest"><b>Р Р†Р’В¬РІР‚РЋ Download for Windows</b></a>
  Р вЂ™Р’В·
  <a href="#-the-ai-agent">AI Agent</a>
  Р вЂ™Р’В·
  <a href="#-editor--workbench">Editor</a>
  Р вЂ™Р’В·
  <a href="#-trust--security">Security</a>
  Р вЂ™Р’В·
  <a href="#%EF%B8%8F-architecture">Architecture</a>
  Р вЂ™Р’В·
  <a href="#-support-the-project">Donate</a>
  Р вЂ™Р’В·
  <a href="https://t.me/aspect_ide">Telegram</a>
</p>

---

![AspectIDE Р Р†Р вЂљРІР‚Сњ Monaco editor with Rust codebase, file explorer, integrated terminal, and AI agent chat side by side](docs/assets/aspect-editor.png)

**AspectIDE** is an open-source, Cursor-class desktop IDE where the heavy lifting Р Р†Р вЂљРІР‚Сњ filesystem, search, Git, LSP, terminal PTY, **and the entire AI agent loop** Р Р†Р вЂљРІР‚Сњ runs in native Rust behind typed IPC, with a React + Monaco workbench on top. The agent isn't a sidebar plugin bolted onto an editor: it is a first-class product surface that inspects your code through a symbol graph, edits with checkpoints and rollback, verifies its own work, and streams every shell command it runs into a live terminal tab you can watch.

> Р Р†РЎв„ўР Р‹ **Install once, stay current** Р Р†Р вЂљРІР‚Сњ releases ship with a signed auto-updater. 19 releases landed in the first 16 days.

> РЎР‚РЎСџР вЂ№Р С“ **Free models built in** Р Р†Р вЂљРІР‚Сњ a curated set of models ships with AspectIDE at no cost. No API key, no card: link your Telegram once (1 account = your own daily/weekly limits) and start coding. Bring your own key any time for the full 35+ provider catalog.

## Why AspectIDE

| | AspectIDE | The usual suspects |
|---|---|---|
| РЎР‚РЎСџР’В¦Р вЂљ **Engine** | Filesystem, search, Git, LSP, PTY, DAP **and the agent runtime** are native Rust crates (~40k lines, ~700 unit tests) behind compile-time-typed IPC | Electron forks running the editor *and* the agent in Node |
| РЎР‚РЎСџРІР‚СћРЎвЂ **Code understanding** | Tree-sitter **code graph** Р Р†Р вЂљРІР‚Сњ the agent queries definitions, callers, callees, and blast radius instantly and exactly, with incremental updates on save | Embedding retrieval and blind grepping |
| РЎР‚РЎСџР’В§Р’В  **Memory** | Per-project **SQLite + FTS5 memory** with a knowledge graph (supersedes/extends/contradicts relations), rank fusion, and recency decay Р Р†Р вЂљРІР‚Сњ local, inspectable, survives restarts | Context resets every session, or lives in someone else's cloud |
| РЎР‚РЎСџР’В¤РІР‚вЂњ **Agent execution** | Native Rust turn loop, parallel read-only tool fan-out, up to 4 **parallel subagents** with a shared message board, prompt caching, context compaction | Single-threaded chat loops |
| РЎР‚РЎСџР Р‰РЎвЂ™ **Web research** | Built-in multi-query research engine: query expansion, cross-engine merge, corpus reranking, canonical-URL dedup, inline `[1]` citations | One-shot search wrapper, if anything |
| РЎР‚РЎСџРІР‚СњР Р‰ **Providers** | **Free models built in** (no key Р Р†Р вЂљРІР‚Сњ link Telegram once, 1 account = your own limits) **+ 35+ BYO-key presets** Р Р†Р вЂљРІР‚Сњ OpenAI, Anthropic, Google, DeepSeek, Groq, OpenRouter, Ollama, LM StudioР Р†Р вЂљР’В¦ Your keys go straight from the Rust client to the provider | Proxied billing and a fixed model list |
| РЎР‚РЎСџРІР‚СњРІР‚в„ў **Privacy** | **Zero listening ports** by default, stdio-first transports, all indexing/search/graph 100% local, SecretGuard redaction | Agent traffic routed through vendor cloud |
| РЎР‚РЎСџРІР‚в„ўРЎвЂ **Price** | Free, MIT, no telemetry surprises, no paywalled features | Subscription |

## РЎР‚РЎСџР’В¤РІР‚вЂњ The AI Agent

Four modes Р Р†Р вЂљРІР‚Сњ **Agent** (autonomous), **Automatic** (autonomous + self-planning, never stops at a plan), **Plan** (read-only, presents a structured plan for approval), **Ask** (explain-only) Р Р†Р вЂљРІР‚Сњ driving **45+ native tools**:

| Category | Tools & standout capabilities |
|---|---|
| РЎР‚РЎСџРІР‚СљРЎСљ Files | Pageable `Read`, exact `StrReplace` (CRLF-tolerant, idempotent), atomic multi-file `PatchEngine` with all-or-nothing rollback, structured `InspectFile` for xlsx/pdf/docx/sqlite/zip/ipynb/media |
| РЎР‚РЎСџРІР‚СњР вЂ№ Search & context | `Grep`, `Glob`, `SemanticSearch`, LSP-powered `SymbolContext`, `RelatedFiles`, `RepoMap`, ranked `ContextBudgeter` under an explicit char budget |
| РЎР‚РЎСџРІР‚СћРЎвЂ Code graph | `CodeGraphDefinition` / `Callers` / `Callees` / `Explain` (blast radius) / `Overview` (communities, god nodes) Р Р†Р вЂљРІР‚Сњ precomputed, instant, exact |
| РЎР‚РЎСџРІР‚вЂњРўС’ Terminal | `Shell` with background jobs and 3-tier safety classification (catastrophic commands are refused even when hidden inside `$()` or compound chains), live output mirrored to a read-only **"aspect .I" terminal tab** |
| РЎР‚РЎСџР Р‰РЎвЂ” Git | `GitContext`, `ReviewDiff` quality gate, `ImpactAnalysis`, `SecretGuard` secret scanner with auto-redaction |
| РЎР‚РЎСџР Р‰РЎвЂ™ Web | `WebFetch` (SSRF-guarded, IP-pinned DNS), `WebResearch` (deep mode: query expansion + link crawl, up to 15 sources), `MultiWebResearch` (up to 6 concurrent queries, 20 merged sources, per-source citations) |
| РЎР‚РЎСџР’В§Р’В  Memory & skills | `RecallMemory` / `RememberMemory` / `RelateMemories` (knowledge-graph edges with confidence), `ListSkills` / `UseSkill` for vetted SKILL.md playbooks |
| РЎР‚РЎСџРІР‚СћРІвЂћвЂ“ Browser | Full agent-browser automation: accessibility snapshots, actions by @ref, screenshots, isolated Chromium sessions |
| РЎР‚РЎСџРІР‚СњРІР‚вЂќ Extensibility | `McpManage` Р Р†Р вЂљРІР‚Сњ the agent installs and connects **MCP servers live**, mid-conversation, and uses their tools next round |
| РЎР‚РЎСџРІР‚ВРўС’ Orchestration | `Task` subagents (explorer / code-reviewer / test-runner / general) running in parallel with a topic-scoped `AgentMessage` board; read-only reviewers are permission-fenced by construction |
| РЎР‚РЎСџРІР‚С”РЎСџ Safety | `Checkpoint` create/diff/restore, per-turn edit review bar, `AskUser` with rendered HTML previews, `PresentPlan` structured planning |

Under the hood: the turn loop batches independent read-only calls concurrently, attaches Anthropic prompt-cache breakpoints so long sessions re-read the conversation from cache, and compacts context with a head+tail transcript window so neither the original task nor the latest state is lost. A global file-edit lock keeps parallel subagents from ever corrupting a write.

![AspectIDE AI chat Р Р†Р вЂљРІР‚Сњ autonomous coding agent with tool calls](docs/assets/aspect-ai-chat.png)

## РЎР‚РЎСџРІР‚СљРЎСљ Editor & Workbench

- **Monaco editor** with split groups, tab management, dirty-close guards, chord keybindings (`Ctrl+M Ctrl+\`), font zoom, minimap, ligatures.
- **LSP, fully wired**: hover, go-to-definition, references, rename, code actions, completion, signature help, inlay hints, semantic tokens, folding, formatting.
- **Zero-config language servers** Р Р†Р вЂљРІР‚Сњ TypeScript/JS, Python, Rust, Go, C/C++ (clangd), Lua, JSON, HTML/CSS, YAML, Bash auto-install into a managed directory, with Node/Rust/Python/Go runtimes provisioned on demand (checksums verified, live progress).
- **Integrated terminal** on a Rust PTY: multiple sessions, 10k scrollback, plus the read-only **aspect .I** tab mirroring the agent's shell in real time.
- **Workspace search & replace** Р Р†Р вЂљРІР‚Сњ regex, case, whole-word, include/exclude globs, bulk or per-hit replace across the project.
- **Git panel** Р Р†Р вЂљРІР‚Сњ branches (switch/create), stage/unstage/discard, commit, per-file diff viewer.
- **Structured previews** Р Р†Р вЂљРІР‚Сњ open xlsx, PDF, docx, SQLite, archives, Jupyter notebooks, images, audio, and video directly; Mermaid diagrams render live.
- **Explorer that scales** Р Р†Р вЂљРІР‚Сњ virtualized tree for monorepos, external drag-drop file import, inline create/rename, git status decorations.
- **Polish everywhere** Р Р†Р вЂљРІР‚Сњ command palette, keybinding profiles, system-font pickers, EN/RU localization (~1,800 strings each), voice input, vision attachments (smart WebP/PNG per provider), Codex-style update toast with live speed.

## РЎР‚РЎСџРІР‚СњРІР‚в„ў Trust & Security

Security posture is documented, enforced in code, and locked by regression tests:

- **Zero listening ports** by default Р Р†Р вЂљРІР‚Сњ UIР Р†РІР‚В РІР‚Сњengine is native IPC; LSP, DAP, and MCP ride child-process stdio. The policy ladder for anything new is written down in [local channels](docs/architecture/local-channels.md).
- **Approval gates with deny-beats-everything semantics** Р Р†Р вЂљРІР‚Сњ a declarative `allow/deny/ask` rule engine (deny > ask > allow) evaluated in trusted Rust, not the renderer. Deny rules fire even inside compound shell commands (`ls && rm -rf /`), and 13 dedicated regression tests lock the precedence order.
- **Workspace jail** Р Р†Р вЂљРІР‚Сњ every raw FS command resolves through a canonicalizing path guard; the agent cannot touch files outside the open workspace.
- **SSRF-proof web tools** Р Р†Р вЂљРІР‚Сњ the model can never disable the private-network guard; DNS is resolved once, screened (private ranges, loopback, CGNAT, IPv4-mapped-IPv6), and pinned.
- **SecretGuard** Р Р†Р вЂљРІР‚Сњ API keys, JWTs, PEM blocks, and connection strings are scanned and redacted in shell output, diffs, and summaries before they reach the chat or logs.
- **Battle-tested** Р Р†Р вЂљРІР‚Сњ an 86-agent adversarial audit produced 54 confirmed findings; every critical and high was fixed and regression-tested (see [SECURITY.md](SECURITY.md)).
- **Signed updates** Р Р†Р вЂљРІР‚Сњ updater artifacts are Ed25519-signed and verified against a pinned public key before applying.

## РЎР‚РЎСџРІР‚СљРЎвЂ Screenshots

| Agent browser automation settings | AI usage & cost tracking |
| --- | --- |
| ![AspectIDE settings Р Р†Р вЂљРІР‚Сњ agent browser automation with isolated Chromium sessions](docs/assets/aspect-settings.png) | ![AspectIDE AI usage Р Р†Р вЂљРІР‚Сњ requests, tokens, cost, and speed per model](docs/assets/aspect-ai-usage.png) |

| Agent workspace | Welcome screen |
| --- | --- |
| ![AspectIDE agent workspace Р Р†Р вЂљРІР‚Сњ parallel subagents and task progress](docs/assets/aspect-agent-workspace.png) | ![AspectIDE welcome screen](docs/assets/aspect-welcome.png) |

## РЎР‚РЎСџРІР‚СљРўС’ Installation

**Windows:** grab the installer from the [latest release](https://github.com/GofMan5/aspect-ide/releases/latest). After the first launch:

- `aspect .` works from any terminal Р Р†Р вЂљРІР‚Сњ AspectIDE registers an `aspect` command on your user PATH (no admin rights needed).
- Right-click any folder in Explorer Р Р†РІР‚В РІР‚в„ў **Open with AspectIDE**.
- Auto-update keeps you on the newest release.

**Build from source:**

```powershell
# Prerequisites: Rust stable, Node.js 22+, pnpm 10+, Tauri 2 platform deps
pnpm install
pnpm dev            # desktop dev build
pnpm tauri:build    # production bundle
```

`pnpm dev:web` runs a browser-only preview for UI iteration; production behavior requires the Tauri desktop runtime.

## РЎР‚РЎСџР РЏРІР‚С”Р С—РЎвЂР РЏ Architecture

One Tauri 2 shell + a Cargo workspace of focused crates (~40k lines of Rust) behind typed IPC Р Р†Р вЂљРІР‚Сњ every DTO derives `ts-rs`, so the TypeScript types are generated from the Rust structs, never hand-maintained. 190+ Tauri commands wire the engine to the workbench.

```text
apps/desktop          Tauri 2 shell, React workbench, Monaco, xterm.js

crates/aspect-core       shared DTOs, typed errors/events, global scan-concurrency budget
crates/aspect-workspace  workspace identity (stable 128-bit IDs) and lifecycle
crates/aspect-fs         parallel ignore-aware scanning, watching, mutations (200k-entry crawl cap)
crates/aspect-editor     document store and open/edit/save lifecycle
crates/aspect-search     parallel content search Р Р†Р вЂљРІР‚Сњ ranked, low-value paths deprioritized
crates/aspect-terminal   PTY service (portable-pty), smart shell resolution
crates/aspect-git        system-git plumbing: NUL-safe parsing, 30s hang-proof timeouts
crates/aspect-ssh        non-interactive OpenSSH driver (inherits your keys and config)
crates/aspect-settings   persisted settings, recents, keybinding profiles

crates/aspect-lsp        LSP client: framing, typed requests, zero-config server discovery
crates/aspect-dap        debug adapter discovery, DAP transport, launch.json parsing
crates/aspect-file-intel xlsx/pdf/docx/sqlite/archive/notebook extraction (zip-bomb guarded)
crates/aspect-codegraph  tree-sitter code graph: Rust/TS/Python, incremental updates,
                      on-disk parse cache, communities, centrality, cycle detection

crates/aspect-memory     per-project agent memory: SQLite + FTS5, rank fusion,
                      retention tiers, knowledge-graph relations
crates/aspect-skills     discoverable SKILL.md modules (project scope wins over global)
crates/aspect-research   web research core: query building, parsing, lexical reranking

crates/aspect-extensions WASM extension host on wasmtime (fuel budgets, hard sandboxing)
crates/aspect-bench      CI performance gate: list Р Р†РІР‚В°Р’В¤1.5s, search Р Р†РІР‚В°Р’В¤2s, events Р Р†РІР‚В°Р’В¤80ms
```

Deep dives: [Rust-first boundaries](docs/architecture/rust-first-boundaries.md) Р вЂ™Р’В· [Milestones](docs/architecture/milestones.md) Р вЂ™Р’В· [Local channels & security posture](docs/architecture/local-channels.md)

## Р Р†РЎС™РІР‚В¦ Quality Gates

Every push and every release tag runs the same bar Р Р†Р вЂљРІР‚Сњ releases take no shortcuts:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets   # -D warnings
cargo test --workspace                   # ~700 unit tests
cargo run -p aspect-bench -- --assert       # hard perf thresholds
pnpm typecheck && pnpm build             # + vitest, AI-pipeline verifies, bundle budgets
```

Bundle budgets keep the entry chunk under 300 KB and every eager chunk under 450 KB Р Р†Р вЂљРІР‚Сњ heavy libraries (Mermaid, graph layouts) load lazily, only when a preview actually opens.

## РЎР‚РЎСџРІР‚вЂќРЎвЂќ Roadmap

- **Inline AI ghost-text completion** Р Р†Р вЂљРІР‚Сњ the flagship next feature.
- Full DAP debug session execution from detected configurations.
- WASM extension host with a stable public contribution API.
- Agent-eval harness gating releases; unified multi-file changeset review.
- Cold-start and workspace-open latency budgets in CI.

## РЎР‚РЎСџРІР‚в„ўРІР‚вЂњ Support the Project

AspectIDE is free and open source, built by one developer shipping at ~1 release/day. If it saves you time, fuel the roadmap:

<!-- donations:start -->
| Platform | Link |
|---|---|
| РЎР‚РЎСџР вЂ№Р С“ **DonationAlerts** | [donationalerts.com/r/gofman5](https://www.donationalerts.com/r/gofman5) |

**Crypto:**

| Network | Address |
|---|---|
| Р Р†РІР‚С™РЎвЂ” Bitcoin (BTC) | `bc1qs5yshuvaxdw7cg9q8602ts9jvc3csh9cyc4q3q` |
| Р С›РЎвЂє Ethereum (ETH / ERC-20) | `0xbbD9c40FfaCDf344D23293887B613A870F6497FB` |
| Р Р†РІР‚С™Р’В® USDT (TRC-20) | `TUitn7ovNfC1N8HaryDecGc8RxsZDqPB9k` |
| Р Р†РІР‚вЂќР вЂ№ Solana (SOL) | `D3YBBhbrCiGtEyQY5rR658yZX98qQau5s6Ae7seFBKov` |
| РЎР‚РЎСџРІР‚в„ўР вЂ№ TON | `UQB7Sn0sWrByEwZaZXLDv99UiyqkQraZdFZ02f8RJ--qlmdN` |
| Р вЂўР С“ Litecoin (LTC) | `ltc1qgpcmcfc0nntj3nhg0x05m3fkgm6tsv3d5r8zqq` |
<!-- donations:end -->

Р Р†Р’В­РЎвЂ™ **Can't donate? Star the repo** Р Р†Р вЂљРІР‚Сњ it's the single biggest boost for an open-source project's visibility.

## РЎР‚РЎСџР’В¤РЎСљ Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Good contributions keep Rust as the product engine, preserve typed IPC, avoid placeholder UX, and include focused tests. New here? Start with docs, reproducible bugs, UI polish, Rust unit tests, or LSP/DAP adapters.

## РЎР‚РЎСџРІР‚в„ўР’В¬ Community

- Telegram: [t.me/aspect_ide](https://t.me/aspect_ide)
- Issues & feature requests: [GitHub Issues](https://github.com/GofMan5/aspect-ide/issues)

## РЎР‚РЎСџРІР‚СљРІР‚С› License

MIT License Р Р†Р вЂљРІР‚Сњ see [LICENSE](LICENSE) and [NOTICE](NOTICE).

---

<p align="center">
  <sub><b>AspectIDE</b> Р Р†Р вЂљРІР‚Сњ open-source AI code editor Р вЂ™Р’В· Cursor alternative Р вЂ™Р’В· Rust IDE Р вЂ™Р’В· Tauri desktop app Р вЂ™Р’В· autonomous coding agent Р вЂ™Р’В· AI pair programmer Р вЂ™Р’В· code graph Р вЂ™Р’В· local-first AI development</sub>
</p>
