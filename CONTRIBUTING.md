# Contributing

Thanks for looking. Read [ARCHITECTURE.md](ARCHITECTURE.md) and [HOW_IT_WORKS.md](HOW_IT_WORKS.md)
first; between them they cover how the proxy, the filter engine and the frontend fit together.

## Getting it running

```bash
pnpm install
pnpm dev          # frontend only, against Vite
pnpm tauri dev    # full desktop app, with the embedded proxy
```

You need Node 20+, pnpm 9, a Rust toolchain (see `rust-toolchain.toml`), and the
[Tauri v2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform. On
Linux, CI installs `libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev
libssl-dev libayatana-appindicator3-dev librsvg2-dev`; install the equivalents locally.

## Before you open a pull request

```bash
pnpm tsc --noEmit
pnpm build
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test --all
```

CI runs the frontend checks and the Rust checks as separate jobs; both must be green. There is
no `tauri build` gate in CI yet (see ROADMAP.md); please still build the desktop app locally
before a proxy or filter change, since a broken bundle only shows there.

## House style

**No em dashes.** Anywhere: prose, comments, docs, UI copy, commit messages. Commas, periods
and parentheses do the job.

**UI copy is short.** A card gets a sentence. The reasoning goes in a tooltip, a comment or
the pull request body, not on the screen.

**Be conservative in the filter parser.** Rules carrying `$options` or exception syntax
(`@@`) are currently skipped rather than risk overblocking; if you extend coverage, add corpus
tests and keep that bias.

**Never bypass the trust verifier.** `tls_verifier.rs` captures cert metadata without
short-circuiting webpki chain verification. A change that makes verification always succeed
is a security bug, not a feature.

## Adding things

A new filter source, reputation feed, or command should follow the shape already in
`list_loader.rs` / `reputation.rs` / `commands.rs`: fail soft (skip a bad line or a failed
fetch rather than aborting the whole load), and never write outside the platform `app_data`
directory documented in SECURITY.md.

## Commits

Subject lines are `(type) lowercase summary`, where type is `feat`, `fix`, `docs`, `refactor`,
`test` or `chore`. No trailing period, no body required.

No AI attribution trailers.

## Reporting a security issue

See [SECURITY.md](SECURITY.md). Please do not open a public issue for a vulnerability; email
the address listed there.
