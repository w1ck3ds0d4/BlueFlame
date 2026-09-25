# Design audit: WickIT design system adoption

Method: Nielsen's ten heuristics plus the web design guidelines skill, run against the
`before` screenshots in `docs/screenshots/before/` and the component source in `src/`. Ranked
most severe first. Each finding says what was done about it in this PR.

Five items below (marked "reported by Daniel") were not found in this pass; he flagged them
directly from screenshots of the running app after the first draft, and they were fixed in the
same branch before this PR was opened.

## Top severity

1. **Status chips read as an alarm for normal operation (reported by Daniel).** The block
   counter next to the address bar drew a heavy coloured border and an unclear glyph for
   "trackers blocked on this page", identical in weight to a real warning, and sat as a second
   disconnected box next to the site-scan chip. Blocking is the working state, not a fault
   (Nielsen H1 visibility of system status conveying the wrong severity; H4 consistency, since
   colour now meant two different things). Fixed: the two chips are wrapped in one bordered
   `.status-group`, the block counter is neutral by default (`--text-secondary` / `--text-faint`,
   no accent or semantic colour), it carries a real icon (`ShieldBan`) instead of a `▒` glyph, and
   the accessible name spells out "N trackers blocked on this site". The site-scan chip keeps its
   real severity colours (`--ok` / `--ok-mid` / `--warn` / `--danger`) since that number is an
   actual finding.
2. **Tab strip showed a native scrollbar and cramped, abruptly clipped titles (reported by
   Daniel).** `.tab` had no minimum width and the strip used `overflow-x: auto`, so as tabs piled
   up the browser's own scrollbar appeared under the strip (H8 aesthetic and minimalist design;
   H4, since no other part of the chrome shows a raw OS scrollbar). Fixed: tabs share the row's
   width and shrink together down to a 96px floor; past that floor the rest move into an overflow
   menu instead of a scrollbar. The active tab now gets the system's action-colour underline
   (indigo) instead of an ASCII `>` marker in the flame accent, and the title fades at its clipped
   edge (a mask gradient) instead of a hard cut.
3. **Bookmarks bar rendered items as bracketed text (reported by Daniel).** `.bookmark-chip`
   used CSS `::before`/`::after` to print literal `[` and `]` around every label, and the actual
   bookmark icon was `display: none` (H2 match with the real world: real bookmark bars use an
   icon plus a label, never brackets). Fixed: proper chips with a `Bookmark` icon (root bookmarks)
   or a `Folder` icon plus a `ChevronRight` (folders), a bordered hover/focus state, no bracket
   characters anywhere.
4. **Bookmark-folder and tab-overflow dropdowns render underneath the page (reported by
   Daniel, root cause verified in code, see PR body for how to check this one).** Both were plain
   DOM dropdowns positioned with `position: absolute`. Every tab's page is a native child
   WebView2 stacked on top of the chrome's DOM, so wherever a DOM dropdown overlapped that
   area it drew underneath the page instead of over it (H1 visibility of system status: the menu
   silently fails to appear). Fixed: both now open through `open_menu_popup`, the same
   child-webview mechanism the hamburger and kebab menus already use, which is drawn as its own
   webview above the tab's webview. Escape and losing focus (the equivalent of an outside click
   for a separate webview) both close it now, which the existing hamburger/kebab popups had not
   had either.
5. **The whole window could scroll, taking the chrome with it (reported by Daniel).** `.shell`
   used `min-height: 100vh` and the sidebar used `position: sticky` as a partial workaround, so a
   long panel (settings, downloads) grew the document past the viewport and the titlebar, address
   bar, tab strip and bookmarks bar scrolled away with everything else (H1; the user loses their
   orientation controls exactly when a panel is long enough to need them). Fixed: `html`/`body`
   are fixed at `100%` height with `overflow: hidden`; the chrome (`.chrome-full`) is a
   non-shrinking flex child; only the new `.main-content` wrapper around the active view scrolls.
   Verified the native tab webview's bounds (`CHROME_HEIGHT` in `src-tauri/src/browser.rs`,
   currently a fixed 134px) still match: the restyled chrome measures 135px in the design preview,
   a 1px difference within existing sub-pixel tolerance, so the constant was left as is.

## High severity

6. **Google Fonts hotlinked from a privacy browser's own chrome.** `App.css` opened with
   `@import url('https://fonts.googleapis.com/...')`, meaning BlueFlame's own UI phoned a third
   party on first paint, the opposite of what the product promises for the pages it filters (H2).
   Fixed: Inter, Space Grotesk and JetBrains Mono are vendored under `public/fonts/` with their
   licences, loaded from `src/fonts.css`; nothing in the chrome fetches an external URL anymore.
7. **No system tokens; the palette, radius and type scale were all local one-offs.** Every colour
   was a hex literal in a local `:root` block, radii were a mix of `0` and a stray `2px`, and the
   whole app forced a single mono font even for sentences (H4 consistency across the wider WickIT
   product line, which this PR joins). Fixed: `src/tokens.css` is a verbatim, hash-checked copy of
   the WickIT kit; every local variable in `App.css` is now an alias onto a system token
   (`--bg: var(--surface-page)`, etc.); body text moved to the system's Inter/`fs-body` role, mono
   stayed for addresses, counts, timestamps and codes per the system's own rule for it.
8. **Action buttons borrowed the brand accent instead of the system's action colour.** `.primary`
   and `.nav-primary` (the address bar's "go" button, the dashboard's main action) filled with
   `--accent`, which this PR turns into the flame's ember colour; a brand mark is not a semantic
   action colour (H4, and the system's own rule that indigo is the only action colour). Fixed: both
   now use `--action-primary` / `--action-primary-hover`; the flame is reserved for the logo, the
   active-tab edge is now indigo instead (see #2), and decorative emphasis (a KPI's highlighted
   number).
9. **No visible, consistent focus ring.** Only 4 of roughly 60 interactive rules in `App.css` set
   any focus style, and 3 of those pointed at the brand accent rather than a real focus indicator
   (WCAG 2.4.7 Focus Visible; H7 flexibility, since keyboard-only use was effectively unsupported
   past those four controls). Fixed: a global `:focus-visible { outline: 2px solid
   var(--focus-ring); outline-offset: 2px; }`, plus the pre-existing bespoke rules pointed at the
   same token; `.tab-card`'s hover and focus states were split apart so focus keeps its own ring
   instead of only a border-colour change.

## Medium severity

10. **No light theme, despite the system shipping one for the Console register.** `App.css`'s own
    comment stated "BlueFlame has no light theme (App.css defines one dark palette only)". Fixed:
    an appearance toggle in Settings (`data-theme` on `<html>`, persisted), and the "after"
    screenshots include the light theme at laptop width per the system's pre-ship checklist.
11. **No reduced-motion guard.** Transitions (tab hover, button states, the trust score's glow)
    ran unconditionally (WCAG 2.2 accessibility minimums). Fixed: a global
    `prefers-reduced-motion: reduce` rule collapses animation/transition duration to near-zero.
12. **Severity colours were arbitrary literals in two places** (`rgba(68, 221, 119, .4)` for a
    running-status border, `rgba(255, 179, 71, .4)` for a warn callout), so the same "ok" or "warn"
    meaning was encoded as a slightly different green or amber than the token system's own
    `wk-ok` / `wk-warn`. Fixed: both now derive from the token via `color-mix()`.

## Not changed in this pass

- Web pages loaded inside a tab are never restyled, per the brief; the audit only covers the
  browser's own chrome.
- `border-radius: 0`, used throughout for the system's "sharp corners only" rule, was left as the
  literal `0` rather than a named token: zero has no unit-dependent ambiguity, so there was
  nothing a token would have added.
- The dev-only `DesignSwitcher` panel (`src/design/design.css`) was left on its own local colours;
  it never ships (`scripts/check-no-design-in-build.mjs` asserts that) and is not part of the
  product's chrome.
