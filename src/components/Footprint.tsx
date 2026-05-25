import { useCallback, useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Mirror of `commands::FootprintSummary` on the Rust side. */
interface FootprintSummary {
  proxy_running: boolean;
  proxy_port: number;
  filters_enabled: boolean;
  tor_bootstrap: string;
  upstream_applied: string;
  requests_total: number;
  requests_blocked: number;
  bytes_saved: number;
  history_count: number;
  bookmarks_count: number;
  downloads_count: number;
  block_log_count: number;
  tabs_total: number;
  tabs_private: number;
  filter_lists_active: number;
  dns_provider: string;
  dns_doh_active: boolean;
}

/** Mirror of `commands::ShredOutcome`. */
interface ShredOutcome {
  history: boolean;
  downloads: boolean;
  block_log: boolean;
  stats: boolean;
  browsing_data: boolean;
  errors: string[];
}

/** Browser-side fingerprint signals - what every page sees in `navigator`
 *  + `screen` + a canvas paint. Collected locally so the user can read off
 *  exactly what they're exposing to the network. */
interface IdentitySignals {
  userAgent: string;
  language: string;
  timezone: string;
  screen: string;
  hardwareConcurrency: number;
  deviceMemoryGb: number | null;
  canvasHash: string;
  webglRenderer: string;
  doNotTrack: string;
  cookieEnabled: boolean;
}

/** Cheap stable hash for the canvas fingerprint. Same idea every commercial
 *  fingerprinter uses: render a string with hinted glyphs, read the pixel
 *  data, fold into a hex digest. We do an FNV-1a fold over the data URL so
 *  the user sees a short, comparable hash without us shipping a real crypto
 *  hash library to the bundle. */
function fnv1aHex(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  // Force unsigned, then pad to 8 hex chars.
  return (h >>> 0).toString(16).padStart(8, '0');
}

function collectIdentitySignals(): IdentitySignals {
  let canvasHash = 'n/a';
  try {
    const c = document.createElement('canvas');
    c.width = 220;
    c.height = 36;
    const ctx = c.getContext('2d');
    if (ctx) {
      // The exact paint matters less than that it's stable per-machine:
      // text rendering hinges on font fallback + AA + subpixel placement,
      // all of which encode the GPU + OS + font config.
      ctx.textBaseline = 'top';
      ctx.font = "13px 'Arial'";
      ctx.fillStyle = '#f55';
      ctx.fillRect(0, 0, 220, 36);
      ctx.fillStyle = '#039';
      ctx.fillText('BlueFlame fingerprint check 🔥', 2, 2);
      ctx.strokeStyle = 'rgba(0,200,255,0.4)';
      ctx.beginPath();
      ctx.arc(180, 18, 12, 0, Math.PI * 2);
      ctx.stroke();
      canvasHash = fnv1aHex(c.toDataURL());
    }
  } catch {
    /* canvas disabled / blocked - leave as n/a */
  }

  let webglRenderer = 'n/a';
  try {
    const gl = document
      .createElement('canvas')
      .getContext('webgl') as WebGLRenderingContext | null;
    if (gl) {
      const ext = gl.getExtension('WEBGL_debug_renderer_info');
      if (ext) {
        webglRenderer = String(gl.getParameter(ext.UNMASKED_RENDERER_WEBGL));
      } else {
        webglRenderer = String(gl.getParameter(gl.RENDERER));
      }
    }
  } catch {
    /* WebGL blocked */
  }

  let timezone = 'unknown';
  try {
    timezone = Intl.DateTimeFormat().resolvedOptions().timeZone || 'unknown';
  } catch {
    /* Intl broken - keep default */
  }

  const dpr = window.devicePixelRatio || 1;
  const screenStr = `${screen.width}x${screen.height} @${dpr}x`;
  const navAny = navigator as Navigator & { deviceMemory?: number };
  const dnt = (navigator as Navigator & { doNotTrack?: string | null }).doNotTrack;

  return {
    userAgent: navigator.userAgent,
    language: navigator.language || (navigator.languages && navigator.languages[0]) || 'unknown',
    timezone,
    screen: screenStr,
    hardwareConcurrency: navigator.hardwareConcurrency || 0,
    deviceMemoryGb: typeof navAny.deviceMemory === 'number' ? navAny.deviceMemory : null,
    canvasHash,
    webglRenderer,
    doNotTrack: dnt === '1' || dnt === 'yes' ? 'on' : 'off',
    cookieEnabled: navigator.cookieEnabled,
  };
}

function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function routeLabel(s: FootprintSummary): { label: string; tone: 'on' | 'warn' | 'off' } {
  if (s.tor_bootstrap === 'ready' || s.upstream_applied.startsWith('built-in-tor')) {
    return { label: `tor (${s.upstream_applied})`, tone: 'on' };
  }
  if (s.tor_bootstrap === 'running') {
    return { label: 'tor: bootstrapping', tone: 'warn' };
  }
  if (s.tor_bootstrap.startsWith('failed')) {
    return { label: s.tor_bootstrap, tone: 'off' };
  }
  if (s.upstream_applied.startsWith('socks5')) {
    return { label: s.upstream_applied, tone: 'on' };
  }
  return { label: 'direct (no Tor / SOCKS)', tone: 'warn' };
}

export function Footprint() {
  const [summary, setSummary] = useState<FootprintSummary | null>(null);
  const [identity] = useState<IdentitySignals>(() => collectIdentitySignals());
  const [shredding, setShredding] = useState(false);
  const [outcome, setOutcome] = useState<ShredOutcome | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const s = await invoke<FootprintSummary>('get_footprint');
      setSummary(s);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    refresh();
    // Refresh every 4s so the counters track live blocking activity
    // without hammering SQLite or the proxy state lock.
    const id = window.setInterval(refresh, 4000);
    return () => window.clearInterval(id);
  }, [refresh]);

  async function shred() {
    setShredding(true);
    setOutcome(null);
    try {
      const r = await invoke<ShredOutcome>('shred_footprint');
      setOutcome(r);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setShredding(false);
      setConfirming(false);
      await refresh();
    }
  }

  const route = useMemo(() => (summary ? routeLabel(summary) : null), [summary]);

  return (
    <div className="footprint-page">
      <header className="footprint-page-header">
        <h2 className="footprint-page-title">footprint</h2>
        <button className="footprint-refresh-btn" onClick={refresh} disabled={!summary}>
          refresh
        </button>
      </header>

      <p className="footprint-page-blurb">
        what BlueFlame can see about your current session - everything the network and the
        local machine knows. shred clears local browsing state without touching bookmarks
        or settings.
      </p>

      {error && <div className="footprint-error">{error}</div>}

      <section className="footprint-section">
        <h3 className="footprint-section-title">network</h3>
        <div className="footprint-rows">
          <Row
            k="route"
            v={route?.label ?? '…'}
            tone={route?.tone}
            note="where your traffic exits"
          />
          <Row
            k="filter proxy"
            v={
              summary
                ? summary.proxy_running && summary.filters_enabled
                  ? `on :${summary.proxy_port}`
                  : summary?.proxy_running
                    ? `off :${summary.proxy_port}`
                    : 'down'
                : '…'
            }
            tone={
              summary && summary.proxy_running && summary.filters_enabled
                ? 'on'
                : summary && summary.proxy_running
                  ? 'warn'
                  : 'off'
            }
          />
          <Row
            k="trackers blocked"
            v={summary ? `${summary.requests_blocked.toLocaleString()} / ${summary.requests_total.toLocaleString()} requests` : '…'}
          />
          <Row
            k="bytes saved"
            v={summary ? fmtBytes(summary.bytes_saved) : '…'}
            note="estimated from blocked response sizes"
          />
          <Row
            k="active filter patterns"
            v={summary ? summary.filter_lists_active.toLocaleString() : '…'}
          />
          <Row
            k="dns"
            v={summary ? summary.dns_provider : '…'}
            tone={
              summary
                ? summary.dns_doh_active
                  ? 'on'
                  : summary.upstream_applied !== 'direct'
                    ? 'on'
                    : 'warn'
                : undefined
            }
            note={
              summary && !summary.dns_doh_active && summary.upstream_applied === 'direct'
                ? 'system resolver - hostnames leak to your ISP. enable DoH in Settings.'
                : summary && !summary.dns_doh_active && summary.upstream_applied !== 'direct'
                  ? 'tor / socks upstream resolves through the tunnel'
                  : undefined
            }
          />
        </div>
      </section>

      <section className="footprint-section">
        <h3 className="footprint-section-title">identity (what every page sees)</h3>
        <div className="footprint-rows">
          <Row k="user-agent" v={identity.userAgent} wrap />
          <Row k="language" v={identity.language} />
          <Row k="timezone" v={identity.timezone} />
          <Row k="screen" v={identity.screen} />
          <Row k="cpu threads" v={String(identity.hardwareConcurrency)} />
          <Row
            k="device memory"
            v={identity.deviceMemoryGb ? `${identity.deviceMemoryGb} GB` : 'not exposed'}
          />
          <Row k="webgl renderer" v={identity.webglRenderer} wrap />
          <Row
            k="canvas hash"
            v={identity.canvasHash}
            note="stable identifier across sites; changes if you change GPU / fonts"
          />
          <Row
            k="do-not-track"
            v={identity.doNotTrack}
            tone={identity.doNotTrack === 'on' ? 'on' : 'warn'}
          />
          <Row
            k="cookies"
            v={identity.cookieEnabled ? 'accepted' : 'blocked'}
            tone={identity.cookieEnabled ? 'warn' : 'on'}
          />
        </div>
      </section>

      <section className="footprint-section">
        <h3 className="footprint-section-title">local storage (gets wiped on shred)</h3>
        <div className="footprint-rows">
          <Row
            k="history entries"
            v={summary ? summary.history_count.toLocaleString() : '…'}
          />
          <Row
            k="bookmarks"
            v={summary ? `${summary.bookmarks_count.toLocaleString()} (kept)` : '…'}
            note="bookmarks are not wiped by shred"
          />
          <Row
            k="downloads logged"
            v={summary ? summary.downloads_count.toLocaleString() : '…'}
          />
          <Row
            k="block log"
            v={summary ? summary.block_log_count.toLocaleString() : '…'}
          />
          <Row
            k="open tabs"
            v={
              summary
                ? `${summary.tabs_total} (${summary.tabs_private} private)`
                : '…'
            }
          />
        </div>

        <div className="footprint-shred">
          {confirming ? (
            <div className="footprint-shred-confirm">
              <span>
                clear history + downloads + cookies + cache + localStorage + IndexedDB +
                service workers + block log + stats? (bookmarks &amp; settings stay)
              </span>
              <div className="footprint-shred-buttons">
                <button
                  className="footprint-shred-btn footprint-shred-go"
                  onClick={shred}
                  disabled={shredding}
                >
                  {shredding ? 'shredding…' : 'yes, shred'}
                </button>
                <button
                  className="footprint-shred-btn"
                  onClick={() => setConfirming(false)}
                  disabled={shredding}
                >
                  cancel
                </button>
              </div>
            </div>
          ) : (
            <button
              className="footprint-shred-btn footprint-shred-arm"
              onClick={() => {
                setOutcome(null);
                setConfirming(true);
              }}
            >
              shred footprint
            </button>
          )}

          {outcome && (
            <div className="footprint-shred-outcome">
              <ResultLine ok={outcome.history} label="history" />
              <ResultLine ok={outcome.downloads} label="downloads" />
              <ResultLine ok={outcome.block_log} label="block log" />
              <ResultLine ok={outcome.stats} label="stats" />
              <ResultLine
                ok={outcome.browsing_data}
                label="cookies / cache / localStorage / IDB / SW"
              />
              {outcome.errors.length > 0 && (
                <ul className="footprint-shred-errors">
                  {outcome.errors.map((e, i) => (
                    <li key={i}>{e}</li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
      </section>
    </div>
  );
}

function Row({
  k,
  v,
  tone,
  note,
  wrap,
}: {
  k: string;
  v: string;
  tone?: 'on' | 'warn' | 'off';
  note?: string;
  wrap?: boolean;
}) {
  return (
    <div className="footprint-row">
      <span className="footprint-row-key">{k}</span>
      <span
        className={`footprint-row-value ${tone ? `footprint-tone-${tone}` : ''} ${
          wrap ? 'footprint-row-wrap' : ''
        }`}
      >
        {v}
      </span>
      {note && <span className="footprint-row-note">{note}</span>}
    </div>
  );
}

function ResultLine({ ok, label }: { ok: boolean; label: string }) {
  return (
    <div className={`footprint-shred-result ${ok ? 'footprint-tone-on' : 'footprint-tone-off'}`}>
      <span aria-hidden>{ok ? '✓' : '×'}</span>
      <span>{label}</span>
    </div>
  );
}
