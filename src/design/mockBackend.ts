// Fake Tauri backend for the design preview. Installs the official
// @tauri-apps/api mocks so the whole chrome renders and behaves in a
// plain browser tab, with no Rust process behind it.
//
// This module (and everything under src/design/) is only ever imported
// from src/design-main.tsx, which is only ever loaded by design.html.
// The production entry (index.html -> src/main.tsx) never imports it,
// so it is not part of `pnpm build`'s output; scripts/check-no-design-in-build.mjs
// asserts that after every build.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';
import { emit } from '@tauri-apps/api/event';
import * as fx from './fixtures';

export type Scenario = 'default' | 'trust-modal' | 'approval' | 'tor-bootstrapping';

interface MutableState {
  tabs: fx.FixtureTab[];
  activeId: number | null;
  mobileUa: boolean;
  blockAds: boolean;
  bookmarks: typeof fx.BOOKMARKS;
  scenario: Scenario;
}

function freshState(scenario: Scenario, mobile: boolean): MutableState {
  return {
    tabs: fx.TABS.map((t) => ({ ...t })),
    activeId: fx.ACTIVE_TAB_ID,
    mobileUa: mobile,
    blockAds: true,
    bookmarks: fx.BOOKMARKS.map((b) => ({ ...b })),
    scenario,
  };
}

function tabsView(state: MutableState) {
  return { tabs: state.tabs, active_id: state.activeId };
}

export type DesignMockState = MutableState;

/** Installs mockWindows + mockIPC with a handler that answers every
 * invoke() the BlueFlame UI makes with realistic fixture data. Call once
 * before the app mounts. */
export function installDesignMocks(scenario: Scenario, mobile = false) {
  mockWindows('main');

  const state = freshState(scenario, mobile);
  let nextTabId = 1000;

  mockIPC(async (cmd, rawArgs) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;

    if (cmd.startsWith('plugin:window|')) {
      const winCmd = cmd.slice('plugin:window|'.length);
      if (winCmd === 'is_maximized' || winCmd === 'is_minimized') return false;
      if (winCmd === 'is_maximizable' || winCmd === 'is_minimizable') return true;
      return null;
    }
    if (cmd.startsWith('plugin:opener|')) return null;

    switch (cmd) {
      case 'log_from_frontend':
        return null;

      case 'get_proxy_status':
        return state.scenario === 'tor-bootstrapping'
          ? fx.PROXY_STATUS_TOR_BOOTSTRAPPING
          : fx.PROXY_STATUS_ON;
      case 'get_stats':
        return fx.STATS;
      case 'get_mobile_ua':
        return state.mobileUa;
      case 'set_mobile_ua':
        state.mobileUa = Boolean(args.mobile);
        return null;
      case 'get_ca_trust_status':
        return state.scenario === 'trust-modal' ? fx.CA_TRUST_UNTRUSTED : fx.CA_TRUST_TRUSTED;
      case 'install_ca':
        return null;
      case 'reveal_ca':
        return null;

      case 'browser_list_tabs':
        return tabsView(state);
      case 'browser_switch_tab':
        if (state.tabs.some((t) => t.id === args.id)) state.activeId = args.id as number;
        return tabsView(state);
      case 'browser_close_tab': {
        state.tabs = state.tabs.filter((t) => t.id !== args.id);
        if (state.activeId === args.id) {
          state.activeId = state.tabs.length ? state.tabs[0].id : null;
        }
        return tabsView(state);
      }
      case 'browser_new_tab':
      case 'browser_new_private_tab': {
        const id = nextTabId++;
        state.tabs = [
          ...state.tabs,
          {
            id,
            url: 'about:blank',
            title: 'New tab',
            loading: false,
            private: cmd === 'browser_new_private_tab',
          },
        ];
        state.activeId = id;
        return tabsView(state);
      }
      case 'browser_hide_all':
        return tabsView(state);
      case 'browser_show_active':
        return tabsView(state);
      case 'browser_navigate_active': {
        const tab = state.tabs.find((t) => t.id === state.activeId);
        if (tab) {
          tab.url = String(args.url ?? tab.url);
          tab.title = tab.url;
        }
        void emit('blueflame:tabs-changed');
        return null;
      }
      case 'browser_back':
      case 'browser_forward':
      case 'browser_reload':
      case 'browser_open_tab':
      case 'browser_open_devtools':
      case 'browser_find_in_page':
      case 'browser_find_clear':
      case 'close_all_popups':
        return null;

      case 'control_respond_approval':
        return null;

      case 'get_recent_blocks':
        return fx.RECENT_BLOCKS.slice(0, (args.limit as number) ?? fx.RECENT_BLOCKS.length);
      case 'clear_block_log':
        return null;
      case 'get_blocks_for_host':
        return 37;

      case 'bookmark_list':
        return state.bookmarks;
      case 'bookmark_folders':
        return fx.BOOKMARK_FOLDERS;
      case 'bookmark_is':
        return state.bookmarks.some((b) => b.url === args.url);
      case 'bookmark_toggle': {
        const url = String(args.url);
        const exists = state.bookmarks.some((b) => b.url === url);
        if (exists) {
          state.bookmarks = state.bookmarks.filter((b) => b.url !== url);
        } else {
          state.bookmarks = [
            ...state.bookmarks,
            { url, title: String(args.title ?? url), created_at: Date.now(), folder: '' },
          ];
        }
        return !exists;
      }
      case 'bookmark_set_folder': {
        const b = state.bookmarks.find((x) => x.url === args.url);
        if (b) b.folder = String(args.folder ?? '');
        return null;
      }
      case 'bookmark_rename_folder':
        return null;
      case 'bookmark_delete_folder': {
        const count = state.bookmarks.filter((b) => b.folder === args.folder).length;
        state.bookmarks = state.bookmarks.filter((b) => b.folder !== args.folder);
        return count;
      }

      case 'get_favicon':
        return fx.FAVICONS[String(args.host)] ?? null;

      case 'get_system_summary':
        return fx.SYSTEM_SUMMARY;
      case 'enable_filters':
      case 'disable_filters':
        return null;

      case 'get_debug_log':
        return fx.DEBUG_LOG.slice(0, (args.limit as number) ?? fx.DEBUG_LOG.length);
      case 'clear_debug_log':
        return null;
      case 'control_recent_log':
        return fx.CLAUDE_LOG;

      case 'downloads_list':
        return fx.DOWNLOADS;
      case 'downloads_cancel':
      case 'downloads_open':
      case 'downloads_reveal':
      case 'downloads_clear':
        return null;

      case 'resize_menu_popup':
      case 'close_menu_popup':
      case 'open_menu_popup':
      case 'show_context_menu':
      case 'hide_context_menu':
      case 'open_trust_panel':
      case 'close_trust_panel':
        return null;

      case 'get_system_metrics':
        return fx.METRICS_SNAPSHOT;

      case 'get_trust': {
        const url = String(args.url ?? '');
        const label = url.includes('proton') ? 'trusted' : 'ok';
        return fx.trustAssessment(label);
      }
      case 'get_trust_history':
        return fx.TRUST_HISTORY;

      case 'url_suggest':
        return fx.URL_SUGGESTIONS;

      case 'get_filter_lists':
        return fx.FILTER_LISTS;
      case 'refresh_filter_lists':
        return { lists_ok: fx.FILTER_LISTS.length, lists_failed: 0, patterns_active: fx.SYSTEM_SUMMARY.patterns_active };
      case 'list_search_engines':
        return fx.SEARCH_ENGINES;
      case 'get_search_engine':
        return 'duckduckgo';
      case 'set_search_engine':
        return null;
      case 'get_metasearch_enabled':
        return false;
      case 'set_metasearch_enabled':
        return null;
      case 'get_tor_settings':
        return fx.TOR_SETTINGS;
      case 'set_tor_settings':
        return null;
      case 'get_reputation_feeds':
        return fx.REPUTATION_FEEDS;
      case 'refresh_reputation_feeds':
        return { lists_ok: fx.REPUTATION_FEEDS.length, lists_failed: 0, patterns_active: fx.SYSTEM_SUMMARY.patterns_active };
      case 'get_block_ads':
        return state.blockAds;
      case 'set_block_ads':
        state.blockAds = Boolean(args.enabled);
        return null;
      case 'export_data':
        return JSON.stringify({ note: 'design preview fixture export' }, null, 2);
      case 'import_data':
        return { settings_imported: 4, bookmarks_imported: 2, bookmarks_skipped: 0 };
      case 'import_bookmarks_html':
        return { bookmarks_imported: 3, bookmarks_skipped: 1 };
      case 'reset_stats':
        return null;

      case 'personal_recent':
        return fx.HISTORY.slice(0, (args.limit as number) ?? fx.HISTORY.length);
      case 'personal_search':
        return fx.HISTORY.filter((h) =>
          h.title.toLowerCase().includes(String(args.query ?? '').toLowerCase()),
        );
      case 'personal_clear_history':
        return null;

      default:
        return null;
    }
  }, { shouldMockEvents: true });

  return state;
}

/** Fires one fixed-partial download-progress event so the downloads
 * screen shows a live in-progress row alongside the finished ones. A
 * static value (not an animated loop) keeps a screenshot deterministic. */
export function emitDownloadProgress() {
  void emit('blueflame:download-progress', {
    id: fx.DOWNLOAD_IN_PROGRESS_ID,
    url: fx.DOWNLOAD_IN_PROGRESS_URL,
    bytes_written: fx.DOWNLOAD_IN_PROGRESS_WRITTEN,
    total: fx.DOWNLOAD_IN_PROGRESS_TOTAL,
    done: false,
  });
}

/** Fires the pending Claude approval-bar request. */
export function emitApprovalRequest() {
  void emit('blueflame:claude-approval-request', fx.APPROVAL_REQUEST);
}

/** Marks the fixture "docs.rs" tab as Claude-driven, for the tab strip badge. */
export function emitClaudeTabBadge() {
  for (const id of fx.CLAUDE_TAB_IDS) {
    void emit('blueflame:claude-tab', { tab_id: id, driving: true });
  }
}

/** App.tsx's own auto-boot effect (see driveToScreen in design-main.tsx)
 * always opens one throwaway "New tab" the instant it mounts, before the
 * fixture tab list has loaded, against a real backend too. Removing it
 * from the mock's own state and re-announcing the tab list is the one
 * way to clean it up that works the same in every shell mode (desktop
 * tab strip, mobile chrome, mobile tab switcher). */
export function removeAutoBootTab(state: DesignMockState) {
  const before = state.tabs.length;
  state.tabs = state.tabs.filter((t) => t.title !== 'New tab' || t.url !== 'about:blank');
  if (state.tabs.length === before) return;
  if (state.activeId !== null && !state.tabs.some((t) => t.id === state.activeId)) {
    state.activeId = state.tabs.length ? state.tabs[0].id : null;
  }
  void emit('blueflame:tabs-changed');
}
