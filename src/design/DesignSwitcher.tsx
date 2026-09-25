import { useState } from 'react';

export interface ScreenDef {
  id: string;
  label: string;
  /** Standalone popups open in a new tab (they are their own webview in
   * the real app, so a plain link click is closer to reality than
   * swapping state in place). */
  standalone?: boolean;
}

export const SCREENS: ScreenDef[] = [
  { id: 'dashboard', label: 'Dashboard' },
  { id: 'browsing', label: 'Browsing (tab strip)' },
  { id: 'bookmarks', label: 'Bookmarks' },
  { id: 'downloads', label: 'Downloads' },
  { id: 'metrics', label: 'Metrics' },
  { id: 'settings', label: 'Settings + history' },
  { id: 'debug', label: 'Debug log' },
  { id: 'trust-modal', label: 'CA trust modal' },
  { id: 'approval', label: 'Claude approval bar' },
  { id: 'mobile', label: 'Mobile shell' },
  { id: 'tabswitcher', label: 'Mobile tab switcher' },
];

export const STANDALONE_SCREENS: { id: string; label: string; href: string }[] = [
  { id: 'menu-hamburger', label: 'Hamburger menu popup', href: '?panel=menu&kind=hamburger&view=dashboard' },
  { id: 'menu-kebab', label: 'Kebab menu popup', href: '?panel=menu&kind=kebab&view=dashboard&bookmarked=1&browsing=1' },
  { id: 'context-menu', label: 'Context menu popup', href: '?panel=context' },
  { id: 'trust-popup', label: 'Trust popup', href: '?panel=trust&url=https://github.com/w1ck3ds0d4/BlueFlame&tab=overview' },
];

export type ScreenId = (typeof SCREENS)[number]['id'] | (typeof STANDALONE_SCREENS)[number]['id'];

interface Props {
  current: string;
}

/** Small floating panel, dev-only, for jumping between the fixture
 * screens and states while working on the design. Never shown outside
 * design.html. Hidden entirely when the screenshot script passes
 * `?hideswitcher=1` so it does not show up in the "before" captures. */
export function DesignSwitcher({ current }: Props) {
  const hide = new URLSearchParams(window.location.search).get('hideswitcher') === '1';
  const [open, setOpen] = useState(!hide);
  if (hide) return null;

  return (
    <div className="design-switcher" data-open={open}>
      <button
        className="design-switcher-toggle"
        onClick={() => setOpen((v) => !v)}
        aria-label="toggle design preview switcher"
      >
        design preview {open ? '▼' : '▲'}
      </button>
      {open && (
        <div className="design-switcher-panel">
          <div className="design-switcher-group">
            <div className="design-switcher-heading">screens</div>
            {SCREENS.map((s) => (
              <a
                key={s.id}
                className={`design-switcher-link ${current === s.id ? 'design-switcher-link-active' : ''}`}
                href={`?screen=${s.id}`}
              >
                {s.label}
              </a>
            ))}
          </div>
          <div className="design-switcher-group">
            <div className="design-switcher-heading">popups (own tab)</div>
            {STANDALONE_SCREENS.map((s) => (
              <a key={s.id} className="design-switcher-link" href={s.href} target="_blank" rel="noreferrer">
                {s.label}
              </a>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
