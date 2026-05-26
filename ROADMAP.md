# BlueFlame v1 Roadmap

## What v1 is

A privacy-first desktop browser shell with an embedded transparent MITM proxy
that filters every request and response, Tor integration via `arti-client`,
and cross-platform reach (Windows, Linux, macOS, Android with iOS to follow).
Local SQLite stores history, bookmarks, downloads and settings; no telemetry,
no cloud, no extensions.

## Current state

v0.2.0 is shipped on Windows with full WebView2 proxy routing, one-click no-admin
CA install, EasyList/EasyPrivacy hot-reloadable filter store, Tor SOCKS bootstrap,
mobile Android UI via `shouldInterceptRequest`, and a privacy dashboard. macOS
and Linux work but CA install is manual; iOS is unbuilt. Filter exception rules
(`@@`) and option matching (`$third-party`, `$script`) are deliberately skipped.
No CI or test suite exists in tree.

## v1 acceptance criteria

- [x] Windows browser shell with MITM proxy + WebView2 routing
- [x] One-click no-admin CA install on Windows
- [x] Tor SOCKS connector via `arti-client` (bootstrap exposed as state)
- [x] Filter engine with EasyPrivacy + EasyList hot-reload
- [x] Bookmarks (Netscape HTML import), history, downloads, search bar
- [x] Android shell with `shouldInterceptRequest` filter path
- [x] Privacy dashboard with request / block counts
- [ ] CI gates (cargo fmt + clippy + cargo test + tsc + vite build + tauri build)
- [ ] Test suite covering filter parser, storage, MITM hooks
- [ ] macOS CA auto-install (`security add-trusted-cert` wrapped, no manual step)
- [ ] Linux CA auto-install (`update-ca-certificates` wrapped + verified)
- [ ] Filter exception rule (`@@`) parsing
- [ ] Settings UI button for filter list and reputation feed refresh
- [ ] Documented release: signed MSI + NSIS + Android APK + macOS DMG + Linux AppImage

## Milestones to v1

### M1. CI and test scaffolding (M)

- [ ] Add `.github/workflows/ci.yml` running `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, frontend `tsc`, `vite build`, `tauri build` on Windows runner
- [ ] Wire `.github/workflows/security.yml` to the SecureCheck reusable workflow
- [ ] Seed unit tests for `filter_parser`, `storage.rs` (LIKE search), `tls_verifier`
- [ ] Add a smoke integration test that loads a static page and asserts a blocked request count

**Acceptance:** every PR has a hard gate; main is green.

### M2. Platform CA install completeness (M)

- [ ] macOS: implement `ca_trust::install_macos()` using `security add-trusted-cert -d -r trustRoot -k <user keychain>`
- [ ] Linux: detect distro family (Debian / Fedora / Arch) and wrap the correct `update-ca-certificates` or `trust anchor` invocation
- [ ] Verify trust on each platform with a real HTTPS request before reporting success
- [ ] Update README CA table from "Not yet" to "Yes" on Linux + macOS

**Acceptance:** running BlueFlame on a fresh macOS or Linux box gives a one-click "Trust CA" path identical to Windows.

### M3. Filter parser hardening (M)

- [ ] Parse and apply filter exception rules (`@@||` and friends)
- [ ] Parse the most-impactful `$options` (`$third-party`, `$script`, `$image`, `$domain=`)
- [ ] Add corpus tests covering EasyList edge cases (regex, anchors, exceptions, domain options)
- [ ] Document the parser's coverage matrix in `src-tauri/src/filter_parser.rs`

**Acceptance:** YouTube ads + a curated test set of 20 sites pass through cleanly without false positives in search results.

### M4. Settings UI completeness (S)

- [ ] Add a "Refresh filter lists" button bound to the existing `refresh_filter_lists` Tauri command
- [ ] Add a "Refresh reputation feeds" button (URLHaus today; placeholder for added feeds)
- [ ] Show last-refresh timestamp + entry count per list

**Acceptance:** users can refresh lists from the UI without invoking `tauri::invoke` manually.

### M5. Release packaging + manual smoke (S/M)

- [ ] Build MSI + NSIS on Windows CI runner
- [ ] Build APK on Android CI runner
- [ ] Build DMG on macOS CI runner once code-signing path is ready
- [ ] Build AppImage on Linux CI runner
- [ ] Manual smoke matrix: open 5 sites, view privacy dashboard, refresh filters, route via Tor, install CA from in-app flow
- [ ] Tag `v1.0.0` after the matrix is clean

**Acceptance:** documented per-platform smoke checklist; one signed installer per OS attached to the v1.0.0 GitHub release.

## Beyond v1 (post-1.0 polish)

- iOS shell via `WKContentRuleList`
- FTS5 history search (swap `LIKE`)
- Surfacing Tor circuit details in the UI
- CA rotation flow if the root key is compromised
- Richer default reputation feed set (gated on redistribution terms)

## Out of scope for v1

- Sync across devices (no telemetry, no cloud - by design)
- Browser extension / WebExtension support
- Any telemetry pipeline or analytics
- Replacing the regex-based filter engine with a full ad-block engine (compaction work is post-v1)
