import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ComponentType } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  ArrowLeft,
  ArrowRight,
  Copy,
  ExternalLink,
  Image as ImageIcon,
  Link as LinkIcon,
  RotateCw,
  Search,
  Shield,
  Star,
  Wrench,
} from 'lucide-react';

type LucideIcon = ComponentType<{ size?: number; strokeWidth?: number }>;

/** Mirrors Rust `ContextMenuPayload`. */
interface Payload {
  page_url: string;
  link_url: string | null;
  link_text: string | null;
  image_url: string | null;
  selection_text: string | null;
  screen_x: number;
  screen_y: number;
}

/**
 * Warm-cached context-menu popup. The Rust side spawns this webview
 * once at boot parked offscreen so right-click feels instant. On
 * every event, Rust emits `context-menu:payload` here; we update
 * state, measure the rendered height in `useLayoutEffect`, then call
 * `show_context_menu` to position + reveal at the click coords.
 *
 * Action handlers and dismiss both call `hide_context_menu` instead
 * of closing the webview, so the bundle stays warm for the next RMB.
 */
export function ContextMenu() {
  const [payload, setPayload] = useState<Payload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    document.body.classList.add('menu-popup-body');
    return () => document.body.classList.remove('menu-popup-body');
  }, []);

  // Subscribe once. Each event replaces the current payload, which
  // re-runs the layout effect below to measure and reveal.
  useEffect(() => {
    const unlistenPromise = listen<Payload>('context-menu:payload', (e) => {
      setError(null);
      setPayload(e.payload);
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, []);

  // Measure the rendered menu and ask Rust to show the popup at the
  // click coords with the exact height. Doing this in the same paint
  // as the new payload means the user never sees the popup at the
  // wrong size or position - it goes from offscreen straight to its
  // final spot.
  useLayoutEffect(() => {
    if (!payload) return;
    const el = rootRef.current;
    if (!el) return;
    const h = el.getBoundingClientRect().height;
    if (h <= 0) return;
    invoke('show_context_menu', {
      x: payload.screen_x,
      y: payload.screen_y,
      height: Math.ceil(h),
    }).catch(() => undefined);
  }, [payload]);

  async function dismiss() {
    await invoke('hide_context_menu').catch(() => undefined);
  }

  async function openTab(url: string, priv_: boolean) {
    if (!url) return;
    try {
      if (priv_) {
        await invoke('browser_new_private_tab');
      } else {
        await invoke('browser_new_tab');
      }
      await invoke('browser_navigate_active', { url });
    } catch (e) {
      setError(String(e));
    }
    await dismiss();
  }

  async function copy(text: string) {
    if (!text) return;
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await navigator.clipboard.writeText(text);
      } else {
        const ta = document.createElement('textarea');
        ta.value = text;
        document.body.appendChild(ta);
        ta.select();
        document.execCommand('copy');
        document.body.removeChild(ta);
      }
    } catch (e) {
      setError(String(e));
    }
    await dismiss();
  }

  async function bookmarkPage() {
    if (!payload) return;
    try {
      await invoke('bookmark_toggle', {
        url: payload.page_url,
        title: '',
      });
    } catch (e) {
      setError(String(e));
    }
    await dismiss();
  }

  async function navCmd(cmd: 'browser_back' | 'browser_forward' | 'browser_reload') {
    try {
      await invoke(cmd);
    } catch (e) {
      setError(String(e));
    }
    await dismiss();
  }

  async function openDevtools() {
    try {
      await invoke('browser_open_devtools');
    } catch (e) {
      setError(String(e));
    }
    await dismiss();
  }

  if (error) {
    return (
      <div className="menu-popup context-menu" ref={rootRef}>
        <div className="context-menu-error">{error}</div>
      </div>
    );
  }
  if (!payload) {
    // First render before any event arrives. The webview is parked
    // offscreen so the user never sees this.
    return <div className="menu-popup context-menu" ref={rootRef} />;
  }

  const hasLink = !!payload.link_url;
  const hasImage = !!payload.image_url;
  const hasSelection = !!payload.selection_text;
  const selectionPreview = hasSelection
    ? truncate(payload.selection_text!.replace(/\s+/g, ' ').trim(), 24)
    : '';

  return (
    <div className="menu-popup context-menu" role="menu" ref={rootRef}>
      {hasLink && (
        <>
          <Item Icon={ExternalLink} label="open link in new tab" onClick={() => openTab(payload.link_url!, false)} />
          <Item Icon={Shield} label="open in private tab" onClick={() => openTab(payload.link_url!, true)} />
          <Item Icon={LinkIcon} label="copy link" onClick={() => copy(payload.link_url!)} />
          <Divider />
        </>
      )}
      {hasImage && (
        <>
          <Item Icon={ImageIcon} label="copy image url" onClick={() => copy(payload.image_url!)} />
          <Item Icon={ExternalLink} label="open image in new tab" onClick={() => openTab(payload.image_url!, false)} />
          <Divider />
        </>
      )}
      {hasSelection && (
        <>
          <Item Icon={Copy} label="copy" onClick={() => copy(payload.selection_text!)} />
          <Item
            Icon={Search}
            label={`search for "${selectionPreview}"`}
            onClick={() => openTab(payload.selection_text!, false)}
          />
          <Divider />
        </>
      )}
      <Item Icon={ArrowLeft} label="back" onClick={() => navCmd('browser_back')} />
      <Item Icon={ArrowRight} label="forward" onClick={() => navCmd('browser_forward')} />
      <Item Icon={RotateCw} label="reload" onClick={() => navCmd('browser_reload')} />
      <Divider />
      <Item Icon={LinkIcon} label="copy page url" onClick={() => copy(payload.page_url)} />
      <Item Icon={Star} label="bookmark page" onClick={bookmarkPage} />
      <Divider />
      <Item Icon={Wrench} label="inspect" onClick={openDevtools} />
    </div>
  );
}

interface ItemProps {
  Icon: LucideIcon;
  label: string;
  onClick: () => void;
}

function Item({ Icon, label, onClick }: ItemProps) {
  return (
    <button role="menuitem" className="menu-popup-item" onClick={onClick}>
      <span className="menu-popup-icon" aria-hidden>
        <Icon size={14} strokeWidth={1.75} />
      </span>
      <span>{label}</span>
    </button>
  );
}

function Divider() {
  return <div className="menu-popup-divider" aria-hidden />;
}

function truncate(s: string, n: number): string {
  return s.length <= n ? s : s.slice(0, n - 1) + '…';
}
