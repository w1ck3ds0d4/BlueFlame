import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { ChevronDown, VenetianMask } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { BRAILLE_FRAMES, useAsciiFrames } from '../ascii';

// Below this width a tab has no room left to shrink further (favicon,
// a sliver of title, close button); tabs shrink together down to this
// floor before any of them move into the overflow menu.
const MIN_TAB_WIDTH = 96;
// Reserved for the "+" / "+P" new-tab buttons and the overflow toggle,
// so the fit calculation below never overestimates how many tabs the
// strip can show without wrapping and reintroducing a scrollbar.
const TRAILING_CONTROLS_WIDTH = 84;
const OVERFLOW_TOGGLE_WIDTH = 34;

interface TabInfo {
  id: number;
  url: string;
  title: string;
  loading: boolean;
  private: boolean;
}

interface Props {
  tabs: TabInfo[];
  activeId: number | null;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
  onNewTab: () => void;
  onNewPrivateTab: () => void;
  /** Tabs Claude opened through the control channel. Rendered with a badge
   * so it's always visible which tabs Claude is driving. */
  claudeTabIds?: number[];
}

export function TabStrip({
  tabs,
  activeId,
  onSelect,
  onClose,
  onNewTab,
  onNewPrivateTab,
  claudeTabIds,
}: Props) {
  const anyLoading = tabs.some((t) => t.loading);
  const spinner = useAsciiFrames(BRAILLE_FRAMES, 90, anyLoading);
  const [favicons, setFavicons] = useState<Record<string, string>>({});
  const fetching = useRef<Set<string>>(new Set());
  const stripRef = useRef<HTMLDivElement | null>(null);
  const [maxVisible, setMaxVisible] = useState(tabs.length);

  // Tabs shrink together down to MIN_TAB_WIDTH as more open; past that
  // point, remaining tabs move into an overflow menu instead of forcing
  // a scrollbar (the strip's own overflow is always hidden, see CSS).
  useLayoutEffect(() => {
    const el = stripRef.current;
    if (!el) return;
    const measure = () => {
      const available = el.clientWidth - TRAILING_CONTROLS_WIDTH;
      const fitsAll = Math.floor(available / MIN_TAB_WIDTH);
      const fitsWithMenu = Math.floor((available - OVERFLOW_TOGGLE_WIDTH) / MIN_TAB_WIDTH);
      setMaxVisible(fitsAll >= tabs.length ? tabs.length : Math.max(1, fitsWithMenu));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [tabs.length]);

  let visibleTabs = tabs.slice(0, maxVisible);
  const activeInView = activeId == null || visibleTabs.some((t) => t.id === activeId);
  if (!activeInView && maxVisible > 0) {
    const active = tabs.find((t) => t.id === activeId);
    if (active) visibleTabs = [...tabs.slice(0, maxVisible - 1), active];
  }
  const visibleIds = new Set(visibleTabs.map((t) => t.id));
  const hiddenTabs = tabs.filter((t) => !visibleIds.has(t.id));

  // The overflow list is a child-webview popup (open_menu_popup), not a
  // DOM dropdown: it can extend down over the page area below the
  // chrome, and a DOM element there would render underneath the active
  // tab's native WebView2. Selecting or closing a tab from inside it
  // comes back as a `blueflame:select-tab` / `blueflame:close-tab`
  // event, which App.tsx turns into the same onSelect/onClose calls a
  // click in the strip itself would have made.
  function openOverflowMenu(btn: HTMLButtonElement) {
    const rect = btn.getBoundingClientRect();
    const items = hiddenTabs.map((t) => ({ id: t.id, url: t.url, title: t.title || t.url }));
    invoke('open_menu_popup', {
      kind: 'tab-overflow',
      anchorX: rect.right - 220,
      anchorY: rect.bottom + 4,
      items: JSON.stringify(items),
    }).catch(() => undefined);
  }

  // Pull the favicon for each tab's host, once per host per session. The
  // backend emits 'blueflame:favicon-ready' when it finishes fetching one,
  // so listen for that and refresh then too.
  useEffect(() => {
    for (const t of tabs) {
      const host = hostOf(t.url);
      if (!host || favicons[host] !== undefined || fetching.current.has(host)) continue;
      fetching.current.add(host);
      invoke<string | null>('get_favicon', { host })
        .then((url) => {
          if (url) setFavicons((prev) => ({ ...prev, [host]: url }));
        })
        .catch(() => undefined)
        .finally(() => {
          fetching.current.delete(host);
        });
    }
  }, [tabs, favicons]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen<string>('blueflame:favicon-ready', (e) => {
      const host = e.payload;
      fetching.current.delete(host);
      invoke<string | null>('get_favicon', { host })
        .then((url) => {
          if (url) setFavicons((prev) => ({ ...prev, [host]: url }));
        })
        .catch(() => undefined);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      unlisten?.();
    };
  }, []);

  return (
    <div className="tab-strip" role="tablist" aria-label="Tabs" ref={stripRef}>
      {visibleTabs.map((t) => {
        const host = hostOf(t.url);
        const fav = host ? favicons[host] : undefined;
        const claudeDriven = claudeTabIds?.includes(t.id) ?? false;
        return (
          // Two sibling buttons, not a button nested inside a button: a
          // <button> cannot nest inside another <button> (invalid HTML,
          // and a nested non-button close target could not take keyboard
          // focus of its own), so a keyboard user could select a tab here
          // but never close one. Same structure as MenuPopup.tsx's
          // "tab-overflow" kind.
          <div
            key={t.id}
            className={`tab-wrap ${t.id === activeId ? 'tab-active' : ''} ${
              t.private ? 'tab-private' : ''
            } ${claudeDriven ? 'tab-claude' : ''}`}
          >
            <button
              role="tab"
              aria-selected={t.id === activeId}
              className="tab"
              onClick={() => onSelect(t.id)}
              title={claudeDriven ? `[Claude is driving this tab] ${t.url}` : t.private ? `private tab: ${t.url}` : t.url}
            >
              <span className="tab-favicon" aria-hidden>
                {t.loading ? (
                  <span className="tab-spinner">{spinner}</span>
                ) : fav ? (
                  <img src={fav} alt="" className="tab-favicon-img" />
                ) : (
                  <span className="tab-spinner tab-spinner-dim">·</span>
                )}
              </span>
              {t.private && (
                <VenetianMask className="tab-private-icon" aria-hidden size={12} strokeWidth={1.75} />
              )}
              {claudeDriven && (
                <span className="tab-claude-badge" aria-label="Claude is driving this tab">
                  C
                </span>
              )}
              <span className="tab-title">{t.title || t.url}</span>
            </button>
            <button
              type="button"
              className="tab-close"
              aria-label={`Close ${t.title}`}
              onClick={(e) => {
                e.stopPropagation();
                onClose(t.id);
              }}
            >
              <span aria-hidden>&times;</span>
            </button>
          </div>
        );
      })}
      {hiddenTabs.length > 0 && (
        <button
          className="tab-overflow-toggle"
          onClick={(e) => openOverflowMenu(e.currentTarget)}
          aria-label={`${hiddenTabs.length} more tabs`}
          aria-haspopup="menu"
          title={`${hiddenTabs.length} more tabs`}
        >
          <span className="tab-overflow-count">{hiddenTabs.length}</span>
          <ChevronDown size={12} strokeWidth={2} aria-hidden />
        </button>
      )}
      <button className="tab-new" onClick={onNewTab} aria-label="New tab" title="new tab (^t)">
        +
      </button>
      <button
        className="tab-new tab-new-private"
        onClick={onNewPrivateTab}
        aria-label="New private tab"
        title="new private tab (^⇧t) - no history, not saved"
      >
        +P
      </button>
    </div>
  );
}

function hostOf(url: string): string {
  if (!url || url.startsWith('data:') || url.startsWith('about:')) return '';
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}
