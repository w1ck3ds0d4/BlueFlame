import type { ComponentType } from 'react';
import {
  Activity,
  LayoutGrid,
  Settings as SettingsIcon,
  Star,
  Terminal,
  Download,
} from 'lucide-react';

type View = 'dashboard' | 'bookmarks' | 'downloads' | 'metrics' | 'settings' | 'debug';

type StatusKind = 'on' | 'off' | 'starting' | 'booting' | 'failed';

interface Props {
  view: View;
  browsing: boolean;
  onSelect: (v: View) => void;
  /** Full status string for the hover tooltip. */
  statusText: string;
  /** Controls the colored dot + short label on the sidebar footer. */
  statusKind: StatusKind;
}

interface NavItem {
  id: View;
  /** Lucide icon component. Rendered at `--icon-size` from CSS. */
  Icon: ComponentType<{ size?: number; strokeWidth?: number }>;
  label: string;
}

// The rail is 48px wide, too narrow for a full word under an icon
// without wrapping or overflowing, so each view is icon-only here; the
// full word carries the tooltip and the accessible name instead of a
// cryptic on-screen abbreviation.
const NAV: NavItem[] = [
  { id: 'dashboard', Icon: LayoutGrid, label: 'Dashboard' },
  { id: 'bookmarks', Icon: Star, label: 'Bookmarks' },
  { id: 'downloads', Icon: Download, label: 'Downloads' },
  { id: 'metrics', Icon: Activity, label: 'Metrics' },
  { id: 'settings', Icon: SettingsIcon, label: 'Settings' },
  { id: 'debug', Icon: Terminal, label: 'Debug' },
];

// What the dot's colour means, spelled out for the tooltip and
// accessible name; the on-screen word stays a constant "proxy" (the
// dot's colour already carries on/off/starting/failed) rather than
// repeating state as an abbreviation like "err" or "…".
const STATUS_MEANING: Record<StatusKind, string> = {
  on: 'running',
  off: 'stopped',
  starting: 'starting',
  booting: 'starting tor',
  failed: 'failed',
};

export function Sidebar({ view, browsing, onSelect, statusText, statusKind }: Props) {
  return (
    <aside className="sidebar" aria-label="primary navigation">
      <nav className="sidebar-nav">
        {NAV.map((item) => {
          const active = !browsing && view === item.id;
          const { Icon } = item;
          return (
            <button
              key={item.id}
              className={`sidebar-btn ${active ? 'sidebar-btn-active' : ''}`}
              onClick={() => onSelect(item.id)}
              title={item.label}
              // aria-label stays the lowercase id, not the capitalized
              // display label: design-main.tsx's screen driver selects
              // this button by aria-label to steer the design preview
              // to each screen, and the id is already a clear, unabbreviated
              // word on its own.
              aria-label={item.id}
              aria-current={active ? 'page' : undefined}
            >
              <span className="sidebar-btn-icon" aria-hidden>
                <Icon size={20} strokeWidth={1.75} />
              </span>
            </button>
          );
        })}
      </nav>
      <div
        className={`sidebar-status sidebar-status-${statusKind}`}
        title={statusText}
        aria-label={`filter proxy: ${STATUS_MEANING[statusKind]}`}
      >
        <span className="sidebar-status-dot" aria-hidden>
          ●
        </span>
        <span className="sidebar-status-label" aria-hidden>
          proxy
        </span>
      </div>
    </aside>
  );
}
