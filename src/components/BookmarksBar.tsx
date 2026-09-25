import { useEffect, useMemo, useState } from 'react';
import { Bookmark as BookmarkIcon, ChevronRight, Folder } from 'lucide-react';
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

  useEffect(() => {
    invoke<Bookmark[]>('bookmark_list')
      .then(setBookmarks)
      .catch(() => setBookmarks([]));
  }, [version]);

  const { rootChips, folderGroups } = useMemo(() => groupByTopFolder(bookmarks), [bookmarks]);

  async function open(url: string) {
    try {
      await invoke('browser_navigate_active', { url });
      onOpened();
    } catch {
      /* ignore */
    }
  }

  // A folder's contents open as a menu-popup child webview (the same
  // mechanism the hamburger and kebab menus use), not a DOM dropdown:
  // this bar sits right above the active tab's native WebView2, and a
  // DOM dropdown here rendered underneath that webview wherever the
  // two overlapped. The chosen bookmark comes back as a
  // `blueflame:navigate-to` event, which App.tsx turns into the same
  // browser_navigate_active + refresh this component's own `open`
  // above does.
  function openFolderMenu(g: FolderGroup, btn: HTMLButtonElement) {
    const rect = btn.getBoundingClientRect();
    const items = g.items.map(({ bookmark, subpath }) => ({
      url: bookmark.url,
      title: subpath ? `${subpath}/${labelFor(bookmark)}` : labelFor(bookmark),
    }));
    invoke('open_menu_popup', {
      kind: 'bookmarks-folder',
      anchorX: rect.left,
      anchorY: rect.bottom + 4,
      folder: g.name,
      items: JSON.stringify(items),
    }).catch(() => undefined);
  }

  return (
    <div className="bookmarks-bar" role="toolbar" aria-label="Bookmarks">
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
              <BookmarkIcon className="bookmark-chip-icon" aria-hidden size={13} strokeWidth={1.75} />
              <span className="bookmark-chip-label">{labelFor(b)}</span>
            </button>
          ))}
          {folderGroups.map((g) => (
            <button
              key={g.name}
              className="bookmark-chip bookmark-folder-chip"
              onClick={(e) => openFolderMenu(g, e.currentTarget)}
              aria-haspopup="menu"
              title={`${g.name} (${g.items.length})`}
            >
              <Folder className="bookmark-chip-icon" aria-hidden size={13} strokeWidth={1.75} />
              <span className="bookmark-chip-label">{g.name}</span>
              <ChevronRight className="bookmark-folder-chevron" aria-hidden size={12} strokeWidth={2} />
            </button>
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
