import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TrustAssessment } from './TrustPopup';

type View = 'dashboard' | 'bookmarks' | 'settings' | 'debug';

interface Props {
  /** The currently-active tab's URL (used to populate the address bar). */
  currentUrl: string;
  /** Whether there is an active tab to navigate. */
  browsing: boolean;
  /** Opens the tab switcher. App.tsx owns its visible/hidden state. */
  onOpenTabSwitcher: () => void;
  /** Total number of open tabs - shown as a counter on the tabs button. */
  tabCount: number;
  /** Current view (passed through to the hamburger popup so it can
   *  highlight the active entry). */
  view: View;
  /** Bumped by App whenever a bookmark-toggle completes so the kebab
   *  popup gets the correct initial "bookmarked" state. */
  bookmarksVersion: number;
  /** Triggered after the active tab's URL changes so App can refresh tab state. */
  onNavigated: () => void;
  /** Asks App to ensure we're in browse mode (active tab visible). */
  onShowBrowser: () => void;
}

type TrustLabel = 'trusted' | 'ok' | 'suspect' | 'danger';

/** Single-row mobile chrome: hamburger / URL input / trust score /
 *  tabs count / kebab. The menus themselves are child webviews (spawned
 *  by `open_menu_popup`) so they sit on top of the active tab's native
 *  webview instead of being covered by it. */
export function MobileChrome({
  currentUrl,
  browsing,
  onOpenTabSwitcher,
  tabCount,
  view,
  bookmarksVersion,
  onNavigated,
  onShowBrowser,
}: Props) {
  const [input, setInput] = useState(currentUrl);
  const [focused, setFocused] = useState(false);
  const [trust, setTrust] = useState<TrustAssessment | null>(null);
  const [bookmarked, setBookmarked] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const hamburgerBtnRef = useRef<HTMLButtonElement | null>(null);
  const kebabBtnRef = useRef<HTMLButtonElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);

  // Mirror the active tab's URL into the input, but not while the
  // user is mid-typing (the desktop UrlBar does the same).
  useEffect(() => {
    if (document.activeElement === inputRef.current) return;
    if (currentUrl.startsWith('data:')) {
      setInput('');
    } else {
      setInput(currentUrl);
    }
  }, [currentUrl]);

  // Poll trust on URL change so the score badge reflects the current
  // page. 3s is enough to catch late-arriving signals without being
  // chatty.
  useEffect(() => {
    if (
      !browsing ||
      !currentUrl ||
      currentUrl.startsWith('data:') ||
      currentUrl.startsWith('about:')
    ) {
      setTrust(null);
      return;
    }
    let cancelled = false;
    const pull = () => {
      invoke<TrustAssessment>('get_trust', { url: currentUrl })
        .then((a) => {
          if (!cancelled) setTrust(a);
        })
        .catch(() => undefined);
    };
    pull();
    const id = window.setInterval(pull, 3000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [browsing, currentUrl]);

  // Whether the active page is already bookmarked - passed as a query
  // param to the kebab popup so it can show "add" or "remove".
  useEffect(() => {
    if (!browsing || !currentUrl || currentUrl.startsWith('data:')) {
      setBookmarked(false);
      return;
    }
    invoke<boolean>('bookmark_is', { url: currentUrl })
      .then(setBookmarked)
      .catch(() => setBookmarked(false));
  }, [currentUrl, browsing, bookmarksVersion]);

  async function submit() {
    const trimmed = input.trim();
    if (!trimmed) return;
    setError(null);
    try {
      await invoke('browser_navigate_active', { url: trimmed });
      onNavigated();
      onShowBrowser();
    } catch (e) {
      setError(String(e));
    }
  }

  function onTrustClick() {
    if (!browsing || !currentUrl) return;
    invoke('open_trust_panel', { url: currentUrl, tab: 'overview' }).catch(
      () => undefined,
    );
  }

  /** Launch the menu popup anchored just below the given button. The
   *  popup is a child webview so it renders on top of the active tab. */
  function openMenu(kind: 'hamburger' | 'kebab', btn: HTMLButtonElement | null) {
    if (!btn) return;
    const rect = btn.getBoundingClientRect();
    const anchorY = rect.bottom + 4;
    const anchorX = kind === 'hamburger' ? rect.left : rect.right - 220;
    invoke('open_menu_popup', {
      kind,
      anchorX,
      anchorY,
      view,
      browsing,
      bookmarked,
    }).catch(() => undefined);
  }

  const trustClass = trust ? `url-trust-${trust.label as TrustLabel}` : 'url-trust-idle';

  return (
    <div className="mobile-chrome">
      <div className="mobile-chrome-row">
        <button
          ref={hamburgerBtnRef}
          className="mobile-icon-btn"
          onClick={() => openMenu('hamburger', hamburgerBtnRef.current)}
          aria-label="menu"
        >
          ≡
        </button>

        <input
          ref={inputRef}
          type="text"
          className="mobile-url-input mono"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              submit();
            }
          }}
          onFocus={(e) => {
            setFocused(true);
            e.currentTarget.select();
          }}
          onBlur={() => setFocused(false)}
          spellCheck={false}
          autoCorrect="off"
          placeholder={focused ? '' : 'search or enter a url'}
          aria-label="address bar"
        />

        <button
          className={`mobile-icon-btn mobile-trust ${trustClass}`}
          onClick={onTrustClick}
          disabled={!browsing}
          aria-label={trust ? `site scan: ${trust.label} (${trust.score})` : 'site scan'}
        >
          {trust ? trust.score : '!'}
        </button>

        <button
          className="mobile-icon-btn mobile-tabs"
          onClick={onOpenTabSwitcher}
          aria-label={`${tabCount} tabs`}
        >
          <span className="mobile-tabs-count">{tabCount}</span>
        </button>

        <button
          ref={kebabBtnRef}
          className="mobile-icon-btn"
          onClick={() => openMenu('kebab', kebabBtnRef.current)}
          aria-label="actions"
        >
          ⋮
        </button>
      </div>
      {error && (
        <div className="mobile-chrome-error" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}
