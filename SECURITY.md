# Security Policy

Lux IDE is early alpha. Security reports are still welcome, especially issues involving workspace file access, command execution, update/install behavior, extension loading, or AI tools that can mutate files.

## Supported Versions

Only the current `main` branch is supported until the first public stable release.

## Reporting a Vulnerability

Please do not open a public issue for a suspected vulnerability.

Report privately through GitHub Security Advisories after the repository is published. If advisories are not enabled yet, contact the maintainers through the project's published maintainer channel.

Include:

- affected version or commit
- operating system
- reproduction steps
- expected impact
- logs, screenshots, or proof of concept when safe to share

## Scope

High-priority areas:

- arbitrary file read/write outside the selected workspace
- command execution without user intent or approval
- unsafe updater or installer behavior
- extension manifest loading that can execute untrusted code
- AI tool calls that bypass approval or workspace boundaries
- credential leakage through logs, prompts, telemetry, or generated context
- any new listening socket that violates the [local channel policy](docs/architecture/local-channels.md) (the core ships with zero listening ports)

Lux IDE must never delete user project directories during uninstall, cleanup, update, or cache maintenance.
