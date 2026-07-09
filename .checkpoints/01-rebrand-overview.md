# Checkpoint 01: Rebrand Overview — Lux IDE to AspectIDE

## Scope
Complete project rebranding from "Lux IDE" to "AspectIDE", affecting all layers.

## Changes made

### Brand identity
- Package name: `lux-ide` -> `aspect-ide`
- Organization: `@lux` -> `@aspect` (pnpm scope)
- Repository: `github.com/GofMan5/lux-ide` -> `github.com/nihmadev/AspectIDE`
- Author: `Lux IDE Team` -> `AspectIDE Team`
- Event prefix: `lux://` -> `aspect://`
- Logo: `lux-mark.svg` deleted, `aspect-mark.svg` created

### License
- Apache-2.0 -> MIT across all files (LICENSE, Cargo.toml, package.json, NOTICE)

### Files that changed (representative)
- `package.json`, `Cargo.toml`, `README.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`
- `apps/desktop/src-tauri/tauri.conf.json`, `apps/desktop/src-tauri/capabilities/default.json`
- `.github/workflows/ci.yml`, `.github/workflows/release.yml`
- `.github/scripts/*.mjs` (release notes, artifact collection, updater manifest)
- `docs/*.md` (PROJECT_REVIEW.md, TOP1_ROADMAP.md, architecture/*, distribution/*)
- `apps/desktop/index.html`, `apps/desktop/vite.config.ts`
- `apps/desktop/public/aspect-mark.svg` (new), `apps/desktop/public/lux-mark.svg` (deleted)
- `apps/desktop/src-tauri/installer/windows/aspect-installer.nsi` (new)
- `apps/desktop/src-tauri/installer/windows/lux-installer.nsi` (deleted)
- `.gitignore`, `AGENTS.md`
- `CHANGELOG.md` (deleted entirely)

### CI/CD pipeline
- Artifact names changed from `lux-*` to `aspect-*`
- Release metadata updated
- Certificate paths and package filters updated
- Bench crate references updated

### Documentation assets
- 7 old screenshots/logos deleted from `docs/assets/` and `etc/screenshots/`
- 2 new assets added: `docs/assets/icon.jpg`, `docs/assets/icon.png`
