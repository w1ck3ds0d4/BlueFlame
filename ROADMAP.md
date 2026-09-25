# Roadmap

**Status:** release: path to v1.0.0 as Daniel's daily browser. **Last reviewed:** 2026-09-25.

BlueFlame is a privacy-first desktop browser shell with an embedded transparent MITM proxy
that filters every request and response, Tor integration via `arti-client`, and cross-platform
reach (Windows, Linux, macOS, Android, with iOS to follow). "Done" for v1 is CI gates and a
test suite on top of the feature set that already shipped in v0.2.0, then a tagged v1.0.0
release.

> How this file is used: Claude Project threads build the first unticked item under **Now**, one item per branch and pull request, and tick it in that same PR as `- [x] ... (#PR)`. Daniel owns the order and the lists; threads never add to Now, Next or Later themselves, they propose under **Ideas**.

## Now (daily driver and Claude control, decided by Daniel 2026-09-25)

BlueFlame replaces Opera GX as Daniel's daily browser, and Claude can drive it the way it drives
Chrome. The core browser and the Claude channel are built in parallel.

- [ ] **CA private key in the TPM**: `ca.rs` writes the root key as a plain PEM file, so anything
      running as Daniel can copy it and later intercept all his TLS. Create the key inside the TPM
      (Microsoft Platform Crypto Provider, non-exportable), sign leaf certificates through it with a
      per-host cache, migrate an existing install (new root trusted, old root removed, old key file
      deleted), and fall back to a non-exportable software key only where no TPM exists. Done when:
      no CA private key exists on disk and the one-click, no-admin trust flow still works.
- [ ] **Password locker design**: a built-in locker that cannot be bulk-stolen: its key is held by
      the TPM (not bound to firmware measurements, so a BIOS update does not destroy it), every unlock
      needs Windows Hello, autofill fills only the exact saved origin, page scripts and the Claude
      channel can never read it, and an offline recovery code survives a TPM reset. Done when: a
      design doc in `docs/` is signed off by Daniel. Doc drafted at `docs/password-locker.md`
      (#104), still awaiting sign off, so this item stays unticked until Daniel signs off.
- [ ] **Claude control channel, phase 1**: a control server inside BlueFlame on a local named pipe
      with a per-session token, driven through a small MCP bridge, exposing a limited tool set (tabs,
      navigate, read page text and structure, click, type, scroll, screenshot) implemented in-process
      (WebView2's own DevTools calls, never an open remote-debugging port). Claude works in a separate
      profile with none of Daniel's logins, a banner shows while Claude drives a tab, each new site
      needs approval, every action is logged, and nothing can read cookies or the password locker.
      Built in #107: the pipe, its ACL, the token handshake, tool dispatch, and the mcp-bridge are
      implemented, with unit tests plus a real named-pipe/ACL/token-handshake/bridge end-to-end test
      (`cargo test --all -- --ignored real_pipe_end_to_end_through_the_bridge` in `src-tauri`, needs
      `pnpm install` in `mcp-bridge` first). No automated test drives a real WebView2 tab or a real
      Claude Code session - both need a live window on Daniel's own machine, which an unattended test
      here has no safe way to tell apart from his real daily-driver BlueFlame instance (same pipe name,
      proxy port, and app-data-derived profile/CA). Nothing further needs building for phase 1. Done
      when: Claude Code can open, read and operate a real page in BlueFlame through the bridge - Daniel
      adds the bridge per README.md and runs that smoke test himself, then ticks this line.
- [x] **Streaming downloads**: `downloads.rs` buffers whole files in memory and refuses anything over
      500 MB. Done when: downloads stream to disk with progress and no size cap. (#105)
- [ ] **Adopt the WickIT design system (flame accent)**: Daniel asked on 2026-09-25 for BlueFlame to share WickIT's design system in the Console register, with the flame as its accent and mark. The browser chrome (tab strip, address bar, panels, privacy dashboard, downloads, settings, the Claude approval bar) moves onto the kit's tokens (`tokens.css` copied verbatim from WickIT HQ's `Design System/Kits/` with a drift check), fonts and components; web pages themselves are never restyled. Placed before the remaining UI work so new screens are built in the final look. Done when: no colour, spacing or radius in `src/` bypasses a token, and the chrome has been checked against the kit at laptop width in dark and light.
- [ ] **Default browser on Windows**: register BlueFlame for http, https and .html so Windows lists
      it. Done when: BlueFlame can be picked in Settings > Default apps.
- [ ] **Multiple windows**: every tab is a child of the single `main` window today. Done when: a tab
      can move to a new window and session restore brings the windows back.
- [ ] **Update mechanism**: Tauri updater with signed update bundles; the signing key stays with
      Daniel. Done when: an installed copy updates itself from a signed release.
- [ ] **Opera GX import**: bookmarks and history, never passwords. Done when: one action imports
      both from Opera GX's profile.
- [ ] **Windows build and installer in CI**: CI builds only on Linux today. Done when: CI runs a full
      `tauri build` on Windows and publishes a signed installer on release.
- [ ] **Clear the 8 open Dependabot alerts**. Done when: the alert count is at or near zero.
- [ ] **Password locker build**, after the design is signed off.
- [ ] **Claude control channel, phase 2**: package the bridge as a one-click Claude desktop
      extension. Done when: the Claude desktop app can add BlueFlame's tools without editing config.
- [ ] **Usability and accessibility pass**: Daniel asked on 2026-09-25 for his products to look better and be more user friendly and professional. Audit every screen with the nielsen-heuristics-audit, wcag-2.2-aa and web-design-guidelines skills, rank the findings, and fix the top ones (keyboard use, focus, contrast, empty and error states, copy). Done when: the audit is in `docs/` and every high-severity finding is fixed, with before and after screenshots at phone and laptop width.
- [ ] **Tag v1.0.0 and cut a GitHub release** once the items above are done.

## Next

- [ ] **macOS CA auto-install**: wrap `security add-trusted-cert -d -r trustRoot -k <user
keychain>` so trust is one click, matching the Windows flow. Done when: a fresh macOS
      install can trust the CA without a manual terminal command.
- [ ] **Linux CA auto-install**: detect distro family and wrap the correct
      `update-ca-certificates` or `trust anchor` invocation. Done when: a fresh Linux install can
      trust the CA without a manual command.
### Linux parity (after Windows v1.0.0, decided by Daniel 2026-09-25)

BlueFlame already builds and runs on Linux (WebKitGTK; CI builds and tests on Ubuntu; .deb, .rpm and AppImage bundles), but the security and Claude features are Windows-only today.

- [ ] **CA key protected on Linux**: keep the root key in the TPM through tpm2-tss where one exists, otherwise in the kernel keyring, never as a plain file. Done when: no CA private key exists on disk on Linux either.
- [ ] **Claude control channel on Linux**: the same token-gated protocol over a Unix socket restricted to the user (mode 0600), with the same InPrivate tabs, approval gate and action log. Done when: the MCP bridge drives a Linux BlueFlame window.
- [ ] **Tab events on Linux**: carry right-click, middle-click and keys over WebKitGTK's private script message channel with the same marker and token as the Windows fix, instead of Tauri IPC (blocked for web pages). Done when: the BlueFlame menu opens on any web page on Linux.
- [ ] **Password locker unlock on Linux**: TPM or keyring-held key with a user-presence check (fprintd or the system password) in place of Windows Hello, same threat model as docs/password-locker.md. Done when: the locker works on Linux with the same guarantees stated honestly.

- [ ] **Proxy bypass list** for sites that reject intercepted certificates (some banking apps).

## Later

- Filter exception rule (`@@`) parsing and the most-impactful `$options` (`$third-party`,
  `$script`, `$image`, `$domain=`)
- Settings UI buttons for filter list and reputation feed refresh
- Signed installers for the other platforms (APK, DMG, AppImage) attached to the release
- iOS shell via `WKContentRuleList`, once a Mac is available for the build
- FTS5 history search (replacing the current `LIKE` search)
- Surfacing Tor circuit details in the UI
- CA rotation flow if the root key is compromised

## Ideas

(empty to start; threads add proposals here)

## Out of scope for v1

- Sync across devices (no telemetry, no cloud, by design)
- Browser extension / WebExtension support (the password locker is built in instead)
- A remote-debugging port as a shipped feature (the Claude channel is in-process and token-gated)
- Any telemetry pipeline or analytics
- Replacing the regex-based filter engine with a full ad-block engine

## Done

- [x] Test suite: filter parser, storage, TLS verifier, proxy and more, run by `cargo test --all` in CI
- [x] v0.2.0 (2026-05-18): YouTube ad blocking, ad/tracker master toggle, trust-score chip
      severity fix, hudsucker 0.24 / rcgen 0.14 port
- [x] Windows browser shell with MITM proxy + WebView2 routing, one-click no-admin CA install
- [x] Tor SOCKS connector via `arti-client`
- [x] Filter engine with EasyPrivacy + EasyList hot-reload
- [x] Bookmarks (Netscape HTML import), history, downloads, search bar
- [x] Android shell with `shouldInterceptRequest` filter path
- [x] Privacy dashboard with request / block counts
