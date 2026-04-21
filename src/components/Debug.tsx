import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface DebugEntry {
  ts: number;
  level: string;
  target: string;
  message: string;
}

type LevelFilter = 'all' | 'error' | 'warn' | 'info';

const POLL_MS = 1000;
const DISPLAY_LIMIT = 500;

export function Debug() {
  const [entries, setEntries] = useState<DebugEntry[]>([]);
  const [filter, setFilter] = useState<LevelFilter>('all');
  const [paused, setPaused] = useState(false);
  const [follow, setFollow] = useState(true);
  const [clearing, setClearing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const scrollerRef = useRef<HTMLDivElement | null>(null);

  async function refresh() {
    try {
      const list = await invoke<DebugEntry[]>('get_debug_log', { limit: DISPLAY_LIMIT });
      setEntries(list);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onClear() {
    setClearing(true);
    try {
      await invoke('clear_debug_log');
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setClearing(false);
    }
  }

  useEffect(() => {
    refresh();
    if (paused) return;
    const id = window.setInterval(refresh, POLL_MS);
    return () => window.clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paused]);

  // Auto-scroll to newest when follow is on and entries change.
  useEffect(() => {
    if (!follow) return;
    const el = scrollerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [entries, follow]);

  const shown =
    filter === 'all' ? entries : entries.filter((e) => e.level === filter);
  const counts = {
    error: entries.filter((e) => e.level === 'error').length,
    warn: entries.filter((e) => e.level === 'warn').length,
    info: entries.filter((e) => e.level === 'info').length,
  };

  return (
    <section className="debug">
      <div className="debug-header">
        <h2 className="settings-title">debug monitor</h2>
        <div className="debug-actions">
          <button
            className={`link ${paused ? 'link-on' : ''}`}
            onClick={() => setPaused((p) => !p)}
            title="pause/resume live polling"
          >
            {paused ? 'resume' : 'pause'}
          </button>
          <button
            className={`link ${follow ? 'link-on' : ''}`}
            onClick={() => setFollow((f) => !f)}
            title="auto-scroll to newest"
          >
            follow
          </button>
          <button
            className="secondary"
            onClick={onClear}
            disabled={clearing || entries.length === 0}
          >
            clear
          </button>
        </div>
      </div>

      {error && <div className="error-banner">debug: {error}</div>}

      <div className="debug-filters" role="toolbar" aria-label="level filter">
        <FilterChip value="all" current={filter} set={setFilter} label="all" count={entries.length} />
        <FilterChip value="error" current={filter} set={setFilter} label="error" count={counts.error} />
        <FilterChip value="warn" current={filter} set={setFilter} label="warn" count={counts.warn} />
        <FilterChip value="info" current={filter} set={setFilter} label="info" count={counts.info} />
      </div>

      <div className="debug-scroll" ref={scrollerRef}>
        {shown.length === 0 ? (
          <div className="debug-empty">
            // {filter === 'all' ? 'no events yet' : `no ${filter} events`}
          </div>
        ) : (
          <ul className="debug-list">
            {shown.map((e, i) => (
              <li key={`${e.ts}-${i}`} className={`debug-row debug-${e.level}`}>
                <span className="debug-ts">{fmtTime(e.ts)}</span>
                <span className="debug-level">{fmtLevel(e.level)}</span>
                <span className="debug-target" title={e.target}>
                  {e.target}
                </span>
                <span className="debug-msg">{e.message}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

interface FilterProps {
  value: LevelFilter;
  current: LevelFilter;
  set: (v: LevelFilter) => void;
  label: string;
  count: number;
}

function FilterChip({ value, current, set, label, count }: FilterProps) {
  return (
    <button
      className={`debug-filter-chip ${current === value ? 'debug-filter-on' : ''}`}
      onClick={() => set(value)}
    >
      {label} <span className="debug-filter-count">{count}</span>
    </button>
  );
}

function fmtTime(epochSecs: number): string {
  const d = new Date(epochSecs * 1000);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  const ss = String(d.getSeconds()).padStart(2, '0');
  return `${hh}:${mm}:${ss}`;
}

function fmtLevel(level: string): string {
  switch (level) {
    case 'error':
      return 'err ';
    case 'warn':
      return 'warn';
    case 'info':
      return 'info';
    case 'debug':
      return 'dbg ';
    case 'trace':
      return 'trc ';
    default:
      return level.padEnd(4).slice(0, 4);
  }
}
