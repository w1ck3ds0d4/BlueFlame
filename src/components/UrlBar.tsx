import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import type { TrustAssessment } from './TrustPopup';

interface Suggestion {
  url: string;
  title: string;
  source: 'bookmark' | 'history';
  visit_count: number;
}

interface Props {
  /** Called after a successful navigation. The caller refreshes tab state. */
  onNavigated: () => void;
  /** Called when the user wants to leave browse mode and see the shell view. */
  onHome: () => void;
  /** The currently-active tab's URL (used to populate the input). */
  currentUrl: string;
  /** The currently-active tab's title (used when saving a bookmark). */
  currentTitle: string;
  /** Whether there is an active tab to navigate. */
  browsing: boolean;
  /** Called after the user toggles the bookmark for the active tab. */
  onBookmarksChanged: () => void;
}

const SUGGEST_DEBOUNCE_MS = 120;
const SUGGEST_LIMIT = 8;

export function UrlBar({
  onNavigated,
  onHome,
  currentUrl,
  currentTitle,
  browsing,
  onBookmarksChanged,
}: Props) {
  const [input, setInput] = useState(currentUrl);
  const [error, setError] = useState<string | null>(null);
  const [bookmarked, setBookmarked] = useState(false);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [highlight, setHighlight] = useState(-1);
  const [open, setOpen] = useState(false);
  const [focused, setFocused] = useState(false);
  const [blocksForHost, setBlocksForHost] = useState(0);
  const [trustOpen, setTrustOpen] = useState(false);
  const [trust, setTrust] = useState<TrustAssessment | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const queryToken = useRef(0);

  const bookmarkable =
    browsing && !!currentUrl && !currentUrl.startsWith('data:') && !currentUrl.startsWith('about:');

  useEffect(() => {
    // While the user is actively typing (input focused), don't overwrite
    // their draft. Pages trigger many on_navigation events for subresource
    // loads and redirects, and each one pushes a new currentUrl down; if
    // we eagerly sync the input, we wipe what the user is mid-typing.
    if (document.activeElement === inputRef.current) {
      return;
    }
    if (currentUrl.startsWith('data:')) {
      setInput('');
    } else {
      setInput(currentUrl);
    }
    setOpen(false);
    setSuggestions([]);
    setHighlight(-1);
  }, [currentUrl]);

  useEffect(() => {
    if (!bookmarkable) {
      setBookmarked(false);
      return;
    }
    invoke<boolean>('bookmark_is', { url: currentUrl })
      .then(setBookmarked)
      .catch(() => setBookmarked(false));
  }, [currentUrl, bookmarkable]);

  // Pull the trust score for the current page so the URL-bar badge shows
  // a live number. Re-pull every ~3s while browsing so late-arriving
  // security signals (third-party request footprint, mixed-content
  // resources) update the badge without the user having to click.
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

  // Close the panel when the user navigates away - the assessment is
  // pinned to the URL the popup was opened on, so if the tab changes
  // its URL out from under it, the panel's data is now for the wrong
  // site. Simpler to close and let the user reopen than to live-update.
  useEffect(() => {
    if (!trustOpen) return;
    invoke('close_trust_panel').catch(() => undefined);
    setTrustOpen(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentUrl]);

  // Per-site block counter: poll the backend every ~1.5s while on a real
  // page so the pill reflects roughly-live activity without hammering.
  useEffect(() => {
    if (!browsing || !currentUrl || currentUrl.startsWith('data:')) {
      setBlocksForHost(0);
      return;
    }
    let host: string;
    try {
      host = new URL(currentUrl).host;
    } catch {
      setBlocksForHost(0);
      return;
    }
    let cancelled = false;
    const pull = () => {
      invoke<number>('get_blocks_for_host', { host })
        .then((n) => {
          if (!cancelled) setBlocksForHost(n);
        })
        .catch(() => undefined);
    };
    pull();
    const id = window.setInterval(pull, 1500);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [browsing, currentUrl]);

  // Close on outside click.
  useEffect(() => {
    function onDocClick(e: MouseEvent) {
      if (!containerRef.current) return;
      if (!containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('mousedown', onDocClick);
    return () => document.removeEventListener('mousedown', onDocClick);
  }, []);

  // Ctrl+L from the App-level shortcut handler lands here.
  useEffect(() => {
    function onFocusRequest() {
      inputRef.current?.focus();
      inputRef.current?.select();
    }
    window.addEventListener('blueflame:focus-url', onFocusRequest);
    return () => window.removeEventListener('blueflame:focus-url', onFocusRequest);
  }, []);

  function requestSuggestions(q: string) {
    const token = ++queryToken.current;
    const trimmed = q.trim();
    if (!trimmed) {
      setSuggestions([]);
      setHighlight(-1);
      return;
    }
    window.setTimeout(() => {
      // Drop results from a stale typing-burst.
      if (token !== queryToken.current) return;
      invoke<Suggestion[]>('url_suggest', { query: trimmed, limit: SUGGEST_LIMIT })
        .then((list) => {
          if (token !== queryToken.current) return;
          setSuggestions(list);
          setHighlight(list.length > 0 ? -1 : -1);
        })
        .catch(() => {
          if (token !== queryToken.current) return;
          setSuggestions([]);
        });
    }, SUGGEST_DEBOUNCE_MS);
  }

  function onInputChange(next: string) {
    setInput(next);
    setOpen(true);
    requestSuggestions(next);
  }

  async function toggleBookmark() {
    if (!bookmarkable) return;
    try {
      const next = await invoke<boolean>('bookmark_toggle', {
        url: currentUrl,
        title: currentTitle || currentUrl,
      });
      setBookmarked(next);
      onBookmarksChanged();
    } catch (e) {
      setError(String(e));
    }
  }

  async function navigateTo(url: string) {
    setError(null);
    try {
      await invoke('browser_navigate_active', { url });
      setOpen(false);
      onNavigated();
    } catch (e) {
      setError(String(e));
    }
  }

  async function submit() {
    if (highlight >= 0 && suggestions[highlight]) {
      await navigateTo(suggestions[highlight].url);
      return;
    }
    const trimmed = input.trim();
    if (!trimmed) return;
    await navigateTo(trimmed);
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
      return;
    }
    if (e.key === 'Escape') {
      if (open) {
        setOpen(false);
        e.preventDefault();
      }
      return;
    }
    if (!open || suggestions.length === 0) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setHighlight((h) => (h + 1 >= suggestions.length ? 0 : h + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setHighlight((h) => (h <= 0 ? suggestions.length - 1 : h - 1));
    }
  }

  async function goBack() {
    try {
      await invoke('browser_back');
    } catch (e) {
      setError(String(e));
    }
  }

  async function goForward() {
    try {
      await invoke('browser_forward');
    } catch (e) {
      setError(String(e));
    }
  }

  async function reload() {
    try {
      await invoke('browser_reload');
    } catch (e) {
      setError(String(e));
    }
  }

  async function home() {
    try {
      await invoke('browser_hide_all');
    } catch {
      /* ignore */
    }
    onHome();
  }

  return (
    <div className="url-bar-row" role="toolbar" aria-label="Browser chrome">
      <button
        className="nav-icon"
        onClick={goBack}
        disabled={!browsing}
        title="back"
        aria-label="back"
      >
        ←
      </button>
      <button
        className="nav-icon"
        onClick={goForward}
        disabled={!browsing}
        title="forward"
        aria-label="forward"
      >
        →
      </button>
      <button
        className="nav-icon"
        onClick={reload}
        disabled={!browsing}
        title="reload"
        aria-label="reload"
      >
        ⟳
      </button>
      <button className="nav-icon" onClick={home} title="home" aria-label="home">
        ~
      </button>

      <div className="url-input-wrap" ref={containerRef}>
        <input
          ref={inputRef}
          type="text"
          className="url-input mono"
          value={input}
          onChange={(e) => onInputChange(e.target.value)}
          onKeyDown={onKeyDown}
          onFocus={(e) => {
            setFocused(true);
            e.currentTarget.select();
            if (input.trim()) {
              setOpen(true);
              requestSuggestions(input);
            }
          }}
          onBlur={() => setFocused(false)}
          spellCheck={false}
          autoCorrect="off"
          role="combobox"
          aria-expanded={open && suggestions.length > 0}
          aria-autocomplete="list"
          aria-controls="url-suggest-list"
          aria-activedescendant={highlight >= 0 ? `url-suggest-${highlight}` : undefined}
          aria-label="address bar"
        />
        {!input && !focused && (
          <span className="url-input-placeholder-cursor" aria-hidden>
            <span>&gt; </span>
            <span className="ascii-cursor blinking">_</span>
          </span>
        )}
        {open && suggestions.length > 0 && (
          <ul
            id="url-suggest-list"
            className="url-suggest"
            role="listbox"
            onMouseDown={(e) => e.preventDefault()}
          >
            {suggestions.map((s, i) => (
              <li
                id={`url-suggest-${i}`}
                key={s.url}
                role="option"
                aria-selected={i === highlight}
                className={`url-suggest-row ${i === highlight ? 'url-suggest-hl' : ''}`}
                onMouseEnter={() => setHighlight(i)}
                onClick={() => navigateTo(s.url)}
              >
                <span
                  className={`url-suggest-badge url-suggest-${s.source}`}
                  aria-label={s.source}
                >
                  {s.source === 'bookmark' ? '[b]' : '[h]'}
                </span>
                <span className="url-suggest-title">{s.title || hostOf(s.url)}</span>
                <span className="url-suggest-url">{s.url}</span>
              </li>
            ))}
          </ul>
        )}
      </div>

      <span
        className={`block-counter ${blocksForHost > 0 ? 'block-counter-on' : 'block-counter-off'}`}
        title={
          browsing
            ? `${blocksForHost} trackers blocked on this site`
            : 'open a page to see per-site blocks'
        }
        aria-label={`${blocksForHost} blocks on this site`}
      >
        <span className="block-counter-glyph" aria-hidden>
          ▒
        </span>
        <span className="block-counter-num">{blocksForHost}</span>
      </span>

      <button
        className={`nav-icon url-trust ${trust ? `url-trust-${trust.label}` : 'url-trust-idle'}`}
        onClick={() => {
          if (!browsing || !currentUrl) return;
          if (trustOpen) {
            invoke('close_trust_panel').catch(() => undefined);
            setTrustOpen(false);
          } else {
            invoke('open_trust_panel', { url: currentUrl, tab: 'overview' }).catch(
              () => undefined,
            );
            setTrustOpen(true);
          }
        }}
        disabled={!browsing}
        title={trust ? `site scan: ${trust.label} (${trust.score})` : 'site scan'}
        aria-label="site scan"
        aria-expanded={trustOpen}
      >
        {trust ? trust.score : '!'}
      </button>

      <button
        className={`nav-icon url-star ${bookmarked ? 'url-star-on' : ''}`}
        onClick={toggleBookmark}
        disabled={!bookmarkable}
        title={bookmarked ? 'remove bookmark' : 'add bookmark'}
        aria-label={bookmarked ? 'remove bookmark' : 'add bookmark'}
        aria-pressed={bookmarked}
      >
        {bookmarked ? '*' : '+'}
      </button>

      <button className="nav-primary" onClick={submit}>
        go
      </button>

      {error && (
        <div className="url-error" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, '');
  } catch {
    return url;
  }
}
