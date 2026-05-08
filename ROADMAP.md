# Roadmap

This document captures gaps and planned work as observed in the current source
tree. It is descriptive (what the code says today), not aspirational.

## Platform coverage

- Windows: full support. WebView2 reads `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
  for proxy routing; `certutil -user -addstore Root` provides one-click,
  no-admin CA install (`src-tauri/src/lib.rs`, README.md "Trust the CA").
- Linux: proxy routing works via `http_proxy` / `https_proxy` env vars. CA trust
  install is manual (`update-ca-certificates`). Auto-install is "Not yet" per
  the README table.
- macOS: per-webview `proxy_url` is wired through `tauri-plugin` `macos-proxy`
  on macOS 14+. CA trust install is manual (`security add-trusted-cert`).
  Auto-install is "Not yet" per the README table.
- Android: WebView's native `shouldInterceptRequest` runs the same filter set,
  no proxy or CA trust step needed. The Gradle project lives at
  `src-tauri/gen/android/`.
- iOS: not built. README states it "will follow via `WKContentRuleList` once a
  Mac is available for the build". No iOS source is present in this repo.

## Trust + security signals

- `src-tauri/src/security.rs` describes the current state as "Phase 1": signals
  attributed by Referer header. Response headers, cert info, and body analysis
  are wired (response and cert are populated by `tls_verifier.rs` and the
  `handle_response` hook); script-behavior analysis is called out as "haven't
  wired yet".
- `CertSnapshot.subject_cn` and `sig_alg` are populated but the trust scorer
  does not read them yet (annotated as forward-compat for a future "cert
  details" UI row).
- Filter exception rules (`@@`) are explicitly skipped in `filter_parser.rs`
  and called out as a "future PR".
- Filter `$options` (e.g. `$third-party`, `$script`) are ignored. The parser
  prefers false negatives (missed ads) over false positives (broken searches);
  context-aware matching is a known limitation.

## Filter list management

- Lists hot-swap on refresh. The README mentions a future "Refresh button in
  Settings" alongside the existing `refresh_filter_lists` Tauri command.

## Reputation feeds

- Default feed set is just URLHaus (`reputation::default_feeds`). Adding more
  feeds is gated on permissive redistribution terms.

## Browser shell

- macOS CA trust auto-install (`ca_trust.rs` notes "deferred until we need it").
- A Settings UI for filter-list and reputation-feed refresh, to replace the
  current Tauri-command-only path.
- iOS shell beyond the Android mobile chrome.

## Performance + ergonomics

- Storage uses `LIKE` for search (`storage.rs`). A swap to FTS5 is called out
  in the module doc as a non-breaking follow-up.
- Filter compile splits across multiple `RegexSet`s to stay under the 128 MiB
  DFA guard. Further compaction (or a true ad-block engine) would unlock more
  list subscriptions.

## Things explicitly NOT planned in code today

- Sync across devices.
- Extension or WebExtension support.
- A telemetry pipeline. The README states "BlueFlame itself phones home to
  nothing" and there is no analytics code in the tree.

## Known open questions

- Whether to surface Tor circuit info in the UI (the bootstrap state is
  exposed as `running` / `ready` / `failed:<msg>` but not the circuit detail).
- How to handle CA rotation if a key is compromised (today the root CA is
  generated once and persisted; there is no rotation path).
- Whether to ship a richer default block set for offline first-run (today the
  built-in patterns cover the major offenders only).
