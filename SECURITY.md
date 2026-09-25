# Security

## Posture

BlueFlame is a local-only application. The only network traffic it makes is
the user's own browsing plus background fetches of the configured filter lists
(EasyPrivacy, EasyList) and reputation feeds (URLHaus by default). There is
no telemetry, no analytics, and no remote configuration channel.

User data (history, bookmarks, settings, trust history, downloads, filter
list cache, reputation cache, root CA, optional Tor state) lives entirely on
disk under the platform `app_data` directory. No data leaves the device.

## Threat model

In scope:

- Network trackers, fingerprinting scripts, ad pixels, and analytics beacons.
  Filtered at the proxy layer before they reach upstream.
- Known-malicious hosts. Cross-checked against the URLHaus reputation feed.
- Page-side red flags surfaced in the trust panel: cross-origin login forms,
  password fields on plain HTTP, mixed content, outdated JS libraries.
- Self-signed or short-lived upstream certs (recorded by `tls_verifier.rs`
  without bypassing webpki chain verification).

Out of scope:

- A user with local code execution on the device. The CA private key sits on
  disk in the app data dir; anyone with read access to that file can sign
  arbitrary leaves.
- State-level network adversaries. Embedded Tor and external SOCKS5 Tor are
  available, but BlueFlame is not a hardened anonymity browser (it is not
  Tor Browser).
- Web app vulnerabilities inside sites the user visits. The proxy filters and
  scores; it does not patch site-side flaws.
- Side-channels in the system WebView itself (WebView2, WebKitGTK, WKWebView,
  Android WebView). Engine-level CVEs are the platform's responsibility.

## Hardening measures (present in the tree)

- TLS uses rustls 0.23 with the `ring` crypto provider explicitly installed
  before any rustls code runs (`src-tauri/src/lib.rs`). Default features pull
  in `webpki-roots` for upstream chain verification.
- The custom `ServerCertVerifier` in `src-tauri/src/tls_verifier.rs` captures
  cert metadata for the trust panel without short-circuiting webpki. We never
  fabricate a successful verification.
- The MITM root CA is generated once on first run via `rcgen` and persisted
  to `<app_data>/ca/blueflame-ca.crt` + `blueflame-ca.key`. Across restarts,
  the existing key is reloaded so the user does not have to re-trust on every
  launch.
- Windows CA install runs `certutil -user -addstore Root`, the user-scope
  trust store. No admin elevation, no machine-wide trust.
- Filter-list and reputation-feed loaders skip bad lines, bad regex, and
  failed HTTP fetches without aborting the whole load (`list_loader.rs`,
  `reputation.rs`). One bad line cannot break filtering.
- The easylist parser is conservative: rules carrying `$options` are skipped
  rather than overblocking, and exception rules (`@@`) are skipped. This
  trades coverage for fewer false positives.
- SQLite LIKE search escapes `%`, `_`, `\` in user input (`storage.rs::escape_like`)
  to avoid wildcard injection.
- Release builds strip debug symbols and use `panic = "abort"` (`Cargo.toml`),
  reducing the surface for symbolicated exploit attempts and simplifying the
  crash domain.
- The hudsucker proxy logs noisy TLS-EOF and transient connect errors are
  suppressed by default (`hudsucker::proxy::internal=off`) so debug logs stay
  legible without hiding genuine errors at `RUST_LOG=hudsucker=debug`.
- The pre-warmed context-menu popup uses a per-launch token baked into the
  per-tab init script, so tab JS can authenticate when calling
  `submit_tab_event` (`src-tauri/src/context_menu.rs`).

## Claude control channel

Phase 1 (`src-tauri/src/control/`, `mcp-bridge/`) lets Claude Code drive
BlueFlame through a local pipe. What it can and can't do:

**Guaranteed:**

- The control pipe (`\\.\pipe\blueflame-control`) and the per-session token
  file (`<app_data>/control/token`) both carry an ACL restricted to the
  current Windows user only, built fresh from that user's own SID
  (`control/windows_impl/acl.rs`), not a well-known SID alias. Nothing else
  on the machine, including another user account or an elevated process
  running as a different principal, can open either one. `lock_down_file`
  (used to ACL the token) fails BlueFlame's boot rather than run with an
  unprotected token if the ACL can't be applied.
- The pipe rejects any connection whose first line isn't a byte-for-byte
  match of the current token, compared in constant time (`control/token.rs`).
  There is no remote-debugging port, ever - the DevTools calls the bridge's
  tools trigger run in-process against the tab's own WebView2 instance via
  `ICoreWebView2::CallDevToolsProtocolMethod`, never over a TCP port.
- Every tab Claude opens is InPrivate (`browser::browser_open_private_tab`):
  no cookies, storage, saved logins, or history from Daniel's regular
  browsing profile, and none of it is written back either.
- Claude can only ever act on tabs it opened itself
  (`control::tabs::ClaudeTabs`); a `navigate`/`click`/`type`/... call
  naming any other tab id, including a guessed one, is rejected before it
  reaches the webview.
- The first tool call against a new origin blocks on Daniel's answer in the
  UI (`control::dispatch::ensure_origin_approved`); a denial or a timeout
  fails the call. The check re-runs on every content-touching tool call
  against the tab's *current* URL, not just at `navigate` time, so an
  in-page link to a different origin can't slip past a stale approval.
- Cookies, local/session storage, and the future password locker are never
  exposed to any tool: `get_page_text` and `read_page` only ever return
  `document.body.innerText` and the accessibility tree, both scoped to
  visible page content.
- Every tool call is appended to `<app_data>/control/action-log.jsonl` and
  kept in a 200-entry ring buffer Daniel can open from the Debug view,
  whether it succeeded, was denied, or errored.

**Not guaranteed / accepted risk:**

- Page content returned to Claude (`get_page_text`, `read_page`, `find`,
  screenshots) is untrusted data. A malicious page can put attacker text
  into its own DOM; nothing in this channel tries to detect or sanitize a
  prompt-injection attempt inside that text, same as any other content an
  LLM is shown. Treat it as data, not instructions, the same way you would
  browser-extracted content anywhere else.
- The DevTools completion callback runs the dispatched closure on
  BlueFlame's own UI thread, blocking it until WebView2 answers or the
  bridge's 10-second timeout gives up on our side; a page that wedges its
  own JS event loop can still stall that one call on the UI thread for the
  duration of the underlying `wait_for_async_operation` pump. It does not
  crash BlueFlame, and it self-resolves once the tab is closed or the page
  recovers.
- A local process already running as Daniel that can read his app data
  directory can read the token file - this is the same trust boundary as
  every other local secret BlueFlame keeps there (the CA key, the session
  file). The control channel does not attempt to defend against a
  same-user attacker with filesystem access; see the CA private key
  caveat below, which is the pre-existing version of that same boundary.
- The approval prompt gates by origin, not by exact page: once Daniel
  allows `https://example.com`, every page and every tool call on that
  origin is allowed for the rest of the session (and future sessions,
  since the decision persists to `<app_data>/control/approvals.json`).

## Root CA caveat

Installing any root CA is a serious action. Anyone who obtains BlueFlame's
CA private key could impersonate HTTPS sites for that user account. The README
calls this out explicitly. Mitigations in code today:

- The CA is user-scoped on Windows (no admin / system-wide install).
- The CA key never leaves the local app data dir.
- BlueFlame ships with the source for the CA generation logic; users can
  audit `ca.rs` to confirm there is no upload path.

If a user stops using BlueFlame, they should remove the CA from the OS trust
store and delete `<app_data>/ca/`.

## Dependencies + supply chain

- `Cargo.toml` pins major versions for every direct dep.
- `package.json` uses caret ranges; `pnpm-lock.yaml` records resolved versions.
- Dependabot config lives at `.github/dependabot.yml` per the README.
- CI runs build + test (`.github/workflows/ci.yml` per the README).

## Vulnerability disclosure

This is a solo-maintained project. Report security issues privately to the
maintainer at `daniel.svs@outlook.com`. Please include:

- A reproducer or minimum failing test case.
- Affected version (`Cargo.toml` version field, currently `0.1.0`).
- Platform (Windows, Linux, macOS, Android) and OS version.
- Impact assessment.

Public disclosure of unfixed issues is discouraged. Coordinated disclosure on
a reasonable timeline is preferred. There is no bug bounty.

## License + redistribution

Dual-licensed under AGPL v3 (see `LICENSE`) and a commercial license (see
`COMMERCIAL.md`). Forks and SaaS deployments under AGPL must release source
under AGPL. Do not strip the trust panel, the filter list integrity checks,
or the no-telemetry promise; these are not optional features.
