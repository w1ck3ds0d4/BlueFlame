import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Matches the Rust `MetricsSnapshot` struct exposed by `get_system_metrics`. */
interface MetricsSnapshot {
  ts: number;
  pid: number;
  uptime_secs: number;
  rss_bytes: number;
  cpu_percent: number;
  thread_count: number | null;
  process_count: number;
  proxy_requests_total: number;
  proxy_requests_blocked: number;
  proxy_bytes_saved: number;
  tab_count: number;
  private_tab_count: number;
}

const POLL_MS = 2000;
/** How many samples to keep for the sparkline trend line. 60 @ 2s = 2 min. */
const HISTORY = 60;

/**
 * Live-updating resource panel for the BlueFlame process tree + the
 * proxy. Pollings every 2s, keeps the last `HISTORY` samples for the
 * per-field sparkline. Mobile + desktop share the same layout (single
 * column of stat cards); desktop just gets more horizontal room.
 */
export function Metrics() {
  const [current, setCurrent] = useState<MetricsSnapshot | null>(null);
  const [paused, setPaused] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const rssHistoryRef = useRef<number[]>([]);
  const cpuHistoryRef = useRef<number[]>([]);
  const reqsHistoryRef = useRef<number[]>([]);
  // `reqs` is cumulative (ever-increasing), so we track the delta
  // per sample for the sparkline rather than the raw counter.
  const lastReqsTotalRef = useRef<number | null>(null);
  // Force re-render when histories update since refs don't trigger.
  const [, setTick] = useState(0);

  useEffect(() => {
    if (paused) return;
    let cancelled = false;
    async function pull() {
      try {
        const snap = await invoke<MetricsSnapshot>('get_system_metrics');
        if (cancelled) return;
        setCurrent(snap);
        setError(null);

        rssHistoryRef.current = pushBounded(rssHistoryRef.current, snap.rss_bytes);
        cpuHistoryRef.current = pushBounded(cpuHistoryRef.current, snap.cpu_percent);

        const lastReqs = lastReqsTotalRef.current;
        const delta =
          lastReqs == null ? 0 : Math.max(0, snap.proxy_requests_total - lastReqs);
        lastReqsTotalRef.current = snap.proxy_requests_total;
        reqsHistoryRef.current = pushBounded(reqsHistoryRef.current, delta);

        setTick((t) => t + 1);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    }
    pull();
    const id = window.setInterval(pull, POLL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [paused]);

  return (
    <section className="metrics-page">
      <div className="metrics-page-header">
        <h2 className="settings-title">metrics</h2>
        <div className="metrics-page-actions">
          <button
            className={`link ${paused ? 'link-on' : ''}`}
            onClick={() => setPaused((p) => !p)}
            title="pause/resume live polling"
          >
            {paused ? 'resume' : 'pause'}
          </button>
        </div>
      </div>

      {error && <div className="error-banner">metrics: {error}</div>}

      {!current ? (
        <div className="metrics-page-empty">// sampling...</div>
      ) : (
        <div className="metrics-cards">
          <MetricCard
            label="memory (rss)"
            value={formatBytes(current.rss_bytes)}
            sub={`${current.process_count} processes`}
            spark={rssHistoryRef.current}
          />
          <MetricCard
            label="cpu"
            value={`${current.cpu_percent.toFixed(1)}%`}
            sub={
              current.thread_count != null
                ? `${current.thread_count} threads`
                : 'threads n/a'
            }
            spark={cpuHistoryRef.current}
            max={cpuMaxFor(cpuHistoryRef.current)}
          />
          <MetricCard
            label="req/s (proxy)"
            value={formatRate(reqsHistoryRef.current)}
            sub={`${current.proxy_requests_total.toLocaleString()} total`}
            spark={reqsHistoryRef.current}
          />
          <MetricCard
            label="requests blocked"
            value={current.proxy_requests_blocked.toLocaleString()}
            sub={`${formatBlockedRatio(current)} of all`}
          />
          <MetricCard
            label="bytes saved"
            value={formatBytes(current.proxy_bytes_saved)}
            sub="by filters"
          />
          <MetricCard
            label="tabs open"
            value={current.tab_count.toString()}
            sub={
              current.private_tab_count > 0
                ? `${current.private_tab_count} private`
                : 'no private'
            }
          />
          <MetricCard
            label="process uptime"
            value={formatDuration(current.uptime_secs)}
            sub={`pid ${current.pid}`}
          />
        </div>
      )}
    </section>
  );
}

interface CardProps {
  label: string;
  value: string;
  sub?: string;
  spark?: number[];
  /** Upper bound for the sparkline. Defaults to max-of-samples. */
  max?: number;
}

function MetricCard({ label, value, sub, spark, max }: CardProps) {
  return (
    <div className="metric-card">
      <div className="metric-card-label">{label}</div>
      <div className="metric-card-value">{value}</div>
      {sub && <div className="metric-card-sub">{sub}</div>}
      {spark && spark.length > 1 && (
        <div className="metric-card-spark" aria-hidden>
          {renderSparkline(spark, max)}
        </div>
      )}
    </div>
  );
}

// ── Pure helpers ──────────────────────────────────────────────────

function pushBounded(arr: number[], value: number): number[] {
  const next = arr.length >= HISTORY ? arr.slice(arr.length - HISTORY + 1) : arr.slice();
  next.push(value);
  return next;
}

/** 8-step block-char ramp. Empty space for "no value", full block for peak. */
const SPARK_CHARS = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

function renderSparkline(values: number[], explicitMax?: number): string {
  if (values.length === 0) return '';
  const lo = 0;
  const hi = Math.max(explicitMax ?? Math.max(...values), 1);
  const range = hi - lo || 1;
  return values
    .map((v) => {
      const norm = Math.max(0, Math.min(1, (v - lo) / range));
      const idx = Math.min(SPARK_CHARS.length - 1, Math.floor(norm * SPARK_CHARS.length));
      return SPARK_CHARS[idx];
    })
    .join('');
}

/**
 * CPU sparkline upper bound. Auto-scales to 100% for single-core
 * workloads and 200/400/etc for multi-core so the line doesn't
 * flatline at the top on an 8-core machine that hits 750%.
 */
function cpuMaxFor(history: number[]): number {
  if (history.length === 0) return 100;
  const peak = Math.max(...history);
  if (peak <= 100) return 100;
  // Round up to the next 100% step so the line has headroom.
  return Math.ceil(peak / 100) * 100;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const kb = bytes / 1024;
  if (kb < 1024) return `${kb.toFixed(1)} KB`;
  const mb = kb / 1024;
  if (mb < 1024) return `${mb.toFixed(1)} MB`;
  const gb = mb / 1024;
  return `${gb.toFixed(2)} GB`;
}

function formatDuration(secs: number): string {
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  if (m < 60) return `${m}m ${s}s`;
  const h = Math.floor(m / 60);
  const mm = m % 60;
  if (h < 24) return `${h}h ${mm}m`;
  const d = Math.floor(h / 24);
  const hh = h % 24;
  return `${d}d ${hh}h`;
}

function formatRate(deltas: number[]): string {
  if (deltas.length === 0) return '0 / s';
  // Average the last few deltas (divided by poll period in seconds)
  // so a single spike doesn't dominate the reading.
  const window = deltas.slice(-5);
  const avgPerPoll = window.reduce((a, b) => a + b, 0) / window.length;
  const perSec = avgPerPoll / (POLL_MS / 1000);
  return `${perSec.toFixed(1)} / s`;
}

function formatBlockedRatio(s: MetricsSnapshot): string {
  if (s.proxy_requests_total === 0) return '0%';
  const pct = (s.proxy_requests_blocked / s.proxy_requests_total) * 100;
  return `${pct.toFixed(1)}%`;
}
