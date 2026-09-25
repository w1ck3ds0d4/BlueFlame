import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

/** Matches Rust's `DownloadEntry`. */
interface DownloadEntry {
  id: number;
  url: string;
  filename: string;
  path: string;
  size: number;
  ts: number;
}

/** Matches Rust's `DownloadProgress`, emitted on `blueflame:download-progress`. */
interface DownloadProgress {
  id: number;
  url: string;
  bytes_written: number;
  total: number | null;
  done: boolean;
}

const PROGRESS_EVENT = 'blueflame:download-progress';

/**
 * Recent-downloads panel. Pulls the log from Rust on mount and
 * every few seconds so new entries show up while the user is
 * watching the page. Per-row "open" / "show in folder" invoke
 * tauri-plugin-opener via the `downloads_open` / `downloads_reveal`
 * commands. Desktop + mobile share the same layout; mobile just
 * gets tighter padding via the existing breakpoint.
 *
 * In-flight downloads stream to disk in Rust, so this panel also
 * subscribes to `blueflame:download-progress` and shows a live row
 * per active download with a cancel button (`downloads_cancel`).
 * The row drops off once the backend reports `done: true`; a
 * `reload()` then picks up the finished entry from the log (or, on
 * cancel/failure, no new entry appears at all).
 */
export function Downloads() {
  const [entries, setEntries] = useState<DownloadEntry[]>([]);
  const [active, setActive] = useState<Map<number, DownloadProgress>>(new Map());
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

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<DownloadProgress>(PROGRESS_EVENT, (event) => {
      const p = event.payload;
      setActive((prev) => {
        const next = new Map(prev);
        if (p.done) {
          next.delete(p.id);
        } else {
          next.set(p.id, p);
        }
        return next;
      });
      if (p.done) reload();
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  async function onCancel(id: number) {
    try {
      await invoke('downloads_cancel', { id });
    } catch (e) {
      setError(String(e));
    }
  }

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

      {active.size > 0 && (
        <ul className="downloads-page-list downloads-page-active-list">
          {Array.from(active.values()).map((p) => (
            <li key={p.id} className="download-row download-row-active">
              <div className="download-row-main">
                <div className="download-row-title">{filenameFromUrl(p.url)}</div>
                <div className="download-row-meta">
                  {formatBytes(p.bytes_written)}
                  {p.total ? ` of ${formatBytes(p.total)}` : ''} · downloading
                </div>
                <div className="download-row-progress-track">
                  <div
                    className="download-row-progress-fill"
                    style={
                      p.total
                        ? { width: `${Math.min(100, (p.bytes_written / p.total) * 100)}%` }
                        : { width: '100%' }
                    }
                  />
                </div>
              </div>
              <div className="download-row-actions">
                <button className="secondary" onClick={() => onCancel(p.id)}>
                  cancel
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}

      {entries.length === 0 && active.size === 0 ? (
        <div className="downloads-page-empty">
          // no downloads yet. files saved from the browser will appear here.
        </div>
      ) : entries.length === 0 ? null : (
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

/** The progress event only carries the source URL, not the final
 * sanitized filename (that's decided when the file lands), so the
 * in-progress row shows a best-effort name from the URL path. */
function filenameFromUrl(url: string): string {
  try {
    const path = new URL(url).pathname;
    const last = path.split('/').filter(Boolean).pop();
    return last ? decodeURIComponent(last) : url;
  } catch {
    return url;
  }
}
