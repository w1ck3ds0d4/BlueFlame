# Roadmap

**Status:** release: path to v1.0.0. **Last reviewed:** 2026-09-24.

BlueFlame is a privacy-first desktop browser shell with an embedded transparent MITM proxy
that filters every request and response, Tor integration via `arti-client`, and cross-platform
reach (Windows, Linux, macOS, Android, with iOS to follow). "Done" for v1 is CI gates and a
test suite on top of the feature set that already shipped in v0.2.0, then a tagged v1.0.0
release.

> How this file is used: Claude Project threads build the first unticked item under **Now**, one item per branch and pull request, and tick it in that same PR as `- [x] ... (#PR)`. Daniel owns the order and the lists; threads never add to Now, Next or Later themselves, they propose under **Ideas**.

## Now (path to v1.0.0)

- [ ] **Add the missing CI gates**: `cargo fmt --check` and `cargo clippy -D warnings` already
      run; add `cargo test`, `pnpm tsc --noEmit` (already present), `pnpm build` (already
      present), and a `tauri build` job so a broken bundle fails CI rather than being found by
      hand. Done when: `.github/workflows/ci.yml` runs a full `tauri build` on at least Windows.
- [ ] **Write the test suite**: unit tests for the filter parser (`filter_parser.rs`), storage
      (`storage.rs` LIKE search), and MITM hooks (`proxy.rs`, `tls_verifier.rs`). Done when: `cargo
test --all` covers all three and runs in CI.
- [ ] **Clear the 8 open Dependabot alerts** and fix the failing glib/rand/@babel bump PRs.
      Done when: the alert count is at or near zero and CI is green on the dependency PRs.
- [ ] **Tag v1.0.0 and cut a GitHub release** once CI and tests are green. Done when: the
      `v1.0.0` tag exists and a GitHub release is published with release notes.

## Next

- [ ] **macOS CA auto-install**: wrap `security add-trusted-cert -d -r trustRoot -k <user
keychain>` so trust is one click, matching the Windows flow. Done when: a fresh macOS
      install can trust the CA without a manual terminal command.
- [ ] **Linux CA auto-install**: detect distro family and wrap the correct
      `update-ca-certificates` or `trust anchor` invocation. Done when: a fresh Linux install can
      trust the CA without a manual command.

## Later

- Filter exception rule (`@@`) parsing and the most-impactful `$options` (`$third-party`,
  `$script`, `$image`, `$domain=`)
- Settings UI buttons for filter list and reputation feed refresh
- Signed installers per platform (MSI, NSIS, APK, DMG, AppImage) attached to the release
- iOS shell via `WKContentRuleList`, once a Mac is available for the build
- FTS5 history search (replacing the current `LIKE` search)
- Surfacing Tor circuit details in the UI
- CA rotation flow if the root key is compromised

## Ideas

(empty to start; threads add proposals here)

## Out of scope for v1

- Sync across devices (no telemetry, no cloud, by design)
- Browser extension / WebExtension support
- Any telemetry pipeline or analytics
- Replacing the regex-based filter engine with a full ad-block engine

## Done

- [x] v0.2.0 (2026-05-18): YouTube ad blocking, ad/tracker master toggle, trust-score chip
      severity fix, hudsucker 0.24 / rcgen 0.14 port
- [x] Windows browser shell with MITM proxy + WebView2 routing, one-click no-admin CA install
- [x] Tor SOCKS connector via `arti-client`
- [x] Filter engine with EasyPrivacy + EasyList hot-reload
- [x] Bookmarks (Netscape HTML import), history, downloads, search bar
- [x] Android shell with `shouldInterceptRequest` filter path
- [x] Privacy dashboard with request / block counts
