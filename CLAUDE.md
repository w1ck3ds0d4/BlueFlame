# BlueFlame

Privacy-first browser shell. Tauri v2 (Rust) backend runs an embedded MITM filter proxy and
owns per-tab WebView instances; a Preact + TypeScript + Vite frontend renders the chrome.
Public, AGPL-3.0-or-later, commercial license offered (see COMMERCIAL.md).

## Commands

```bash
pnpm install
pnpm dev             # vite dev server (frontend only)
pnpm build           # tsc && vite build
pnpm preview         # vite preview
pnpm tauri dev       # full desktop app
```

Rust side (`src-tauri/`):

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all
```

CI (`.github/workflows/ci.yml`) runs a `frontend` job (ubuntu: `pnpm tsc --noEmit`, `pnpm
build`) and a `rust` job (ubuntu, with webkit/gtk system deps: `cargo fmt --check`, `cargo
clippy -D warnings`, `cargo test --all`). No `tauri build` or test suite exists in CI yet;
see ROADMAP.md.

## Layout

| Path                                 | What it is                                                                                                                                                                                                                                                                                        |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/`                               | Preact/TS frontend: `App.tsx`, `components/` (chrome, dashboard, settings, overlays)                                                                                                                                                                                                              |
| `src-tauri/src/`                     | Rust backend: `proxy.rs` (MITM core), `ca.rs`/`ca_trust.rs`/`trust.rs` (root CA + trust scoring), `filter_parser.rs`/`list_loader.rs` (filter engine), `reputation.rs` (URLHaus feed), `storage.rs` (SQLite), `embedded_tor.rs`/`socks_connector.rs` (Tor), `commands.rs` (Tauri command surface) |
| `ARCHITECTURE.md`, `HOW_IT_WORKS.md` | Component breakdown and plain-words walkthrough                                                                                                                                                                                                                                                   |
| `ROADMAP.md`                         | Path to v1.0.0                                                                                                                                                                                                                                                                                    |

## Conventions

- Commits: `(type) lowercase summary` (feat, fix, chore, docs, refactor, revert, test), no
  trailing period, no body.
- No em dashes or en dashes anywhere: code, copy, comments, commits, PRs. ASCII hyphens only.
- Branch + PR per change; Daniel reviews and merges.
- Never print, log or commit a key, token, password, or the CA private key.
- No AI attribution trailers in commits or PR text.

## Do not read

`node_modules/`, `dist/`, `src-tauri/target/`, `pnpm-lock.yaml`, `src-tauri/gen/android/`
(generated Gradle project).
