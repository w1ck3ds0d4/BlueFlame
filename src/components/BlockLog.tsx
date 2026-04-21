import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface BlockedEntry {
  ts: number;
  url: string;
}

export function BlockLog() {
  const [entries, setEntries] = useState<BlockedEntry[]>([]);
  const [paused, setPaused] = useState(false);
  const [clearing, setClearing] = useState(false);

  async function refresh() {
    try {
      const list = await invoke<BlockedEntry[]>('get_recent_blocks', { limit: 100 });
      setEntries(list);
    } catch {
      // Rendering-only - swallow transient errors rather than cluttering the UI
    }
  }

  async function onClear() {
    setClearing(true);
    try {
      await invoke('clear_block_log');
      await refresh();
    } finally {
      setClearing(false);
    }
  }

  useEffect(() => {
    refresh();
    if (paused) return;
    const id = setInterval(refresh, 1000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paused]);

  return (
    <section className="block-log">
      <div className="block-log-header">
        <h3>recent blocks</h3>
        <div className="block-log-actions">
          <button className="link" onClick={() => setPaused((p) => !p)}>
            {paused ? 'resume' : 'pause'}
          </button>
          <button className="link" onClick={onClear} disabled={clearing || entries.length === 0}>
            clear
          </button>
        </div>
      </div>

      {entries.length === 0 ? (
        <div className="block-log-empty">
          // no blocks yet - browse with trackers and they will appear here live
        </div>
      ) : (
        <ul className="block-log-list">
          {entries.map((e, i) => (
            <li key={`${e.ts}-${i}`} className="block-log-row">
              <span className="block-log-time mono">{fmtTime(e.ts)}</span>
              <span className="block-log-url mono" title={e.url}>
                {e.url}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function fmtTime(epochSecs: number): string {
  const d = new Date(epochSecs * 1000);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  const ss = String(d.getSeconds()).padStart(2, '0');
  return `${hh}:${mm}:${ss}`;
}
