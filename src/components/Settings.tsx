import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { PersonalIndex } from './PersonalIndex';
import { BRAILLE_FRAMES, useAsciiFrames } from '../ascii';

interface FilterListEntry {
  name: string;
  url: string;
  cached: boolean;
  cached_at: number | null;
}

interface RefreshResult {
  lists_ok: number;
  lists_failed: number;
  patterns_active: number;
}

interface CaTrustStatus {
  cert_path: string;
  trusted: boolean;
  auto_install_supported: boolean;
}

interface EngineOption {
  id: string;
  name: string;
}

interface TorSettingsDto {
  enabled: boolean;
  proxy_addr: string;
  built_in: boolean;
  built_in_supported: boolean;
  applied_mode: string;
}

interface ReputationFeed {
  name: string;
  url: string;
  cached: boolean;
  cached_at: number | null;
}

export function Settings() {
  const [lists, setLists] = useState<FilterListEntry[]>([]);
  const [trust, setTrust] = useState<CaTrustStatus | null>(null);
  const [engines, setEngines] = useState<EngineOption[]>([]);
  const [selectedEngine, setSelectedEngine] = useState<string>('duckduckgo');
  const [metasearchOn, setMetasearchOn] = useState(false);
  const [lastRefresh, setLastRefresh] = useState<RefreshResult | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [tor, setTor] = useState<TorSettingsDto>({
    enabled: false,
    proxy_addr: '127.0.0.1:9050',
    built_in: false,
    built_in_supported: false,
    applied_mode: 'direct',
  });
  const [torSaving, setTorSaving] = useState(false);
  const [repFeeds, setRepFeeds] = useState<ReputationFeed[]>([]);
  const [repRefreshing, setRepRefreshing] = useState(false);
  const [lastRepRefresh, setLastRepRefresh] = useState<RefreshResult | null>(null);
  const [mobileUa, setMobileUa] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function loadAll() {
    // Each call populates its own slice of state. A failure in one (e.g.
    // get_ca_trust_status can probe the OS trust store and hiccup) must
    // not wipe the others back to initial defaults - that's how the Tor
    // toggle appeared to "not save" across restarts.
    const results = await Promise.allSettled([
      invoke<FilterListEntry[]>('get_filter_lists'),
      invoke<CaTrustStatus>('get_ca_trust_status'),
      invoke<EngineOption[]>('list_search_engines'),
      invoke<string>('get_search_engine'),
      invoke<boolean>('get_metasearch_enabled'),
      invoke<TorSettingsDto>('get_tor_settings'),
      invoke<ReputationFeed[]>('get_reputation_feeds'),
      invoke<boolean>('get_mobile_ua'),
    ]);
    const [l, t, es, chosen, meta, torS, reps, mobUa] = results;
    if (l.status === 'fulfilled') setLists(l.value);
    if (t.status === 'fulfilled') setTrust(t.value);
    if (es.status === 'fulfilled') setEngines(es.value);
    if (chosen.status === 'fulfilled') setSelectedEngine(chosen.value);
    if (meta.status === 'fulfilled') setMetasearchOn(meta.value);
    if (torS.status === 'fulfilled') setTor(torS.value);
    if (reps.status === 'fulfilled') setRepFeeds(reps.value);
    if (mobUa.status === 'fulfilled') setMobileUa(mobUa.value);
    const first = results.find((r) => r.status === 'rejected') as
      | PromiseRejectedResult
      | undefined;
    setError(first ? String(first.reason) : null);
  }

  async function onBrowserModeChange(next: boolean) {
    try {
      await invoke('set_mobile_ua', { mobile: next });
      // set_mobile_ua rebuilds every tab with the new UA, which as
      // a side effect makes the previously-active tab visible again.
      // The Settings panel only renders while the user is NOT in
      // browse mode, so hide the rebuilt tabs to restore the
      // Settings view on top.
      await invoke('browser_hide_all').catch(() => undefined);
      setMobileUa(next);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onRefreshReputation() {
    setRepRefreshing(true);
    setError(null);
    try {
      const r = await invoke<RefreshResult>('refresh_reputation_feeds');
      setLastRepRefresh(r);
      const fresh = await invoke<ReputationFeed[]>('get_reputation_feeds');
      setRepFeeds(fresh);
    } catch (e) {
      setError(String(e));
    } finally {
      setRepRefreshing(false);
    }
  }

  async function onTorSave() {
    setTorSaving(true);
    setError(null);
    try {
      await invoke('set_tor_settings', {
        enabled: tor.enabled,
        proxyAddr: tor.proxy_addr,
        builtIn: tor.built_in,
      });
      // Re-read so the `applied_mode` reflects what's actually running.
      const fresh = await invoke<TorSettingsDto>('get_tor_settings');
      setTor(fresh);
    } catch (e) {
      setError(String(e));
    } finally {
      setTorSaving(false);
    }
  }

  async function onEngineChange(id: string) {
    try {
      await invoke('set_search_engine', { id });
      setSelectedEngine(id);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onMetasearchToggle(next: boolean) {
    try {
      await invoke('set_metasearch_enabled', { enabled: next });
      setMetasearchOn(next);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onRefresh() {
    setRefreshing(true);
    setError(null);
    try {
      const r = await invoke<RefreshResult>('refresh_filter_lists');
      setLastRefresh(r);
      await loadAll();
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshing(false);
    }
  }

  async function onResetStats() {
    try {
      await invoke('reset_stats');
    } catch (e) {
      setError(String(e));
    }
  }

  async function onRevealCa() {
    try {
      await invoke('reveal_ca');
    } catch (e) {
      setError(String(e));
    }
  }

  async function onInstallCa() {
    try {
      await invoke('install_ca');
      await loadAll();
    } catch (e) {
      setError(String(e));
    }
  }

  useEffect(() => {
    loadAll();
  }, []);

  const spinner = useAsciiFrames(BRAILLE_FRAMES, 90, refreshing);

  return (
    <section className="settings">
      <h2 className="settings-title">settings</h2>

      {error && <div className="error">{error}</div>}

      <PersonalIndex />

      <div className="panel">
        <div className="panel-header">
          <h3>browser mode</h3>
        </div>
        <div className="panel-note">
          Controls the user-agent sent to sites. UA-sniffing sites like YouTube /
          Reddit / Twitter serve their mobile layout only when they see a mobile
          UA; pure window resizing won't trigger it. Flipping this reloads every
          open tab with the new UA so the change is global; scroll position,
          form input, and private-tab sessions are lost on the reload.
        </div>
        <div className="engine-grid">
          <label className={`engine-choice ${!mobileUa ? 'engine-active' : ''}`}>
            <input
              type="radio"
              name="browser-mode"
              checked={!mobileUa}
              onChange={() => onBrowserModeChange(false)}
            />
            <span>desktop</span>
          </label>
          <label className={`engine-choice ${mobileUa ? 'engine-active' : ''}`}>
            <input
              type="radio"
              name="browser-mode"
              checked={mobileUa}
              onChange={() => onBrowserModeChange(true)}
            />
            <span>mobile (Android)</span>
          </label>
        </div>
      </div>

      <div className="panel">
        <div className="panel-header">
          <h3>search engine</h3>
        </div>
        <div className="panel-note">
          Used when the URL bar input is a search query instead of a URL. All options are
          privacy-aligned and do not require accounts (except Kagi, which is paid).
        </div>
        <div className="engine-grid">
          {engines.map((e) => (
            <label
              key={e.id}
              className={`engine-choice ${selectedEngine === e.id ? 'engine-active' : ''}`}
            >
              <input
                type="radio"
                name="search-engine"
                checked={selectedEngine === e.id}
                onChange={() => onEngineChange(e.id)}
              />
              <span>{e.name}</span>
            </label>
          ))}
        </div>

        <label className="meta-toggle">
          <input
            type="checkbox"
            checked={metasearchOn}
            onChange={(e) => onMetasearchToggle(e.currentTarget.checked)}
          />
          <span>
            <strong>metasearch mode (beta)</strong>
            <small>
              Render BlueFlame's own results page instead of opening the engine's site. Sources:
              DuckDuckGo HTML (more engines coming).
            </small>
          </span>
        </label>
      </div>

      <div className="panel">
        <div className="panel-header">
          <h3>filter lists</h3>
          <button className="primary" onClick={onRefresh} disabled={refreshing}>
            {refreshing ? `${spinner} refreshing` : 'refresh now'}
          </button>
        </div>

        {lastRefresh && (
          <div className="panel-note">
            last refresh: <strong>{lastRefresh.patterns_active.toLocaleString()}</strong>{' '}
            patterns active ({lastRefresh.lists_ok} ok, {lastRefresh.lists_failed} failed)
          </div>
        )}

        <table className="settings-table">
          <thead>
            <tr>
              <th>name</th>
              <th>url</th>
              <th>cached</th>
            </tr>
          </thead>
          <tbody>
            {lists.map((l) => (
              <tr key={l.url}>
                <td>{l.name}</td>
                <td className="mono truncate">{l.url}</td>
                <td>{l.cached ? formatAge(l.cached_at) : 'no'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="panel">
        <div className="panel-header">
          <h3>reputation feeds</h3>
          <button
            className="primary"
            onClick={onRefreshReputation}
            disabled={repRefreshing}
          >
            {repRefreshing ? `${spinner} refreshing` : 'refresh now'}
          </button>
        </div>
        <div className="panel-note">
          Host-level known-bad lists that feed the <strong>malware</strong>{' '}
          category score. Entries are hosts extracted from each feed's URL list,
          cached on disk between runs.
        </div>

        {lastRepRefresh && (
          <div className="panel-note">
            last refresh: <strong>+{lastRepRefresh.patterns_active.toLocaleString()}</strong>{' '}
            hosts merged ({lastRepRefresh.lists_ok} ok,{' '}
            {lastRepRefresh.lists_failed} failed)
          </div>
        )}

        <table className="settings-table">
          <thead>
            <tr>
              <th>name</th>
              <th>url</th>
              <th>cached</th>
            </tr>
          </thead>
          <tbody>
            {repFeeds.map((f) => (
              <tr key={f.url}>
                <td>{f.name}</td>
                <td className="mono truncate">{f.url}</td>
                <td>{f.cached ? formatAge(f.cached_at) : 'no'}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="panel">
        <div className="panel-header">
          <h3>root ca</h3>
          <div className={`trust-badge ${trust?.trusted ? 'trust-ok' : 'trust-warn'}`}>
            {trust?.trusted ? 'trusted' : 'not trusted'}
          </div>
        </div>
        {trust && (
          <>
            <div className="panel-note">
              <code className="mono">{trust.cert_path}</code>
            </div>
            <div className="panel-actions">
              {trust.auto_install_supported && !trust.trusted && (
                <button className="primary" onClick={onInstallCa}>
                  install for current user
                </button>
              )}
              <button className="secondary" onClick={onRevealCa}>
                show cert file
              </button>
            </div>
          </>
        )}
      </div>

      <div className="panel">
        <div className="panel-header">
          <h3>tor upstream</h3>
          <span
            className={`trust-badge ${
              tor.applied_mode === 'direct' ? 'trust-warn' : 'trust-ok'
            }`}
          >
            {tor.applied_mode}
          </span>
        </div>
        <div className="panel-note">
          Route the MITM proxy's upstream through Tor. Two ways: (1) use the built-in arti client
          bundled into BlueFlame (slow first boot while it fetches the Tor directory, no external
          daemon needed), or (2) point at an existing local SOCKS5 endpoint (Tor daemon on{' '}
          <code className="mono">127.0.0.1:9050</code>, Tor Browser on{' '}
          <code className="mono">127.0.0.1:9150</code>). Changes apply after restart.
        </div>

        <label className="meta-toggle">
          <input
            type="checkbox"
            checked={tor.built_in}
            disabled={!tor.built_in_supported}
            onChange={(e) =>
              setTor({ ...tor, built_in: e.currentTarget.checked })
            }
          />
          <span>
            <strong>
              use built-in tor (arti){!tor.built_in_supported ? ' - unavailable in this build' : ''}
            </strong>
            <small>
              Wins over the SOCKS5 option below. Bootstrap takes ~10-30s on first run while arti
              fetches a directory consensus.
            </small>
          </span>
        </label>

        <label className="meta-toggle">
          <input
            type="checkbox"
            checked={tor.enabled}
            disabled={tor.built_in}
            onChange={(e) => setTor({ ...tor, enabled: e.currentTarget.checked })}
          />
          <span>
            <strong>route upstream through external SOCKS5</strong>
            <small>
              Use a tor daemon, tor browser, or any SOCKS5 endpoint you already run. Disabled
              while built-in tor is selected.
            </small>
          </span>
        </label>
        <div className="panel-actions">
          <input
            type="text"
            className="url-input mono"
            value={tor.proxy_addr}
            onChange={(e) => setTor({ ...tor, proxy_addr: e.currentTarget.value })}
            spellCheck={false}
            autoCorrect="off"
            placeholder="127.0.0.1:9050"
            disabled={tor.built_in}
            style={{ flex: 1 }}
          />
          <button className="primary" onClick={onTorSave} disabled={torSaving}>
            {torSaving ? 'saving' : 'save'}
          </button>
        </div>
        {(tor.enabled || tor.built_in) &&
          tor.applied_mode === 'direct' && (
            <div className="panel-note" style={{ color: 'var(--warn)' }}>
              // setting saved. restart blueflame to route traffic through tor.
            </div>
          )}
      </div>

      <div className="panel">
        <div className="panel-header">
          <h3>stats</h3>
          <button className="secondary" onClick={onResetStats}>
            reset counters
          </button>
        </div>
        <div className="panel-note">
          Resetting zeroes total requests, blocked, and bytes saved. The proxy and filters keep
          running.
        </div>
      </div>
    </section>
  );
}

function formatAge(epochSecs: number | null): string {
  if (!epochSecs) return 'no';
  const ageSecs = Date.now() / 1000 - epochSecs;
  if (ageSecs < 60) return `${Math.floor(ageSecs)}s ago`;
  if (ageSecs < 3600) return `${Math.floor(ageSecs / 60)}m ago`;
  if (ageSecs < 86400) return `${Math.floor(ageSecs / 3600)}h ago`;
  return `${Math.floor(ageSecs / 86400)}d ago`;
}
