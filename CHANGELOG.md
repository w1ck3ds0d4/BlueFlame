# Changelog

All notable changes to BlueFlame are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-05-18

First minor release. Significant feature work and a major dependency
port. Draft GitHub release ships an MSI + an NSIS installer for x64
Windows.

### Added

- **YouTube ad blocking** via injected content script. URL-level
  filtering can't catch YouTube ads because the ad video stream comes
  down the same `googlevideo.com` host as the actual video, so the
  only reliable handle is the DOM. Mirrors uBlock Origin's approach
  (#35, #36, #37):
  - Cosmetic hide list for masthead / in-feed / sidebar promos.
  - `MutationObserver` watching class-attribute changes on the player
    container, so the in-stream fast-forward fires the instant
    `.ad-showing` lands instead of polling on a timer.
  - Fast-forward mechanics: mute, seek to `duration - 0.1`, force
    `play()`. Same handle uBlock uses.
  - Interstitial / companion-ad handling for the static "Sponsored"
    cards YouTube shows at the tail of every ad break (`base44.com`,
    "Final Fantasy XIV" / etc. - hidden via the
    `.ytp-ad-action-interstitial` cosmetic rule plus the
    interstitial-specific Skip selectors).
  - Generic aria-label fallback that walks any visible button inside
    an ad container and clicks the first whose accessible name
    contains "skip", so future class renames don't break us.
- **"Block ads & trackers" master toggle in Settings (#34)** with a
  persisted backing store. The runtime AtomicBool is now seeded from
  the SQLite settings table at boot, so disabling ad-blocking via the
  toolbar or Settings actually survives a restart. Default `true` on
  fresh installs.
- New `--ok-mid: #aadd55` (lime / yellow-green) color token sitting
  between `--warn` (orange) and `--ok` (green) on the trust-score
  severity scale (#32).

### Changed

- **Trust-score chip now reads as severity** (#32, #33). The four
  tiers (`danger` / `suspect` / `ok` / `trusted`) form a continuous
  red -> orange -> lime -> green gradient. The mid-good 51-80 tier
  used to render in the brand cyan, which looked like the app accent
  instead of a severity signal.
- **Desktop -> Mobile -> Desktop flip is now reliable on Windows**.
  `set_mobile_ua` resizes the window before rebuilding tabs so every
  new webview ends up sized to the new content area on the first
  pass. `apply_window_size_for_mode` toggles `set_resizable` /
  `set_size` in an order that avoids `WM_NCCALCSIZE` clipping the
  desktop resize against the still-mobile client area.

### Fixed

- **Trust-score colors weren't painting at all** (#33). The four
  `.url-trust-<label>` rules were single-class selectors with the
  same specificity `(0,0,1,0)` as `.nav-icon`, which is declared
  lower in `App.css` and won the cascade - every chip rendered as a
  plain muted nav button regardless of severity. Compound
  `.url-trust.url-trust-<label>` selectors `(0,0,2,0)` now beat
  `.nav-icon` regardless of source order.

### Internals

- **`hudsucker 0.24` / `rcgen 0.14` API port (#30)**. The MITM proxy
  stopped compiling - `RcgenAuthority::new` now takes
  `(Issuer<'static, KeyPair>, cache_size, CryptoProvider)`, and the
  proxy builder typestate is `WantsCa -> with_ca -> WantsClient ->
  with_http_connector -> WantsHandlers -> with_client(ClientBuilder)`.
  Direct `rcgen` dep bumped from `0.13` to `0.14` so our types
  match hudsucker's transitive version (no more duplicated
  `KeyPair` / `Issuer` types in the dep tree).
- Tauri 2.11.0 -> 2.11.2, tauri-build 2.6.0 -> 2.6.1,
  tauri-plugin-opener 2.5.3 -> 2.5.4, sysinfo 0.38.4 -> 0.39.1
  (#26 - #29).

### Held back

- `tor-rtcompat 0.41 -> 0.42` (#25): incompatible with the current
  `arti-client 0.41` which transitively pins `tor-rtcompat ^0.41`.
  Cargo pulls both versions and the runtime trait bounds diverge.
  Will land once `arti-client` bumps to a version targeting
  `tor-rtcompat 0.42`.

## [0.1.0]

Initial repository contents. No published release - this is the
baseline the 0.2.0 changelog is measured against.
