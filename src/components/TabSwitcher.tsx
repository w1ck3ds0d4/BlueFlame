import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useWebviewOverlay } from '../useWebviewOverlay';

interface TabInfo {
  id: number;
  url: string;
  title: string;
  loading: boolean;
  private: boolean;
}

interface Props {
  open: boolean;
  browsing: boolean;
  tabs: TabInfo[];
  activeId: number | null;
  onClose: () => void;
  onPick: (id: number) => void;
  onCloseTab: (id: number) => void;
  onNewTab: () => void;
  onNewPrivateTab: () => void;
}

/** Full-screen mobile tab switcher. Opens from the tabs-count button in
 *  MobileChrome. While visible we hide all native tab webviews so the
 *  card grid isn't covered; on pick we restore the active tab. */
export function TabSwitcher({
  open,
  browsing,
  tabs,
  activeId,
  onClose,
  onPick,
  onCloseTab,
  onNewTab,
  onNewPrivateTab,
}: Props) {
  const [favicons, setFavicons] = useState<Record<string, string>>({});

  // Hide the active tab's native webview while the switcher is open so
  // the cards aren't covered. Restored by the parent via `onPick` /
  // `onClose` calling `showBrowser`.
  useWebviewOverlay(open, browsing);

  useEffect(() => {
    if (!open) return;
    for (const t of tabs) {
      const host = hostOf(t.url);
      if (!host || favicons[host] !== undefined) continue;
      invoke<string | null>('get_favicon', { host })
        .then((url) => {
          if (url) setFavicons((prev) => ({ ...prev, [host]: url }));
        })
        .catch(() => undefined);
    }
  }, [open, tabs, favicons]);

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose();
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div className="tab-switcher" role="dialog" aria-label="Tab switcher">
      <div className="tab-switcher-header">
        <span className="tab-switcher-title">{tabs.length} tab{tabs.length === 1 ? '' : 's'}</span>
        <button
          className="mobile-icon-btn"
          onClick={onClose}
          aria-label="close tab switcher"
        >
          ×
        </button>
      </div>
      <div className="tab-switcher-grid">
        {tabs.map((t) => {
          const host = hostOf(t.url);
          const fav = host ? favicons[host] : undefined;
          return (
            <div
              key={t.id}
              className={`tab-card ${t.id === activeId ? 'tab-card-active' : ''} ${
                t.private ? 'tab-card-private' : ''
              }`}
              role="button"
              tabIndex={0}
              onClick={() => onPick(t.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onPick(t.id);
                }
              }}
            >
              <div className="tab-card-head">
                <span className="tab-card-favicon" aria-hidden>
                  {fav ? <img src={fav} alt="" /> : <span>·</span>}
                </span>
                <span className="tab-card-title">{t.title || t.url || 'new tab'}</span>
                <button
                  className="tab-card-close"
                  aria-label={`close ${t.title}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    onCloseTab(t.id);
                  }}
                >
                  ×
                </button>
              </div>
              <div className="tab-card-url">
                {t.private && <span className="tab-card-badge">priv</span>}
                {displayUrl(t.url)}
              </div>
            </div>
          );
        })}
      </div>
      <div className="tab-switcher-footer">
        <button className="mobile-menu-item" onClick={onNewTab}>
          <span className="mobile-menu-icon" aria-hidden>+</span>
          <span>new tab</span>
        </button>
        <button className="mobile-menu-item" onClick={onNewPrivateTab}>
          <span className="mobile-menu-icon" aria-hidden>+P</span>
          <span>new private tab</span>
        </button>
      </div>
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

function displayUrl(url: string): string {
  if (!url) return '';
  if (url.startsWith('data:')) return 'new tab';
  if (url.startsWith('about:')) return url;
  try {
    const u = new URL(url);
    return u.host + u.pathname + u.search;
  } catch {
    return url;
  }
}
