import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { BlockLog } from './BlockLog';

interface ProxyStatus {
  running: boolean;
  port: number;
  filters_enabled: boolean;
  tor_bootstrap: string;
}

interface Stats {
  requests_total: number;
  requests_blocked: number;
  bytes_saved: number;
}

interface SystemSummary {
  uptime_secs: number;
  patterns_active: number;
  lists_total: number;
  last_refresh_secs: number | null;
}

interface BlockedEntry {
  ts: number;
  url: string;
}

interface Props {
  status: ProxyStatus;
  stats: Stats;
  onToggled: () => void;
}

const SPARK_WINDOW_SECS = 60;
const TOP_HOSTS = 6;

export function Dashboard({ status, stats, onToggled }: Props) {
  const [summary, setSummary] = useState<SystemSummary | null>(null);
  const [blocks, setBlocks] = useState<BlockedEntry[]>([]);
  const [nowSec, setNowSec] = useState(() => Math.floor(Date.now() / 1000));

  async function toggleFilters() {
    if (status.filters_enabled) {
      await invoke('disable_filters');
    } else {
      await invoke('enable_filters');
    }
    onToggled();
  }

  useEffect(() => {
    async function tick() {
      try {
        const [s, b] = await Promise.all([
          invoke<SystemSummary>('get_system_summary'),
          invoke<BlockedEntry[]>('get_recent_blocks', { limit: 500 }),
        ]);
        setSummary(s);
        setBlocks(b);
        setNowSec(Math.floor(Date.now() / 1000));
      } catch {
        /* refreshing-only */
      }
    }
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, []);

  const blockedPct =
    stats.requests_total > 0
      ? ((stats.requests_blocked / stats.requests_total) * 100).toFixed(1)
      : '0.0';

  const spark = sparkline(blocks, nowSec);
  const peak = spark.peak;
  const topHosts = rankHosts(blocks, TOP_HOSTS);

  return (
    <>
      <section className="system-line" aria-label="system summary">
        <span className="kv">
          <span className="k">uptime</span>{' '}
          <span className="v">{summary ? fmtUptime(summary.uptime_secs) : '-'}</span>
        </span>
        <span className="dot">·</span>
        <span className="kv">
          <span className="k">lists</span>{' '}
          <span className="v">{summary ? summary.lists_total : '-'}</span>
        </span>
        <span className="dot">·</span>
        <span className="kv">
          <span className="k">patterns</span>{' '}
          <span className="v">
            {summary ? summary.patterns_active.toLocaleString() : '-'}
          </span>
        </span>
        <span className="dot">·</span>
        <span className="kv">
          <span className="k">last refresh</span>{' '}
          <span className="v">
            {summary ? fmtRelative(summary.last_refresh_secs, nowSec) : '-'}
          </span>
        </span>
        {status.tor_bootstrap && (
          <>
            <span className="dot">·</span>
            <span className="kv">
              <span className="k">tor</span>{' '}
              <span className="v">{fmtTorState(status.tor_bootstrap)}</span>
            </span>
          </>
        )}
      </section>

      <section className="dashboard">
        <div className="card">
          <div className="label">requests</div>
          <div className="value">{stats.requests_total.toLocaleString()}</div>
        </div>
        <div className="card">
          <div className="label">blocked</div>
          <div className="value accent">{stats.requests_blocked.toLocaleString()}</div>
          <div className="sub">{blockedPct}% of total</div>
        </div>
        <div className="card">
          <div className="label">bytes saved</div>
          <div className="value">{formatBytes(stats.bytes_saved)}</div>
        </div>
      </section>

      <section className="spark-strip" aria-label="activity last 60s">
        <div className="spark-header">
          <span className="spark-label">activity / blocks per second (last 60s)</span>
          <span className="spark-peak">peak {peak}/s</span>
        </div>
        <div className="spark-line" aria-hidden>
          {spark.glyphs}
        </div>
      </section>

      <section className="actions">
        <button className="primary" onClick={toggleFilters} disabled={!status.running}>
          {status.filters_enabled ? 'disable filters' : 'enable filters'}
        </button>
      </section>

      <section className="top-blocked">
        <div className="top-blocked-header">top blocked // last {blocks.length} events</div>
        {topHosts.length === 0 ? (
          <div className="top-blocked-empty">// no blocks yet</div>
        ) : (
          <ul className="top-blocked-list">
            {topHosts.map(([host, count]) => {
              const barMax = topHosts[0][1];
              const width = Math.max(1, Math.round((count / barMax) * 20));
              return (
                <li key={host} className="top-blocked-row">
                  <span className="tb-host" title={host}>
                    {host}
                  </span>
                  <span className="tb-bar" aria-hidden>
                    {'\u2593'.repeat(width)}
                  </span>
                  <span className="tb-count">{count}</span>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      <BlockLog />

      <section className="shortcuts-hint" aria-label="keyboard shortcuts">
        <span className="sh-title">// keybinds</span>
        <span className="sh-pair">
          <kbd>^L</kbd> url
        </span>
        <span className="sh-pair">
          <kbd>^T</kbd> new tab
        </span>
        <span className="sh-pair">
          <kbd>^W</kbd> close
        </span>
        <span className="sh-pair">
          <kbd>^R</kbd> reload
        </span>
        <span className="sh-pair">
          <kbd>^F</kbd> find in page
        </span>
        <span className="sh-pair">
          <kbd>^D</kbd> bookmark
        </span>
        <span className="sh-pair">
          <kbd>^Tab</kbd> next
        </span>
        <span className="sh-pair">
          <kbd>^1..9</kbd> jump
        </span>
        <span className="sh-pair">
          <kbd>^,</kbd> settings
        </span>
        <span className="sh-pair">
          <kbd>^⇧D</kbd> dashboard
        </span>
      </section>
    </>
  );
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

function fmtUptime(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  const rem = mins % 60;
  if (hrs < 24) return rem > 0 ? `${hrs}h${rem}m` : `${hrs}h`;
  const days = Math.floor(hrs / 24);
  return `${days}d${hrs % 24}h`;
}

function fmtTorState(s: string): string {
  if (s === 'running') return 'bootstrapping...';
  if (s === 'ready') return 'ready';
  if (s.startsWith('failed')) {
    const msg = s.slice('failed:'.length).trim();
    return msg ? `failed (${msg})` : 'failed';
  }
  return s;
}

function fmtRelative(tsSec: number | null, nowSec: number): string {
  if (!tsSec) return 'never';
  const age = Math.max(0, nowSec - tsSec);
  if (age < 60) return `${age}s ago`;
  if (age < 3600) return `${Math.floor(age / 60)}m ago`;
  if (age < 86400) return `${Math.floor(age / 3600)}h ago`;
  return `${Math.floor(age / 86400)}d ago`;
}

function hostOf(url: string): string {
  try {
    return new URL(url).host.replace(/^www\./, '');
  } catch {
    return url;
  }
}

function rankHosts(blocks: BlockedEntry[], n: number): [string, number][] {
  const counts = new Map<string, number>();
  for (const b of blocks) {
    const h = hostOf(b.url);
    counts.set(h, (counts.get(h) ?? 0) + 1);
  }
  return Array.from(counts.entries())
    .sort((a, b) => b[1] - a[1])
    .slice(0, n);
}

function sparkline(blocks: BlockedEntry[], nowSec: number): { glyphs: string; peak: number } {
  const buckets = new Array<number>(SPARK_WINDOW_SECS).fill(0);
  const start = nowSec - SPARK_WINDOW_SECS + 1;
  for (const b of blocks) {
    if (b.ts < start || b.ts > nowSec) continue;
    const i = Math.max(0, Math.min(SPARK_WINDOW_SECS - 1, b.ts - start));
    buckets[i] = (buckets[i] ?? 0) + 1;
  }
  const peak = buckets.reduce((m, v) => (v > m ? v : m), 0);
  const glyphs = ['\u2581', '\u2582', '\u2583', '\u2584', '\u2585', '\u2586', '\u2587', '\u2588'];
  // Idle seconds render as a non-breaking space so the gaps actually look
  // empty and the real activity bars stand out. A sqrt curve maps
  // mid-range counts to visible heights instead of collapsing them all
  // to the minimum block when a single spike dominates the peak.
  const denom = Math.sqrt(Math.max(peak, 1));
  const line = buckets
    .map((c) => {
      if (c <= 0) return '\u00A0';
      const ratio = Math.sqrt(c) / denom;
      const idx = Math.max(
        0,
        Math.min(glyphs.length - 1, Math.floor(ratio * (glyphs.length - 1))),
      );
      return glyphs[idx];
    })
    .join('');
  return { glyphs: line, peak };
}
