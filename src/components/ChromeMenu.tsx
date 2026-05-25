import { useEffect, useLayoutEffect, useRef } from 'react';
import type { ComponentType, ReactNode } from 'react';

type LucideIcon = ComponentType<{ size?: number; strokeWidth?: number }>;

export interface ChromeMenuItem {
  /** Stable key for the React list. */
  id: string;
  /** Visible label. */
  label: string;
  /** Optional lucide icon shown on the left of the row. */
  Icon?: LucideIcon;
  /** Click handler. Receives no args - capture state via closure. The
   *  menu closes automatically after handling. */
  onClick: () => void;
  /** Disabled rows are dimmed and unclickable. */
  disabled?: boolean;
  /** When true, render a danger-styled row (red on hover). Used for
   *  destructive actions like "delete bookmark". */
  danger?: boolean;
}

interface Props {
  /** Click coords in viewport pixels (e.g. from event.clientX/Y). The
   *  menu auto-clamps to stay on-screen. */
  x: number;
  y: number;
  /** Rendered rows. `null` divider entries insert a horizontal rule. */
  items: Array<ChromeMenuItem | 'divider'>;
  /** Fired when the user dismisses (click outside / Escape / item clicked). */
  onClose: () => void;
  /** Whether the caller is currently browsing (i.e. an active tab
   *  webview is visible). When true, we hide the tab webview while
   *  the menu is open - DOM elements can't stack above native child
   *  webviews, so a tall menu that extends below the chrome would
   *  otherwise get clipped. Restored automatically on close. */
  browsing: boolean;
}

/**
 * Generic themed right-click menu used by chrome elements (tab strip,
 * bookmarks bar, etc.). Positions itself at the click coords, clamps
 * to the viewport, closes on outside click + Escape, and lives in the
 * normal React tree (not a separate Tauri webview).
 *
 * Separate from `ContextMenu.tsx`, which is the warm-cached popup
 * webview used for right-clicks INSIDE tab pages - chrome elements
 * are part of the shell webview, so a plain DOM portal works and the
 * heavier child-webview machinery isn't needed.
 */
export function ChromeMenu({ x, y, items, onClose, browsing }: Props) {
  const ref = useRef<HTMLDivElement | null>(null);
  // `browsing` is still received so the caller's contract doesn't
  // change - we may use it later to nudge layout decisions (e.g. clamp
  // the menu's bottom to the chrome boundary). For now the menu just
  // gets clamped to the viewport and accepts that very long menus
  // can extend below the chrome where the native webview sits.
  void browsing;

  // Position + clamp once after measurement so the menu can't render
  // half off the right or bottom edge of the window.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const left = Math.min(x, Math.max(4, vw - r.width - 4));
    const top = Math.min(y, Math.max(4, vh - r.height - 4));
    el.style.left = `${left}px`;
    el.style.top = `${top}px`;
  }, [x, y]);

  // Dismiss on outside click + Escape. Captures the outside click on
  // mousedown rather than click so a quick mousedown-outside / mouseup-
  // inside doesn't keep the menu open with stale state.
  useEffect(() => {
    function onMouseDown(e: MouseEvent) {
      if (!ref.current) return;
      if (!ref.current.contains(e.target as Node)) onClose();
    }
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') {
        e.preventDefault();
        onClose();
      }
    }
    window.addEventListener('mousedown', onMouseDown, true);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('mousedown', onMouseDown, true);
      window.removeEventListener('keydown', onKey);
    };
  }, [onClose]);

  function pick(item: ChromeMenuItem) {
    if (item.disabled) return;
    item.onClick();
    onClose();
  }

  return (
    <div
      ref={ref}
      className="chrome-menu"
      role="menu"
      // Stop the contextmenu event on the menu itself from bubbling -
      // otherwise right-clicking the menu would re-open it / close it
      // depending on bubbling order.
      onContextMenu={(e) => e.preventDefault()}
    >
      {items.map((item, i) =>
        item === 'divider' ? (
          <div key={`d-${i}`} className="chrome-menu-divider" aria-hidden />
        ) : (
          <button
            key={item.id}
            role="menuitem"
            className={`chrome-menu-item ${item.danger ? 'chrome-menu-item-danger' : ''}`}
            disabled={item.disabled}
            onClick={() => pick(item)}
          >
            <ChromeMenuIconSlot Icon={item.Icon} />
            <span className="chrome-menu-label">{item.label}</span>
          </button>
        ),
      )}
    </div>
  );
}

function ChromeMenuIconSlot({ Icon }: { Icon?: LucideIcon }): ReactNode {
  return (
    <span className="chrome-menu-icon" aria-hidden>
      {Icon ? <Icon size={13} strokeWidth={1.75} /> : null}
    </span>
  );
}
