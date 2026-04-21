import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { BRAILLE_FRAMES, useAsciiFrames } from '../ascii';

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
}

export function TabStrip({ tabs, activeId, onSelect, onClose, onNewTab, onNewPrivateTab }: Props) {
  const anyLoading = tabs.some((t) => t.loading);
  const spinner = useAsciiFrames(BRAILLE_FRAMES, 90, anyLoading);
  const [favicons, setFavicons] = useState<Record<string, string>>({});
  const fetching = useRef<Set<string>>(new Set());

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
    <div className="tab-strip" role="tablist" aria-label="Tabs">
      {tabs.map((t) => {
        const host = hostOf(t.url);
        const fav = host ? favicons[host] : undefined;
        return (
          <button
            key={t.id}
            role="tab"
            aria-selected={t.id === activeId}
            className={`tab ${t.id === activeId ? 'tab-active' : ''} ${
              t.private ? 'tab-private' : ''
            }`}
            onClick={() => onSelect(t.id)}
            title={t.private ? `[private] ${t.url}` : t.url}
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
            <span className="tab-title">{t.title || t.url}</span>
            <span
              className="tab-close"
              role="button"
              aria-label={`Close ${t.title}`}
              onClick={(e) => {
                e.stopPropagation();
                onClose(t.id);
              }}
            >
              ×
            </span>
          </button>
        );
      })}
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
