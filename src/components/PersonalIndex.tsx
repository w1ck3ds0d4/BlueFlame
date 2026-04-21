import { useEffect, useState } from 'react';
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
}

export function PersonalIndex() {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<Visit[]>([]);
  const [recent, setRecent] = useState<Visit[]>([]);
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  const [clearing, setClearing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function loadRecent() {
    try {
      const r = await invoke<Visit[]>('personal_recent', { limit: 20 });
      setRecent(r);
      const b = await invoke<Bookmark[]>('bookmark_list');
      setBookmarks(b);
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

  useEffect(() => {
    loadRecent();
  }, []);

  const showing = query.trim() ? results : recent;
  const heading = query.trim() ? '// search results' : '// recent visits';

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
          <ul className="pi-list">
            {bookmarks.map((b) => (
              <li key={b.url} className="pi-row" onClick={() => openInTab(b.url)}>
                <span className="pi-title">{b.title || b.url}</span>
                <span className="pi-url mono">{b.url}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
