import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Matches Rust's `DownloadEntry`. */
interface DownloadEntry {
  id: number;
  url: string;
  filename: string;
  path: string;
  size: number;
  ts: number;
}

/**
 * Recent-downloads panel. Pulls the log from Rust on mount and
 * every few seconds so new entries show up while the user is
 * watching the page. Per-row "open" / "show in folder" invoke
 * tauri-plugin-opener via the `downloads_open` / `downloads_reveal`
 * commands. Desktop + mobile share the same layout; mobile just
 * gets tighter padding via the existing breakpoint.
 */
export function Downloads() {
  const [entries, setEntries] = useState<DownloadEntry[]>([]);
  const [error, setError] = useState<string | null>(null);

  async function reload() {
    try {
      const list = await invoke<DownloadEntry[]>('downloads_list', { limit: 200 });
      setEntries(list);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    reload();
    const id = window.setInterval(reload, 3000);
    return () => window.clearInterval(id);
  }, []);

  async function onOpen(path: string) {
    try {
      await invoke('downloads_open', { path });
    } catch (e) {
      setError(String(e));
    }
  }

  async function onReveal(path: string) {
    try {
      await invoke('downloads_reveal', { path });
    } catch (e) {
      setError(String(e));
    }
  }

  async function onClear() {
    if (!confirm('Clear the downloads list? The files on disk are untouched.')) return;
    try {
      await invoke('downloads_clear');
      await reload();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <section className="downloads-page">
      <div className="downloads-page-header">
        <h2 className="settings-title">downloads</h2>
        <div className="downloads-page-actions">
          <button
            className="secondary"
            onClick={onClear}
            disabled={entries.length === 0}
          >
            clear list
          </button>
        </div>
      </div>

      {error && <div className="error-banner">downloads: {error}</div>}

      {entries.length === 0 ? (
        <div className="downloads-page-empty">
          // no downloads yet. files saved from the browser will appear here.
        </div>
      ) : (
        <ul className="downloads-page-list">
          {entries.map((e) => (
            <li key={e.id} className="download-row">
              <div className="download-row-main">
                <div className="download-row-title">{e.filename}</div>
                <div className="download-row-meta">
                  {formatBytes(e.size)} · {formatAge(e.ts)} · <span className="download-row-path">{e.path}</span>
                </div>
                <div className="download-row-url">{e.url}</div>
              </div>
              <div className="download-row-actions">
                <button className="secondary" onClick={() => onOpen(e.path)}>
                  open
                </button>
                <button className="secondary" onClick={() => onReveal(e.path)}>
                  show
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  return `${(mb / 1024).toFixed(2)} GB`;
}

function formatAge(epochSecs: number): string {
  const age = Math.max(0, Date.now() / 1000 - epochSecs);
  if (age < 60) return `${Math.floor(age)}s ago`;
  if (age < 3600) return `${Math.floor(age / 60)}m ago`;
  if (age < 86400) return `${Math.floor(age / 3600)}h ago`;
  return `${Math.floor(age / 86400)}d ago`;
}
