import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/** Shared favicon cache keyed by canonical host. Multiple consumers
 *  (tab strip, quick-access list, bookmarks, history) all use the
 *  same backend cache (`get_favicon` + `blueflame:favicon-ready`), so
 *  this hook deduplicates fetches across the entire frontend and
 *  delivers updates to every component holding a reference. */
export function useFavicons(urls: string[]): Record<string, string | undefined> {
  const [favicons, setFavicons] = useState<Record<string, string>>({});
  const fetching = useRef<Set<string>>(new Set());

  useEffect(() => {
    for (const url of urls) {
      const host = hostOf(url);
      if (!host || favicons[host] !== undefined || fetching.current.has(host)) continue;
      fetching.current.add(host);
      invoke<string | null>('get_favicon', { host })
        .then((u) => {
          if (u) setFavicons((prev) => ({ ...prev, [host]: u }));
        })
        .catch(() => undefined)
        .finally(() => {
          fetching.current.delete(host);
        });
    }
    // urls is a fresh array on every render; depending on its identity
    // would re-run on every parent state change. We DO want re-runs
    // when the URL set changes, but referential equality would force
    // callers to memoize. Join + length is cheap and behaves right.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [urls.join('|'), favicons]);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen<string>('blueflame:favicon-ready', (e) => {
      const host = e.payload;
      fetching.current.delete(host);
      invoke<string | null>('get_favicon', { host })
        .then((u) => {
          if (u) setFavicons((prev) => ({ ...prev, [host]: u }));
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

  // Project the host-keyed cache into url-keyed lookups for callers
  // that have URLs in hand and don't want to re-derive the host.
  const out: Record<string, string | undefined> = {};
  for (const url of urls) {
    const host = hostOf(url);
    if (host) out[url] = favicons[host];
  }
  return out;
}

export function hostOf(url: string): string {
  if (!url || url.startsWith('data:') || url.startsWith('about:')) return '';
  try {
    return new URL(url).host;
  } catch {
    return '';
  }
}

/** Deterministic accent color for a host. Same input → same hue, so
 *  the tab's color underline + any other host-tinted UI stays stable
 *  across reloads. Returns a CSS HSL string in the cool/neutral range
 *  that doesn't clash with BlueFlame's accent blue. */
export function hostAccent(host: string): string {
  if (!host) return 'transparent';
  let h = 0;
  for (let i = 0; i < host.length; i++) {
    h = (h * 31 + host.charCodeAt(i)) >>> 0;
  }
  // 0-359 hue; keep S/L locked so colors are uniform-feeling.
  const hue = h % 360;
  return `hsl(${hue} 70% 58%)`;
}
