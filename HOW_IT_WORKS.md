# How It Works

BlueFlame is a privacy-first browser shell. It looks and behaves like a
regular browser, but every page you visit goes through a local filter that
strips trackers, ad pixels, and analytics beacons before the page renders.
Nothing about your browsing leaves the machine except the requests you
actually want to make.

## What it does

- Blocks trackers and ads on the network. Out of the box, BlueFlame ships
  with a built-in baseline plus EasyPrivacy and EasyList subscriptions.
  Doubleclick, Google Analytics, GTM, Hotjar, Mixpanel, Segment, Amplitude,
  Facebook pixels, and the long tail are blocked.
- Scores per-host trust. The trust panel summarizes signals from the proxy
  (third-party requests, blocked counts, mixed content), the upstream TLS
  cert (issuer, validity window, self-signed flag), and the page itself
  (cross-origin login forms, passwords on plain HTTP, outdated JS libraries).
- Cross-checks hosts against the URLHaus malware feed.
- Optionally routes everything through Tor. An embedded Tor client is built
  in (no separate daemon needed). External SOCKS5 Tor is also supported.
- Stores history, bookmarks, settings, and downloads locally in SQLite.
- Does not phone home. There is no telemetry, no analytics, no remote config.

## Key features

Browser shell:

- Tabs, tab switcher, middle-click open in new tab.
- URL bar with autocomplete suggestions from history and bookmarks.
- Bookmarks with a slash-path folder tree, full-page browser, Netscape
  `bookmarks.html` import, JSON export of all settings and bookmarks.
- Downloads list (Content-Disposition aware).
- Find-in-page (`Ctrl+F`).
- Themed right-click context menu and dropdown menus.
- Resource monitor with ASCII sparklines for CPU, memory, and network.
- Search bar with configurable engines, including Ahmia for `.onion`
  indexing.
- New-tab "personal index" page showing recent and most-visited sites.
- Mobile (Android) chrome with the same filtering, no proxy / CA setup
  needed on Android.

Privacy:

- Embedded MITM proxy on `127.0.0.1:18080`, generated self-signed root CA,
  filter list hot-reload, real-time block log, privacy dashboard with
  request total, blocked count, and bytes saved.
- Toggle filters on or off without restarting the proxy.
- Private tabs isolated from the main session store.
- Embedded Tor with bootstrap status (`running`, `ready`, `failed:<msg>`)
  visible in the UI.

## How to use it

### First run

1. Launch BlueFlame. The proxy auto-starts and the system WebView is
   configured to route through it.
2. A first-run modal asks to install the BlueFlame root CA. On Windows this
   is one click (no admin). On macOS and Linux the modal points you at the
   manual `security add-trusted-cert` or `update-ca-certificates` command.
3. Without the CA installed, HTTPS sites show cert warnings. Once trusted,
   browsing is seamless.
4. The dashboard starts updating every two seconds with totals.

### Browsing

- Type a URL or query into the URL bar; matches from your history and
  bookmarks autocomplete inline.
- `Ctrl+T` opens a tab, `Ctrl+W` closes one, `Ctrl+1`-`Ctrl+9` jumps to a
  numbered tab, `Ctrl+Tab` cycles, `Ctrl+L` focuses the URL bar, `Ctrl+F`
  opens find-in-page, `Ctrl+R` reloads, `Ctrl+D` bookmarks, `F12` opens the
  in-app debug view. These also work from inside tab webviews (the chrome
  catches them and dispatches).
- Middle-click any link to open it in a new BlueFlame tab.
- Right-click for the themed context menu (open, copy link, view source,
  inspect). Long-press does the same on mobile.

### Privacy dashboard

- Live counters for total requests, blocked requests, and estimated bytes
  saved.
- A scrollable per-request block log showing exactly what the proxy stripped.
- A trust panel that opens for the active host with category-scored signals.

### Filter lists

- The Settings view shows currently subscribed lists.
- The first launch compiles the built-in baseline immediately, then loads
  any cached lists from `<app_data>/filter-cache/`, then fetches fresh
  copies in the background.
- A bad list line will not break filtering; failures are logged and skipped.
- The `refresh_filter_lists` Tauri command (later, a Refresh button) can
  re-pull on demand.

### Tor

- Open Settings, enable embedded Tor (default) or point to an external
  SOCKS5 endpoint. Restart the app for the change to take effect.
- The dashboard shows the Tor bootstrap state. First-run bootstrap can take
  up to 30 seconds.

### Bookmarks and history

- Bookmark the current page from the URL bar or `Ctrl+D`.
- Organize bookmarks into folders via slash paths (`Work/Dev`, `Reading`).
- Import a `bookmarks.html` from another browser via Settings.
- Export your full settings + bookmarks JSON for backup.
- The personal index (new-tab page) surfaces recent and most-visited sites
  from your local history.

### Downloads

- Files served with `Content-Disposition: attachment` are saved to the OS
  Downloads folder and listed in the Downloads view, where you can open
  them or reveal them in the file manager.

## Where things live on disk

- `<app_data>/ca/blueflame-ca.crt` and `blueflame-ca.key`: root CA.
- `<app_data>/personal.sqlite`: history, bookmarks, settings, trust history.
- `<app_data>/filter-cache/`: cached filter list bodies.
- `<app_data>/tor/`: embedded Tor state and cache (only if enabled).
- `<app_data>/session.json`: last session's tabs (restored on launch).
