import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Bookmark {
  url: string;
  title: string;
  created_at: number;
}

interface Props {
  /** Bump to trigger a reload (changes when a bookmark is toggled elsewhere). */
  version: number;
  /** Called after the user clicks a bookmark and it was successfully opened. */
  onOpened: () => void;
}

export function BookmarksBar({ version, onOpened }: Props) {
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);

  useEffect(() => {
    invoke<Bookmark[]>('bookmark_list')
      .then(setBookmarks)
      .catch(() => setBookmarks([]));
  }, [version]);

  async function open(url: string) {
    try {
      await invoke('browser_navigate_active', { url });
      onOpened();
    } catch {
      /* ignore */
    }
  }

  return (
    <div className="bookmarks-bar" role="toolbar" aria-label="Bookmarks">
      {bookmarks.length === 0 ? (
        <span className="bookmarks-empty">// star a page to pin it here</span>
      ) : (
        bookmarks.map((b) => (
          <button
            key={b.url}
            className="bookmark-chip"
            onClick={() => open(b.url)}
            title={b.url}
          >
            <span className="bookmark-chip-star" aria-hidden>
              ★
            </span>
            <span className="bookmark-chip-label">{labelFor(b)}</span>
          </button>
        ))
      )}
    </div>
  );
}

function labelFor(b: Bookmark): string {
  if (b.title) return b.title;
  try {
    return new URL(b.url).host.replace(/^www\./, '');
  } catch {
    return b.url;
  }
}
