# Local channel policy — zero listening ports

Lux ships with **no listening ports in the core**. This is a deliberate security
posture (port scanners find nothing to probe on a user's machine) and a product
claim worth protecting. Every new feature that needs a local communication
channel must follow the priority ladder below.

## Current state (audited 2026-07-03)

| Channel | Transport | Listening port |
|---|---|---|
| UI ↔ Rust | Native Tauri IPC (`tauri://` assets in production) | none |
| LSP servers | child process **stdio** | none |
| MCP servers | child process **stdio** (JSON-RPC) | none |
| DAP adapters (js-debug, debugpy) | child process **stdio** | none |
| DAP adapter (CodeLLDB) | TCP-only adapter: binds `127.0.0.1:0` (ephemeral), TOCTOU-shrunk reservation, killed with the session (`Drop` → `start_kill`, `kill_on_drop`) | only while a debug session runs |
| Integrated terminal | in-process PTY (`lux-terminal`) | none |
| Shell / agent tools | child processes over pipes | none |
| Updater / WebResearch / SSH | outbound connections only | none |
| agent-browser daemon + dashboard | external Vercel CLI; daemon binds loopback; dashboard is opt-in and its `start` is approval-gated (opens a local HTTP server) | only when the user starts it |

## The ladder — in order of preference

1. **stdio.** Child protocols (LSP, DAP, MCP, any future sidecar) speak over
   stdin/stdout. No socket exists, nothing to scan, the OS ties lifetime to the
   process. This is the default and requires justification to deviate from.
2. **Named pipes (Windows) / UNIX domain sockets.** When a stream channel must
   outlive a single child or connect independent processes. Not reachable by
   TCP port scans; access is controlled by filesystem ACLs. In Rust:
   `tokio::net::windows::named_pipe` / `tokio::net::UnixListener`.
3. **`127.0.0.1` + ephemeral port + token, as a last resort.** If a component
   only speaks TCP (some debug adapters, browser CDP):
   - bind `127.0.0.1:0` — never `0.0.0.0`, never a fixed port;
   - keep the listener's lifetime as short as the feature allows and tear it
     down with the owning session;
   - if Lux itself serves the socket, require a per-launch random token on
     every request (write `port + token` to a `0600` file under app data —
     the Claude Code IDE-bridge / Chrome DevTools pattern) and validate
     `Origin` for anything browser-reachable;
   - agent tools that would open such a listener must be approval-gated
     (see `BrowserDashboard`: `start` requires user consent).
4. **Never:** `0.0.0.0` binds, fixed well-known ports, unauthenticated local
   HTTP/WS servers, listeners that stay alive "just in case".

## Enforcement points in code

- `lux-dap/src/workspace.rs::sort_adapters_by_selection_priority` — when
  several adapters could serve a configuration, an installed **stdio** adapter
  is chosen over a TCP one (regression-tested).
- `lux-dap/src/session.rs` — TCP adapters get an ephemeral loopback port with a
  single-syscall bind/hand-off window; sessions kill the adapter process on
  disconnect/drop, which closes its listener.
- `ai_turn.rs` (`browser_is_side_effecting`) and
  `aiRuntimeBrowser.ts::browserDashboardTool` — `BrowserDashboard start/stop`
  require explicit user approval because `start` opens a local HTTP server.

When reviewing a PR that adds any `TcpListener::bind`, HTTP server, or
WebSocket server: require a written justification against this ladder.
