import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import type { ComponentType } from 'react';
import { invoke } from '@tauri-apps/api/core';
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
 * Child-webview popup that renders the context menu. Loaded at
 * `?panel=context&ctx_id=<uuid>`; fetches its payload on mount from
 * the Rust-side in-memory store (one-shot: a second fetch returns
 * the not-found error, so a stale reload can't replay an action).
 *
 * Layout: icon-label rows. Items are context-dependent - a link
 * click shows link-related actions; page background shows
 * navigation/bookmark actions. Every action closes the popup via
 * `close_context_menu`.
 */
export function ContextMenu() {
  const params = new URLSearchParams(window.location.search);
  const ctxId = params.get('ctx_id') ?? '';

  const [payload, setPayload] = useState<Payload | null>(null);
  const [error, setError] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    document.body.classList.add('menu-popup-body');
    return () => document.body.classList.remove('menu-popup-body');
  }, []);

  useEffect(() => {
    if (!ctxId) {
      setError('missing ctx_id');
      return;
    }
    invoke<Payload>('get_context_payload', { ctxId })
      .then(setPayload)
      .catch((e) => setError(String(e)));
  }, [ctxId]);

  useLayoutEffect(() => {
    const el = rootRef.current;
    if (!el) return;
    const h = el.getBoundingClientRect().height;
    if (h > 0) {
      invoke('resize_context_menu', { height: Math.ceil(h) }).catch(() => undefined);
    }
  }, [payload]);

  async function close() {
    await invoke('close_context_menu').catch(() => undefined);
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
    await close();
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
    await close();
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
    await close();
  }

  async function navCmd(cmd: 'browser_back' | 'browser_forward' | 'browser_reload') {
    try {
      await invoke(cmd);
    } catch (e) {
      setError(String(e));
    }
    await close();
  }

  async function openDevtools() {
    try {
      await invoke('browser_open_devtools');
    } catch (e) {
      setError(String(e));
    }
    await close();
  }

  if (error) {
    return (
      <div className="menu-popup context-menu" ref={rootRef}>
        <div className="context-menu-error">{error}</div>
      </div>
    );
  }
  if (!payload) {
    return (
      <div className="menu-popup context-menu" ref={rootRef}>
        <div className="context-menu-empty">...</div>
      </div>
    );
  }

  const hasLink = !!payload.link_url;
  const hasImage = !!payload.image_url;
  const hasSelection = !!payload.selection_text;
  // Trim + cap selection so the menu label doesn't stretch wide on
  // multi-line copies. 24 chars + ellipsis fits the 240px popup.
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
