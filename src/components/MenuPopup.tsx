import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ComponentType } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import {
  Activity,
  ArrowLeft,
  ArrowRight,
  Bookmark,
  BookmarkPlus,
  Download,
  LayoutGrid,
  List,
  Plus,
  RotateCw,
  Settings as SettingsIcon,
  Shield,
  Star,
  Terminal,
} from 'lucide-react';

type LucideIcon = ComponentType<{ size?: number; strokeWidth?: number }>;

type View = 'dashboard' | 'bookmarks' | 'downloads' | 'metrics' | 'settings' | 'debug';

type StatusKind = 'on' | 'off' | 'starting' | 'booting' | 'failed';

interface ProxyStatus {
  running: boolean;
  port: number;
  filters_enabled: boolean;
  tor_bootstrap: string;
}

const STATUS_LABEL: Record<StatusKind, string> = {
  on: 'on',
  off: 'off',
  starting: '…',
  booting: 'tor',
  failed: 'err',
};

/**
 * Standalone menu panel. Lives in its own child webview spawned by
 * `open_menu_popup` (Rust) so it renders ON TOP of the active tab's
 * native webview - a React-DOM dropdown would get covered.
 *
 * Reads `kind=hamburger|kebab` from the URL. Each item invokes a Tauri
 * command directly or emits a `blueflame:select-view` event the main
 * App picks up; then calls `close_menu_popup` to dismiss itself.
 */
export function MenuPopup() {
  const params = new URLSearchParams(window.location.search);
  const kind = params.get('kind') ?? 'hamburger';
  const initialView = (params.get('view') as View) ?? 'dashboard';
  const bookmarkedInitial = params.get('bookmarked') === '1';
  const browsingParam = params.get('browsing') === '1';

  const [bookmarked] = useState(bookmarkedInitial);
  const [statusKind, setStatusKind] = useState<StatusKind>('off');
  const [statusText, setStatusText] = useState<string>('off');
  const rootRef = useRef<HTMLDivElement | null>(null);

  // Tint the whole webview dark (body default is white) and shrink-to-
  // fit the child webview to the rendered content height so there's no
  // dead space below the items.
  useEffect(() => {
    document.body.classList.add('menu-popup-body');
    return () => document.body.classList.remove('menu-popup-body');
  }, []);

  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const h = el.getBoundingClientRect().height;
    if (h > 0) {
      invoke('resize_menu_popup', { height: Math.ceil(h) }).catch(() => undefined);
    }
  }, [statusText, bookmarked]);

  useEffect(() => {
    if (kind !== 'hamburger') return;
    let cancelled = false;
    const pull = async () => {
      try {
        const s = await invoke<ProxyStatus>('get_proxy_status');
        if (cancelled) return;
        const torBoot = s.tor_bootstrap === 'running';
        const torFail = s.tor_bootstrap.startsWith('failed');
        if (torBoot) {
          setStatusKind('booting');
          setStatusText('tor:bootstrapping');
        } else if (torFail) {
          setStatusKind('failed');
          setStatusText(`tor:failed`);
        } else if (!s.running) {
          setStatusKind('starting');
          setStatusText('proxy:starting');
        } else if (s.filters_enabled) {
          setStatusKind('on');
          setStatusText(`proxy:on :${s.port}`);
        } else {
          setStatusKind('off');
          setStatusText(`proxy:off :${s.port}`);
        }
      } catch {
        /* ignore */
      }
    };
    pull();
    const id = window.setInterval(pull, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [kind]);

  async function close() {
    await invoke('close_menu_popup').catch(() => undefined);
  }

  async function pickView(view: View) {
    await emit('blueflame:select-view', view).catch(() => undefined);
    await close();
  }

  async function runCmd(cmd: string) {
    await invoke(cmd).catch(() => undefined);
    await close();
  }

  async function toggleBookmark() {
    await emit('blueflame:toggle-bookmark').catch(() => undefined);
    await close();
  }

  async function openNewTab(priv: boolean) {
    await emit(priv ? 'blueflame:new-private-tab' : 'blueflame:new-tab').catch(
      () => undefined,
    );
    await close();
  }

  if (kind === 'hamburger') {
    const items: { id: View; Icon: LucideIcon; label: string }[] = [
      { id: 'dashboard', Icon: LayoutGrid, label: 'dash' },
      { id: 'bookmarks', Icon: Star, label: 'bkm' },
      { id: 'downloads', Icon: Download, label: 'dl' },
      { id: 'metrics', Icon: Activity, label: 'mtr' },
      { id: 'settings', Icon: SettingsIcon, label: 'set' },
      { id: 'debug', Icon: Terminal, label: 'dbg' },
    ];
    return (
      <div className="menu-popup" role="menu" ref={rootRef}>
        {items.map((item) => {
          const { Icon } = item;
          return (
            <button
              key={item.id}
              role="menuitem"
              className={`menu-popup-item ${item.id === initialView ? 'menu-popup-item-active' : ''}`}
              onClick={() => pickView(item.id)}
            >
              <span className="menu-popup-icon" aria-hidden>
                <Icon size={16} strokeWidth={1.75} />
              </span>
              <span>{item.label}</span>
            </button>
          );
        })}
        <div
          className={`menu-popup-status sidebar-status-${statusKind}`}
          title={statusText}
        >
          <span className="sidebar-status-dot" aria-hidden>
            ●
          </span>
          <span>{STATUS_LABEL[statusKind]}</span>
          <span className="menu-popup-status-text">{statusText}</span>
        </div>
      </div>
    );
  }

  // kebab
  const canNavigate = browsingParam;
  return (
    <div className="menu-popup menu-popup-right" role="menu" ref={rootRef}>
      <KebabItem Icon={ArrowLeft} label="back" onClick={() => runCmd('browser_back')} disabled={!canNavigate} />
      <KebabItem Icon={ArrowRight} label="forward" onClick={() => runCmd('browser_forward')} disabled={!canNavigate} />
      <KebabItem Icon={RotateCw} label="reload" onClick={() => runCmd('browser_reload')} disabled={!canNavigate} />
      <KebabItem
        Icon={bookmarked ? BookmarkPlus : Bookmark}
        label={bookmarked ? 'remove bookmark' : 'add bookmark'}
        onClick={toggleBookmark}
        disabled={!canNavigate}
      />
      <KebabItem Icon={List} label="all bookmarks" onClick={() => pickView('bookmarks')} />
      <div className="menu-popup-divider" aria-hidden />
      <KebabItem Icon={Plus} label="new tab" onClick={() => openNewTab(false)} />
      <KebabItem Icon={Shield} label="new private tab" onClick={() => openNewTab(true)} />
    </div>
  );
}

interface KebabItemProps {
  Icon: LucideIcon;
  label: string;
  onClick: () => void;
  disabled?: boolean;
}

function KebabItem({ Icon, label, onClick, disabled }: KebabItemProps) {
  return (
    <button role="menuitem" className="menu-popup-item" onClick={onClick} disabled={disabled}>
      <span className="menu-popup-icon" aria-hidden>
        <Icon size={16} strokeWidth={1.75} />
      </span>
      <span>{label}</span>
    </button>
  );
}
