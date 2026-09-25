# Design preview mode

BlueFlame's chrome cannot normally be seen outside the desktop app: in a
plain browser tab it crashes, because components like `TitleBar` call
`getCurrentWindow()` from `@tauri-apps/api/window`, which has no Tauri
runtime to talk to there. That makes it impossible to screenshot or
review the design without building and running the full Tauri app.

Design preview mode is a development-only way around that. It installs
the official Tauri mocks (`@tauri-apps/api/mocks`: `mockWindows` and
`mockIPC`) before the real chrome mounts, answering every `invoke()` and
event the UI uses with realistic fixture data instead of a live backend.
The whole shell then renders and behaves in a normal browser tab.

## Running it

```bash
pnpm dev:design      # vite dev server, opens /design.html
pnpm build:design     # production-mode build to dist-design/, for pnpm shots
```

`design.html` and `src/design-main.tsx` are a second, separate Vite
entry point. `index.html` (the real app) never references either, so
`pnpm build` (what CI runs, and what `tauri build` bundles) never
includes them. `pnpm build`'s `postbuild` hook
(`scripts/check-no-design-in-build.mjs`) asserts this by scanning
`dist/` for design-preview markers after every production build.

## What's mocked

`src/design/mockBackend.ts` implements `installDesignMocks()`, a single
`mockIPC` handler covering every Tauri command the frontend calls (tabs,
bookmarks, downloads, settings, trust, metrics, the Claude control
channel, and so on), backed by fixture data in `src/design/fixtures.ts`.
A few backend-driven events (`blueflame:claude-approval-request`,
`blueflame:claude-tab`, `blueflame:download-progress`,
`context-menu:payload`) are emitted directly so their UI shows up too.

## Screens

`design-main.tsx` reads `?screen=<id>` and drives the mounted `<App/>`
into that state the same way a person would (clicking the matching
sidebar button, opening a tab, and so on), since `App` keeps this as
internal state rather than reading it from the URL. The three popups
that are their own webview in the real app (menu, context menu, trust)
are reached the same way production does: `?panel=menu|context|trust`
with their existing query params.

The floating "design preview" panel in the bottom right (rendered by
`src/design/DesignSwitcher.tsx`) links to every screen and popup for
manual review. Pass `&hideswitcher=1` to hide it, which
`scripts/screenshots.mjs` does for its captures.

## Screenshots

```bash
pnpm shots
```

Builds the design preview, serves `dist-design/` locally, and renders
every screen with headless Edge at 1280x800 and 1920x1080 into
`docs/screenshots/before/`. BlueFlame has one dark palette only (no
`prefers-color-scheme` or `data-theme` split in `App.css`), so this only
captures dark; a later light theme should add a light pass here and in
this script's `SIZES`/naming.

The `before/` set is the baseline for the visual redesign this preview
mode exists to unblock; a later PR should add an `after/` set from the
same script for comparison.
