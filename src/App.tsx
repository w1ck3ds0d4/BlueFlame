import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { Bookmarks } from './components/Bookmarks';
import { Downloads } from './components/Downloads';
import { BookmarksBar } from './components/BookmarksBar';
import { CaTrustModal } from './components/CaTrustModal';
import { Metrics } from './components/Metrics';
import { Dashboard } from './components/Dashboard';
import { Debug } from './components/Debug';
import { FindBar } from './components/FindBar';
import { MobileChrome } from './components/MobileChrome';
import { Settings } from './components/Settings';
import { Sidebar } from './components/Sidebar';
import { TabSwitcher } from './components/TabSwitcher';
import { TitleBar } from './components/TitleBar';
import { UrlBar } from './components/UrlBar';
import { TabStrip } from './components/TabStrip';
import { BRAILLE_FRAMES, useAsciiFrames } from './ascii';
import './App.css';

interface ProxyStatus {
  running: boolean;
  port: number;
  filters_enabled: boolean;
  tor_bootstrap: string;
}

interface CaTrustStatus {
  cert_path: string;
  trusted: boolean;
  auto_install_supported: boolean;
}

interface Stats {
  requests_total: number;
  requests_blocked: number;
  bytes_saved: number;
}

interface TabInfo {
  id: number;
  url: string;
  title: string;
  loading: boolean;
  private: boolean;
}

interface TabsView {
  tabs: TabInfo[];
  active_id: number | null;
}

type View = 'dashboard' | 'bookmarks' | 'downloads' | 'metrics' | 'settings' | 'debug';

export default function App() {
  const [view, setView] = useState<View>('dashboard');
  const [browseVisible, setBrowseVisible] = useState(false);
  const [tabs, setTabs] = useState<TabInfo[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [status, setStatus] = useState<ProxyStatus>({
    running: false,
    port: 0,
    filters_enabled: false,
    tor_bootstrap: '',
  });
  const [stats, setStats] = useState<Stats>({
    requests_total: 0,
    requests_blocked: 0,
    bytes_saved: 0,
  });
  const [error, setError] = useState<string | null>(null);
  const [trustModalOpen, setTrustModalOpen] = useState(false);
  const [trustDismissed, setTrustDismissed] = useState(false);
  const [bookmarksVersion, setBookmarksVersion] = useState(0);
  const [findBarOpen, setFindBarOpen] = useState(false);
  const [mobileShell, setMobileShell] = useState(false);
  const [tabSwitcherOpen, setTabSwitcherOpen] = useState(false);

  // Flipping back to desktop should close the mobile tab switcher so it
  // doesn't linger over the restored desktop chrome.
  useEffect(() => {
    if (!mobileShell) setTabSwitcherOpen(false);
  }, [mobileShell]);

  // The url bar only has a page to act on while the browse stage is live.
  const browsing = browseVisible && activeId !== null;

  const applyTabsView = useCallback((v: TabsView) => {
    setTabs(v.tabs);
    setActiveId(v.active_id);
    if (v.tabs.length === 0 || v.active_id === null) {
      setBrowseVisible(false);
    }
  }, []);

  async function refresh() {
    try {
      const s = await invoke<ProxyStatus>('get_proxy_status');
      setStatus(s);
      const st = await invoke<Stats>('get_stats');
      setStats(st);
      // Pick up the desktop/mobile toggle so the shell's chrome CSS
      // (narrowed to match the phone viewport) follows the Settings
      // change without requiring a manual re-render.
      const mob = await invoke<boolean>('get_mobile_ua');
      setMobileShell(mob);
      setError(null);

      if (!trustDismissed) {
        const trust = await invoke<CaTrustStatus>('get_ca_trust_status');
        setTrustModalOpen(!trust.trusted);
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function refreshTabs() {
    try {
      const v = await invoke<TabsView>('browser_list_tabs');
      applyTabsView(v);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onSelectTab(id: number) {
    try {
      const v = await invoke<TabsView>('browser_switch_tab', { id });
      applyTabsView(v);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onCloseTab(id: number) {
    try {
      const v = await invoke<TabsView>('browser_close_tab', { id });
      applyTabsView(v);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onNewTab() {
    try {
      const v = await invoke<TabsView>('browser_new_tab');
      applyTabsView(v);
    } catch (e) {
      setError(String(e));
    }
  }

  async function onNewPrivateTab() {
    try {
      const v = await invoke<TabsView>('browser_new_private_tab');
      applyTabsView(v);
    } catch (e) {
      setError(String(e));
    }
  }

  async function goHome() {
    setBrowseVisible(false);
    try {
      const v = await invoke<TabsView>('browser_hide_all');
      applyTabsView(v);
    } catch {
      /* ignore */
    }
  }

  async function showBrowser() {
    try {
      const v = await invoke<TabsView>('browser_show_active');
      applyTabsView(v);
      if (v.active_id !== null) setBrowseVisible(true);
    } catch {
      /* ignore */
    }
  }

  useEffect(() => {
    refresh();
    refreshTabs();
    const id = setInterval(refresh, 2000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [trustDismissed]);

  // Backend fires this whenever a tab's URL or derived title changes (page
  // navigates, redirect, SPA route swap). Re-pull the tab list so the tab
  // strip + URL bar follow the real page the user is on.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen('blueflame:tabs-changed', () => {
      refreshTabs();
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Refs so the menu-popup event listeners see the latest active tab
  // without re-subscribing on every tab change.
  const tabsRef = useRef(tabs);
  const activeIdRef = useRef(activeId);
  useEffect(() => {
    tabsRef.current = tabs;
    activeIdRef.current = activeId;
  }, [tabs, activeId]);

  // Events from the menu popup child webview. The popup can't directly
  // mutate App state, so each action is dispatched as an event that we
  // translate back into the same calls the in-DOM menu used to make.
  useEffect(() => {
    const unlisteners: UnlistenFn[] = [];
    listen<string>('blueflame:select-view', (e) => {
      const v = e.payload as View;
      if (
        v === 'dashboard' ||
        v === 'bookmarks' ||
        v === 'downloads' ||
        v === 'metrics' ||
        v === 'settings' ||
        v === 'debug'
      ) {
        setView(v);
        goHome();
      }
    })
      .then((fn) => unlisteners.push(fn))
      .catch(() => undefined);
    listen('blueflame:new-tab', () => {
      onNewTab().then(showBrowser);
    })
      .then((fn) => unlisteners.push(fn))
      .catch(() => undefined);
    listen('blueflame:new-private-tab', () => {
      onNewPrivateTab().then(showBrowser);
    })
      .then((fn) => unlisteners.push(fn))
      .catch(() => undefined);
    listen('blueflame:toggle-bookmark', () => {
      const tab = tabsRef.current.find((t) => t.id === activeIdRef.current);
      if (!tab) return;
      const url = tab.url;
      if (!url || url.startsWith('data:') || url.startsWith('about:')) return;
      invoke<boolean>('bookmark_toggle', { url, title: tab.title || url })
        .then(() => setBookmarksVersion((v) => v + 1))
        .catch(() => undefined);
    })
      .then((fn) => unlisteners.push(fn))
      .catch(() => undefined);
    return () => {
      for (const fn of unlisteners) fn();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Any full-screen React overlay (tab switcher, CA trust modal) needs
  // the child-webview popups closed so they don't float on top of the
  // now-hidden tab.
  useEffect(() => {
    if (tabSwitcherOpen || trustModalOpen) {
      invoke('close_all_popups').catch(() => undefined);
    }
  }, [tabSwitcherOpen, trustModalOpen]);

  // Open a new tab on first boot so the user lands on the speed-dial page
  // instead of the empty dashboard. Fire once after the initial
  // refreshTabs resolves and reports zero tabs.
  const didAutoBoot = useRef(false);
  useEffect(() => {
    if (didAutoBoot.current) return;
    if (tabs.length > 0 || activeId !== null) {
      didAutoBoot.current = true;
      return;
    }
    didAutoBoot.current = true;
    (async () => {
      await onNewTab();
      await showBrowser();
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabs.length, activeId]);

  const activeTab = tabs.find((t) => t.id === activeId) ?? null;

  useEffect(() => {
    function cycleTab(dir: 1 | -1) {
      if (tabs.length < 2 || activeId === null) return;
      const i = tabs.findIndex((t) => t.id === activeId);
      if (i < 0) return;
      const next = (i + dir + tabs.length) % tabs.length;
      onSelectTab(tabs[next].id).then(showBrowser);
    }

    async function toggleActiveBookmark() {
      if (!activeTab) return;
      const url = activeTab.url;
      if (!url || url.startsWith('data:') || url.startsWith('about:')) return;
      try {
        await invoke<boolean>('bookmark_toggle', {
          url,
          title: activeTab.title || url,
        });
        setBookmarksVersion((v) => v + 1);
      } catch {
        /* ignore */
      }
    }

    // Central dispatcher for Ctrl/Meta shortcuts. Called from two places:
    //   (a) window.keydown - when focus is in the React shell
    //   (b) blueflame:tab-shortcut Tauri event - relayed up from the
    //       tab webview's init script when focus is inside the page.
    // Both paths call this with the same {key, shift} shape.
    function runShortcut(k: string, shift: boolean) {
      if (k === 'l') {
        window.dispatchEvent(new CustomEvent('blueflame:focus-url'));
      } else if (k === 't') {
        if (shift) onNewPrivateTab().then(showBrowser);
        else onNewTab().then(showBrowser);
      } else if (k === 'w') {
        if (activeId !== null) onCloseTab(activeId);
      } else if (k === 'r') {
        invoke('browser_reload').catch(() => undefined);
      } else if (k === 'f') {
        setFindBarOpen(true);
      } else if (k === 'd') {
        if (shift) {
          setView('dashboard');
          goHome();
        } else {
          toggleActiveBookmark();
        }
      } else if (k === ',') {
        setView('settings');
        goHome();
      } else if (k === 'tab') {
        cycleTab(shift ? -1 : 1);
      } else if (/^[1-9]$/.test(k)) {
        const idx = parseInt(k, 10) - 1;
        if (tabs[idx]) onSelectTab(tabs[idx].id).then(showBrowser);
      }
    }

    function onKeyDown(e: KeyboardEvent) {
      if (!(e.ctrlKey || e.metaKey)) return;
      const k = e.key.toLowerCase();
      // Keys we know how to handle - preventDefault only for those so
      // we don't swallow native Ctrl+C / Ctrl+V / etc. that we don't
      // implement here.
      const handled = new Set([
        'l', 't', 'w', 'r', 'f', 'd', ',', 'tab',
        '1', '2', '3', '4', '5', '6', '7', '8', '9',
      ]);
      if (!handled.has(k)) return;
      e.preventDefault();
      runShortcut(k, e.shiftKey);
    }

    window.addEventListener('keydown', onKeyDown);
    let unlistenTab: UnlistenFn | undefined;
    listen<{ key: string; shift: boolean }>('blueflame:tab-shortcut', (e) => {
      runShortcut(e.payload.key, e.payload.shift);
    })
      .then((fn) => {
        unlistenTab = fn;
      })
      .catch(() => undefined);
    return () => {
      window.removeEventListener('keydown', onKeyDown);
      unlistenTab?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tabs, activeId, activeTab]);

  const proxyRunning = status.running && status.filters_enabled;
  const torBootstrapping = status.tor_bootstrap === 'running';
  const torFailed = status.tor_bootstrap.startsWith('failed');
  // Keep the spinner ticking during bootstrap too so the user sees life.
  const spinnerFrame = useAsciiFrames(BRAILLE_FRAMES, 90, proxyRunning || torBootstrapping);

  let statusText: string;
  let statusKind: 'on' | 'off' | 'starting' | 'booting' | 'failed';
  if (torBootstrapping) {
    statusText = `${spinnerFrame} tor:bootstrapping`;
    statusKind = 'booting';
  } else if (torFailed) {
    statusText = `tor:failed (${status.tor_bootstrap.slice('failed:'.length).trim()})`;
    statusKind = 'failed';
  } else if (!status.running) {
    statusText = `${spinnerFrame} proxy:starting`;
    statusKind = 'starting';
  } else if (status.filters_enabled) {
    statusText = `proxy:on :${status.port}`;
    statusKind = 'on';
  } else {
    statusText = `proxy:off :${status.port}`;
    statusKind = 'off';
  }

  return (
    <main className={`shell ${mobileShell ? 'mobile-shell' : ''}`}>
      <TitleBar mobile={mobileShell} />
      <Sidebar
        view={view}
        browsing={browsing}
        onSelect={(v) => {
          setView(v);
          goHome();
        }}
        statusText={statusText}
        statusKind={statusKind}
      />
      <div className="main-col">
      <header className="chrome-full">
        {mobileShell ? (
          <MobileChrome
            currentUrl={activeTab?.url ?? ''}
            browsing={browsing}
            onOpenTabSwitcher={() => setTabSwitcherOpen(true)}
            tabCount={tabs.length}
            view={view}
            bookmarksVersion={bookmarksVersion}
            onNavigated={() => {
              refreshTabs();
              showBrowser();
            }}
            onShowBrowser={showBrowser}
          />
        ) : (
          <>
            <UrlBar
              browsing={browsing}
              currentUrl={activeTab?.url ?? ''}
              currentTitle={activeTab?.title ?? ''}
              onNavigated={() => {
                refreshTabs();
                showBrowser();
              }}
              onHome={goHome}
              onBookmarksChanged={() => setBookmarksVersion((v) => v + 1)}
            />

            <TabStrip
              tabs={tabs}
              activeId={activeId}
              onSelect={async (id) => {
                await onSelectTab(id);
                await showBrowser();
              }}
              onClose={onCloseTab}
              onNewTab={async () => {
                await onNewTab();
                await showBrowser();
              }}
              onNewPrivateTab={async () => {
                await onNewPrivateTab();
                await showBrowser();
              }}
            />

            <BookmarksBar
              version={bookmarksVersion}
              onOpened={async () => {
                await refreshTabs();
                await showBrowser();
              }}
            />
          </>
        )}

        <FindBar open={findBarOpen && browsing} onClose={() => setFindBarOpen(false)} />
      </header>

      {error && <div className="error-banner">{error}</div>}

      {browsing ? (
        <div className="browse-stage" aria-label="Browse area - the native webview renders below" />
      ) : view === 'dashboard' ? (
        <Dashboard status={status} stats={stats} onToggled={refresh} />
      ) : view === 'bookmarks' ? (
        <Bookmarks version={bookmarksVersion} />
      ) : view === 'downloads' ? (
        <Downloads />
      ) : view === 'metrics' ? (
        <Metrics />
      ) : view === 'debug' ? (
        <Debug />
      ) : (
        <Settings />
      )}

      {!browsing && (
        <footer className="footer">
          <span>blueflame // privacy-first browser shell</span>
        </footer>
      )}
      </div>

      <TabSwitcher
        open={tabSwitcherOpen && mobileShell}
        browsing={browsing}
        tabs={tabs}
        activeId={activeId}
        onClose={async () => {
          setTabSwitcherOpen(false);
          if (browseVisible) await showBrowser();
        }}
        onPick={async (id) => {
          setTabSwitcherOpen(false);
          await onSelectTab(id);
          await showBrowser();
        }}
        onCloseTab={async (id) => {
          await onCloseTab(id);
        }}
        onNewTab={async () => {
          setTabSwitcherOpen(false);
          await onNewTab();
          await showBrowser();
        }}
        onNewPrivateTab={async () => {
          setTabSwitcherOpen(false);
          await onNewPrivateTab();
          await showBrowser();
        }}
      />

      {trustModalOpen && (
        <CaTrustModal
          browsing={browsing}
          onDismissed={() => {
            setTrustModalOpen(false);
            setTrustDismissed(true);
          }}
        />
      )}
    </main>
  );
}
