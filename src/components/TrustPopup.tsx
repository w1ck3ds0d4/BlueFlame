import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

// Types re-exported so other call sites can still import them from here
// without rewiring imports. The trust panel used to live in-DOM; it's
// now its own child webview loaded at `/?panel=trust` so it can render
// ON TOP of the tab's native webview (DOM z-index can't beat a native
// webview, but another child webview added after the tab can).
export type TrustLabel = 'trusted' | 'ok' | 'suspect' | 'danger';
export type TrustTab = 'overview' | 'malware' | 'scam' | 'vuln';

export interface TrustSignal {
  id: string;
  message: string;
  kind: 'positive' | 'neutral' | 'negative';
  category: string;
}

export interface TrustCategory {
  key: 'malware' | 'scam' | 'vuln';
  name: string;
  score: number;
  label: TrustLabel;
  signals: TrustSignal[];
}

export interface TrustAssessment {
  score: number;
  label: TrustLabel;
  signals: TrustSignal[];
  categories: {
    malware: TrustCategory;
    scam: TrustCategory;
    vuln: TrustCategory;
  };
}

interface TrustSample {
  score: number;
  recorded_at: number;
}

const TABS: { key: TrustTab; label: string }[] = [
  { key: 'overview', label: 'overview' },
  { key: 'malware', label: 'malware' },
  { key: 'scam', label: 'scam' },
  { key: 'vuln', label: 'vuln' },
];

/**
 * Standalone trust panel. Lives in its own child webview spawned by
 * `open_trust_panel` (Rust). Reads `url` + initial `tab` from query
 * params, fetches its own trust assessment + history, and closes via
 * the `close_trust_panel` command. No in-DOM render path anymore.
 */
export function TrustPopup() {
  const params = new URLSearchParams(window.location.search);
  const currentUrl = params.get('url') ?? '';
  const initialTab = (params.get('tab') as TrustTab) ?? 'overview';

  const [tab, setTab] = useState<TrustTab>(
    TABS.some((t) => t.key === initialTab) ? initialTab : 'overview',
  );
  const [assessment, setAssessment] = useState<TrustAssessment | null>(null);
  const [history, setHistory] = useState<TrustSample[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!currentUrl || currentUrl.startsWith('data:') || currentUrl.startsWith('about:')) {
      setError('open a real page to see a trust assessment');
      return;
    }
    setError(null);
    invoke<TrustAssessment>('get_trust', { url: currentUrl })
      .then(setAssessment)
      .catch((e) => setError(String(e)));
  }, [currentUrl]);

  useEffect(() => {
    if (!currentUrl) return;
    let host: string;
    try {
      host = new URL(currentUrl).host;
    } catch {
      return;
    }
    if (!host) return;
    invoke<TrustSample[]>('get_trust_history', { host, limit: 48 })
      .then(setHistory)
      .catch(() => setHistory([]));
  }, [currentUrl, assessment?.score]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        invoke('close_trust_panel').catch(() => undefined);
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const active = assessment ? categoryForTab(assessment, tab) : null;

  return (
    <div className="trust-popup" role="dialog" aria-label="site trust assessment">
      <div className="trust-panel-header">
        <span className="trust-panel-title">site scan</span>
        <button
          className="trust-panel-close"
          onClick={() => {
            invoke('close_trust_panel').catch(() => undefined);
          }}
          aria-label="close"
        >
          ×
        </button>
      </div>

      <div className="trust-tabs" role="tablist">
        {TABS.map((t) => (
          <button
            key={t.key}
            role="tab"
            aria-selected={t.key === tab}
            className={`trust-tab ${t.key === tab ? 'trust-tab-active' : ''}`}
            onClick={() => setTab(t.key)}
          >
            {t.label}
          </button>
        ))}
      </div>

      {error && <div className="trust-panel-error">// {error}</div>}

      {assessment && active && (
        <>
          <div className={`trust-score trust-${active.label}`}>
            <span className="trust-score-num">{active.score}</span>
            <span className="trust-score-label">{active.label}</span>
            {tab === 'overview' && history.length >= 2 && (
              <Sparkline samples={history} />
            )}
          </div>
          <ul className="trust-signals">
            {active.signals.length === 0 && (
              <li className="trust-signal trust-signal-neutral">
                <span className="trust-signal-glyph" aria-hidden>
                  ·
                </span>
                <span className="trust-signal-msg">no signals in this category yet</span>
              </li>
            )}
            {active.signals.map((s) => (
              <li key={s.id} className={`trust-signal trust-signal-${s.kind}`}>
                <span className="trust-signal-glyph" aria-hidden>
                  {signalGlyph(s.kind)}
                </span>
                <span className="trust-signal-msg">{s.message}</span>
              </li>
            ))}
          </ul>
          <div className="trust-panel-footnote">
            // local-only heuristic, not a remote scanner
          </div>
        </>
      )}
    </div>
  );
}

interface ActiveView {
  score: number;
  label: TrustLabel;
  signals: TrustSignal[];
}

function categoryForTab(a: TrustAssessment, t: TrustTab): ActiveView {
  if (t === 'overview') return { score: a.score, label: a.label, signals: a.signals };
  const c = a.categories[t];
  return { score: c.score, label: c.label, signals: c.signals };
}

function signalGlyph(kind: string): string {
  if (kind === 'positive') return '+';
  if (kind === 'negative') return '!';
  return '·';
}

interface SparkProps {
  samples: TrustSample[];
}

function Sparkline({ samples }: SparkProps) {
  const W = 120;
  const H = 28;
  const PAD = 2;
  const n = samples.length;
  if (n < 2) return null;
  const xs = (i: number) => PAD + (i * (W - 2 * PAD)) / (n - 1);
  const ys = (s: number) => H - PAD - (s / 100) * (H - 2 * PAD);
  const d = samples
    .map((s, i) => `${i === 0 ? 'M' : 'L'} ${xs(i).toFixed(1)} ${ys(s.score).toFixed(1)}`)
    .join(' ');
  const last = samples[samples.length - 1];
  return (
    <svg
      className="trust-sparkline"
      viewBox={`0 0 ${W} ${H}`}
      width={W}
      height={H}
      aria-label={`trust history, ${n} samples`}
    >
      <path d={d} fill="none" strokeWidth={1.5} />
      <circle cx={xs(n - 1)} cy={ys(last.score)} r={2} />
    </svg>
  );
}
