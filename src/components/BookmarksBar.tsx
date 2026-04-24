import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Bookmark {
  url: string;
  title: string;
  created_at: number;
  folder: string;
}

interface Props {
  /** Bump to trigger a reload (changes when a bookmark is toggled elsewhere). */
  version: number;
  /** Called after the user clicks a bookmark and it was successfully opened. */
  onOpened: () => void;
}

interface FolderGroup {
  /** Top-level folder name ("" = root). */
  name: string;
  /** Bookmarks under this group, including those in subfolders.
   * `subpath` is the folder path relative to the top-level group
   * (empty when the bookmark sits directly in the group). */
  items: Array<{ bookmark: Bookmark; subpath: string }>;
}

export function BookmarksBar({ version, onOpened }: Props) {
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const [openFolder, setOpenFolder] = useState<string | null>(null);
  const barRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    invoke<Bookmark[]>('bookmark_list')
      .then(setBookmarks)
      .catch(() => setBookmarks([]));
  }, [version]);

  // Close the open folder when clicking outside the bar.
  useEffect(() => {
    if (openFolder === null) return;
    function handler(e: MouseEvent) {
      if (!barRef.current?.contains(e.target as Node)) setOpenFolder(null);
    }
    window.addEventListener('mousedown', handler);
    return () => window.removeEventListener('mousedown', handler);
  }, [openFolder]);

  const { rootChips, folderGroups } = useMemo(() => groupByTopFolder(bookmarks), [bookmarks]);

  async function open(url: string) {
    setOpenFolder(null);
    try {
      await invoke('browser_navigate_active', { url });
      onOpened();
    } catch {
      /* ignore */
    }
  }

  return (
    <div className="bookmarks-bar" role="toolbar" aria-label="Bookmarks" ref={barRef}>
      {bookmarks.length === 0 ? (
        <span className="bookmarks-empty">// star a page to pin it here</span>
      ) : (
        <>
          {rootChips.map((b) => (
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
          ))}
          {folderGroups.map((g) => (
            <div key={g.name} className="bookmark-folder">
              <button
                className={`bookmark-chip bookmark-folder-chip ${openFolder === g.name ? 'open' : ''}`}
                onClick={() => setOpenFolder(openFolder === g.name ? null : g.name)}
                aria-haspopup="true"
                aria-expanded={openFolder === g.name}
                title={`${g.name} (${g.items.length})`}
              >
                <span className="bookmark-folder-icon" aria-hidden>
                  ▸
                </span>
                <span className="bookmark-chip-label">{g.name}</span>
              </button>
              {openFolder === g.name && (
                <div className="bookmark-folder-panel" role="menu">
                  {g.items.map(({ bookmark, subpath }) => (
                    <button
                      key={bookmark.url}
                      className="bookmark-folder-item"
                      onClick={() => open(bookmark.url)}
                      title={bookmark.url}
                      role="menuitem"
                    >
                      {subpath && (
                        <span className="bookmark-folder-item-path">{subpath}/</span>
                      )}
                      <span className="bookmark-folder-item-label">{labelFor(bookmark)}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          ))}
        </>
      )}
    </div>
  );
}

function groupByTopFolder(bookmarks: Bookmark[]): {
  rootChips: Bookmark[];
  folderGroups: FolderGroup[];
} {
  const root: Bookmark[] = [];
  const groups = new Map<string, FolderGroup>();
  for (const b of bookmarks) {
    const folder = b.folder?.trim() ?? '';
    if (!folder) {
      root.push(b);
      continue;
    }
    const [top, ...rest] = folder.split('/');
    if (!groups.has(top)) groups.set(top, { name: top, items: [] });
    groups.get(top)!.items.push({ bookmark: b, subpath: rest.join('/') });
  }
  return {
    rootChips: root,
    folderGroups: [...groups.values()].sort((a, b) => a.name.localeCompare(b.name)),
  };
}

function labelFor(b: Bookmark): string {
  if (b.title) return b.title;
  try {
    return new URL(b.url).host.replace(/^www\./, '');
  } catch {
    return b.url;
  }
}
