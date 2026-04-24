import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Visit {
  id: number;
  url: string;
  title: string;
  visited_at: number;
  visit_count: number;
}

interface Bookmark {
  url: string;
  title: string;
  created_at: number;
  folder: string;
}

export function PersonalIndex() {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Visit[]>([]);
  const [recent, setRecent] = useState<Visit[]>([]);
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const [folders, setFolders] = useState<string[]>([]);
  const [clearing, setClearing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [moveTarget, setMoveTarget] = useState<Bookmark | null>(null);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  async function loadRecent() {
    try {
      const r = await invoke<Visit[]>('personal_recent', { limit: 20 });
      setRecent(r);
      const b = await invoke<Bookmark[]>('bookmark_list');
      setBookmarks(b);
      const f = await invoke<string[]>('bookmark_folders');
      setFolders(f);
    } catch (e) {
      setError(String(e));
    }
  }

  async function search(q: string) {
    setQuery(q);
    if (!q.trim()) {
      setResults([]);
      return;
    }
    try {
      const r = await invoke<Visit[]>('personal_search', { query: q, limit: 30 });
      setResults(r);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onClear() {
    if (!confirm('Wipe all browsing history? Bookmarks stay.')) return;
    setClearing(true);
    try {
      await invoke('personal_clear_history');
      await loadRecent();
      setResults([]);
    } catch (e) {
      setError(String(e));
    } finally {
      setClearing(false);
    }
  }

  async function openInTab(url: string) {
    try {
      await invoke('browser_open_tab', { url });
    } catch (e) {
      setError(String(e));
    }
  }

  async function moveBookmark(url: string, folder: string) {
    try {
      await invoke('bookmark_set_folder', { url, folder });
      await loadRecent();
      setMoveTarget(null);
    } catch (e) {
      setError(String(e));
    }
  }

  function toggleFolder(name: string) {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  }

  useEffect(() => {
    loadRecent();
  }, []);

  const showing = query.trim() ? results : recent;
  const heading = query.trim() ? '// search results' : '// recent visits';

  const grouped = useMemo(() => groupBookmarksByFolder(bookmarks), [bookmarks]);

  return (
    <div className="panel">
      <div className="panel-header">
        <h3>personal search</h3>
        <button
          className="secondary"
          onClick={onClear}
          disabled={clearing || recent.length === 0}
        >
          clear history
        </button>
      </div>

      <div className="panel-note">
        local-only: browsing history stays on disk under{' '}
        <code className="mono">personal.sqlite</code>. never leaves this machine.
      </div>

      <input
        type="text"
        className="url-input mono"
        placeholder="grep history..."
        value={query}
        onChange={(e) => search(e.currentTarget.value)}
        spellCheck={false}
        autoCorrect="off"
      />

      {error && <div className="error">{error}</div>}

      <div className="pi-section">
        <div className="pi-section-title">{heading}</div>
        {showing.length === 0 ? (
          <div className="pi-empty">
            {query.trim() ? '// no matches' : '// no visits yet'}
          </div>
        ) : (
          <ul className="pi-list">
            {showing.map((v) => (
              <li key={v.id} className="pi-row" onClick={() => openInTab(v.url)}>
                <span className="pi-title">{v.title || v.url}</span>
                <span className="pi-url mono">{v.url}</span>
                <span className="pi-meta">{v.visit_count}×</span>
              </li>
            ))}
          </ul>
        )}
      </div>

      {bookmarks.length > 0 && (
        <div className="pi-section">
          <div className="pi-section-title">// bookmarks ({bookmarks.length})</div>
          {grouped.map((g) => {
            const isCollapsed = collapsed.has(g.folder);
            return (
              <div key={g.folder || '__root__'} className="pi-folder">
                {g.folder && (
                  <button
                    type="button"
                    className="pi-folder-header"
                    onClick={() => toggleFolder(g.folder)}
                    aria-expanded={!isCollapsed}
                  >
                    <span className="pi-folder-caret" aria-hidden>
                      {isCollapsed ? '▸' : '▾'}
                    </span>
                    <span className="pi-folder-name">{g.folder}</span>
                    <span className="pi-folder-count">({g.bookmarks.length})</span>
                  </button>
                )}
                {!isCollapsed && (
                  <ul className="pi-list">
                    {g.bookmarks.map((b) => (
                      <li
                        key={b.url}
                        className="pi-row pi-bookmark-row"
                        onClick={() => openInTab(b.url)}
                      >
                        <span className="pi-title">{b.title || b.url}</span>
                        <span className="pi-url mono">{b.url}</span>
                        <button
                          className="pi-move-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            setMoveTarget(b);
                          }}
                          title="Move to folder"
                        >
                          move
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            );
          })}
        </div>
      )}

      {moveTarget && (
        <MoveBookmarkDialog
          bookmark={moveTarget}
          folders={folders}
          onCancel={() => setMoveTarget(null)}
          onMove={(folder) => moveBookmark(moveTarget.url, folder)}
        />
      )}
    </div>
  );
}

interface FolderGroup {
  folder: string;
  bookmarks: Bookmark[];
}

function groupBookmarksByFolder(bookmarks: Bookmark[]): FolderGroup[] {
  const buckets = new Map<string, Bookmark[]>();
  for (const b of bookmarks) {
    const key = b.folder?.trim() ?? '';
    if (!buckets.has(key)) buckets.set(key, []);
    buckets.get(key)!.push(b);
  }
  // Root first, then folders alphabetically.
  const groups: FolderGroup[] = [];
  if (buckets.has('')) groups.push({ folder: '', bookmarks: buckets.get('')! });
  const namedKeys = [...buckets.keys()].filter((k) => k !== '').sort();
  for (const k of namedKeys) groups.push({ folder: k, bookmarks: buckets.get(k)! });
  return groups;
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
          <label htmlFor="pi-new-folder">new folder (slash for nesting, e.g. Work/Dev)</label>
          <div className="pi-modal-new-row">
            <input
              id="pi-new-folder"
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
