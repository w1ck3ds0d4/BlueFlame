import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Bookmark {
  url: string;
  title: string;
  created_at: number;
  folder: string;
}

interface Props {
  /** Bumped when a bookmark toggles elsewhere so we reload. */
  version: number;
}

/**
 * Full-page bookmarks browser. Renders a breadcrumb + a list of subfolders
 * and bookmarks at the current path. Tap a folder to descend; tap a bookmark
 * to open it in a new tab. Designed to fit the mobile viewport (one column,
 * large tap targets) and degrades gracefully on desktop.
 */
export function Bookmarks({ version }: Props) {
  const [all, setAll] = useState<Bookmark[]>([]);
  const [path, setPath] = useState<string[]>([]);
  const [query, setQuery] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [moveTarget, setMoveTarget] = useState<Bookmark | null>(null);
  const [allFolders, setAllFolders] = useState<string[]>([]);

  async function reload() {
    try {
      const list = await invoke<Bookmark[]>('bookmark_list');
      setAll(list);
      const folders = await invoke<string[]>('bookmark_folders');
      setAllFolders(folders);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    reload();
  }, [version]);

  const currentPath = path.join('/');

  // Compute the children (subfolders + bookmarks) of the current path.
  const { subfolders, items } = useMemo(
    () => childrenOf(all, currentPath),
    [all, currentPath],
  );

  // Filtered search results span the whole tree.
  const searchHits = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return null;
    return all.filter(
      (b) =>
        b.title.toLowerCase().includes(q) ||
        b.url.toLowerCase().includes(q) ||
        b.folder.toLowerCase().includes(q),
    );
  }, [all, query]);

  async function openBookmark(url: string) {
    try {
      await invoke('browser_open_tab', { url });
    } catch (e) {
      setError(String(e));
    }
  }

  async function removeBookmark(url: string) {
    try {
      await invoke('bookmark_toggle', { url, title: '' });
      await reload();
    } catch (e) {
      setError(String(e));
    }
  }

  async function move(url: string, folder: string) {
    try {
      await invoke('bookmark_set_folder', { url, folder });
      await reload();
      setMoveTarget(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function renameFolder() {
    if (!currentPath) return;
    const next = prompt('Rename folder to:', currentPath);
    if (next == null || next.trim() === currentPath) return;
    try {
      await invoke('bookmark_rename_folder', { old: currentPath, new: next.trim() });
      setPath(next.trim().split('/').filter(Boolean));
      await reload();
    } catch (e) {
      setError(String(e));
    }
  }

  async function deleteFolder() {
    if (!currentPath) return;
    if (
      !confirm(
        `Delete folder "${currentPath}" and every bookmark inside it? This can't be undone.`,
      )
    ) {
      return;
    }
    try {
      await invoke<number>('bookmark_delete_folder', { folder: currentPath });
      setPath(path.slice(0, -1));
      await reload();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <section className="bookmarks-page">
      <div className="bookmarks-page-header">
        <h2 className="settings-title">bookmarks</h2>
        <div className="bookmarks-page-count">
          {all.length} total · {allFolders.length} folder
          {allFolders.length === 1 ? '' : 's'}
        </div>
      </div>

      <input
        type="text"
        className="url-input mono"
        placeholder="grep bookmarks..."
        value={query}
        onChange={(e) => setQuery(e.currentTarget.value)}
        spellCheck={false}
        autoCorrect="off"
      />

      {error && <div className="error-banner">{error}</div>}

      {searchHits != null ? (
        <div className="bookmarks-page-list" role="list">
          <div className="bookmarks-page-section-title">
            // {searchHits.length} match{searchHits.length === 1 ? '' : 'es'}
          </div>
          {searchHits.length === 0 ? (
            <div className="bookmarks-page-empty">// no bookmarks match</div>
          ) : (
            searchHits.map((b) => (
              <BookmarkRow
                key={b.url}
                bookmark={b}
                showFolder
                onOpen={() => openBookmark(b.url)}
                onMove={() => setMoveTarget(b)}
                onRemove={() => removeBookmark(b.url)}
              />
            ))
          )}
        </div>
      ) : (
        <>
          <div className="bookmarks-breadcrumb" role="navigation" aria-label="Folder path">
            <button
              type="button"
              className={`bookmarks-crumb ${path.length === 0 ? 'current' : ''}`}
              onClick={() => setPath([])}
            >
              // root
            </button>
            {path.map((segment, i) => (
              <span key={i} className="bookmarks-crumb-wrap">
                <span className="bookmarks-crumb-sep" aria-hidden>
                  /
                </span>
                <button
                  type="button"
                  className={`bookmarks-crumb ${i === path.length - 1 ? 'current' : ''}`}
                  onClick={() => setPath(path.slice(0, i + 1))}
                >
                  {segment}
                </button>
              </span>
            ))}
            {currentPath && (
              <div className="bookmarks-crumb-actions">
                <button className="secondary" onClick={renameFolder}>
                  rename
                </button>
                <button className="secondary" onClick={deleteFolder}>
                  delete
                </button>
              </div>
            )}
          </div>

          <div className="bookmarks-page-list" role="list">
            {subfolders.length === 0 && items.length === 0 ? (
              <div className="bookmarks-page-empty">
                {currentPath ? '// folder is empty' : '// no bookmarks yet'}
              </div>
            ) : (
              <>
                {subfolders.map((name) => (
                  <button
                    key={name}
                    type="button"
                    className="bookmarks-page-folder"
                    onClick={() => setPath([...path, name])}
                  >
                    <span className="bookmarks-page-folder-icon" aria-hidden>
                      ▸
                    </span>
                    <span className="bookmarks-page-folder-name">{name}</span>
                    <span className="bookmarks-page-folder-count">
                      {descendantCount(all, [...path, name].join('/'))}
                    </span>
                  </button>
                ))}
                {items.map((b) => (
                  <BookmarkRow
                    key={b.url}
                    bookmark={b}
                    onOpen={() => openBookmark(b.url)}
                    onMove={() => setMoveTarget(b)}
                    onRemove={() => removeBookmark(b.url)}
                  />
                ))}
              </>
            )}
          </div>
        </>
      )}

      {moveTarget && (
        <MoveBookmarkDialog
          bookmark={moveTarget}
          folders={allFolders}
          onCancel={() => setMoveTarget(null)}
          onMove={(folder) => move(moveTarget.url, folder)}
        />
      )}
    </section>
  );
}

interface BookmarkRowProps {
  bookmark: Bookmark;
  showFolder?: boolean;
  onOpen: () => void;
  onMove: () => void;
  onRemove: () => void;
}

function BookmarkRow({ bookmark, showFolder, onOpen, onMove, onRemove }: BookmarkRowProps) {
  return (
    <div className="bookmarks-page-row">
      <button className="bookmarks-page-row-main" onClick={onOpen} title={bookmark.url}>
        <span className="bookmarks-page-row-star" aria-hidden>
          ★
        </span>
        <span className="bookmarks-page-row-text">
          <span className="bookmarks-page-row-title">{bookmark.title || bookmark.url}</span>
          <span className="bookmarks-page-row-url">
            {showFolder && bookmark.folder && (
              <span className="bookmarks-page-row-folder">[{bookmark.folder}] </span>
            )}
            {bookmark.url}
          </span>
        </span>
      </button>
      <div className="bookmarks-page-row-actions">
        <button className="secondary" onClick={onMove} title="Move to folder">
          move
        </button>
        <button className="secondary" onClick={onRemove} title="Delete bookmark">
          del
        </button>
      </div>
    </div>
  );
}

interface MoveDialogProps {
  bookmark: Bookmark;
  folders: string[];
  onCancel: () => void;
  onMove: (folder: string) => void;
}

function MoveBookmarkDialog({ bookmark, folders, onCancel, onMove }: MoveDialogProps) {
  const [custom, setCustom] = useState('');
  return (
    <div className="pi-modal-scrim" onClick={onCancel}>
      <div className="pi-modal" onClick={(e) => e.stopPropagation()}>
        <div className="pi-modal-title">move bookmark</div>
        <div className="pi-modal-subtitle">{bookmark.title || bookmark.url}</div>

        <div className="pi-modal-list">
          <button className="pi-modal-option" onClick={() => onMove('')}>
            <span className="pi-modal-option-label">// root (no folder)</span>
          </button>
          {folders.map((f) => (
            <button
              key={f}
              className={`pi-modal-option ${f === bookmark.folder ? 'current' : ''}`}
              onClick={() => onMove(f)}
            >
              <span className="pi-modal-option-label">{f}</span>
              {f === bookmark.folder && (
                <span className="pi-modal-option-current">(current)</span>
              )}
            </button>
          ))}
        </div>

        <div className="pi-modal-new">
          <label htmlFor="bkm-new-folder">new folder (slash for nesting)</label>
          <div className="pi-modal-new-row">
            <input
              id="bkm-new-folder"
              type="text"
              className="url-input mono"
              value={custom}
              onChange={(e) => setCustom(e.currentTarget.value)}
              spellCheck={false}
              autoCorrect="off"
              placeholder="folder path"
            />
            <button
              className="secondary"
              onClick={() => custom.trim() && onMove(custom.trim())}
              disabled={!custom.trim()}
            >
              create + move
            </button>
          </div>
        </div>

        <div className="pi-modal-buttons">
          <button className="secondary" onClick={onCancel}>
            cancel
          </button>
        </div>
      </div>
    </div>
  );
}

/** Immediate subfolders + bookmarks directly at the given folder path. */
function childrenOf(
  all: Bookmark[],
  path: string,
): { subfolders: string[]; items: Bookmark[] } {
  const subs = new Set<string>();
  const items: Bookmark[] = [];
  for (const b of all) {
    const f = b.folder ?? '';
    if (f === path) {
      items.push(b);
      continue;
    }
    // Check whether this bookmark is inside a subfolder of `path`.
    const under = path === '' ? f : f.startsWith(path + '/') ? f.slice(path.length + 1) : null;
    if (under === null) continue;
    const firstSeg = under.split('/')[0];
    if (firstSeg) subs.add(firstSeg);
  }
  return {
    subfolders: [...subs].sort(),
    items: items.sort((a, b) => b.created_at - a.created_at),
  };
}

/** How many bookmarks live anywhere under a folder path (for the count badge). */
function descendantCount(all: Bookmark[], path: string): number {
  const prefix = path + '/';
  return all.filter((b) => b.folder === path || b.folder.startsWith(prefix)).length;
}
