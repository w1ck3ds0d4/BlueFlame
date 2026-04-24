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
  icon: string;
  label: string;
}

const NAV: NavItem[] = [
  { id: 'dashboard', icon: '▮▮▮', label: 'dash' },
  { id: 'bookmarks', icon: '★', label: 'bkm' },
  { id: 'downloads', icon: '↓', label: 'dl' },
  { id: 'metrics', icon: '◊', label: 'mtr' },
  { id: 'settings', icon: '[=]', label: 'set' },
  { id: 'debug', icon: '>_', label: 'dbg' },
];

const STATUS_LABEL: Record<StatusKind, string> = {
  on: 'on',
  off: 'off',
  starting: '…',
  booting: 'tor',
  failed: 'err',
};

export function Sidebar({ view, browsing, onSelect, statusText, statusKind }: Props) {
  return (
    <aside className="sidebar" aria-label="primary navigation">
      <nav className="sidebar-nav">
        {NAV.map((item) => {
          const active = !browsing && view === item.id;
          return (
            <button
              key={item.id}
              className={`sidebar-btn ${active ? 'sidebar-btn-active' : ''}`}
              onClick={() => onSelect(item.id)}
              title={item.id}
              aria-label={item.id}
              aria-current={active ? 'page' : undefined}
            >
              <span className="sidebar-btn-icon" aria-hidden>
                {item.icon}
              </span>
              <span className="sidebar-btn-label">{item.label}</span>
            </button>
          );
        })}
      </nav>
      <div className={`sidebar-status sidebar-status-${statusKind}`} title={statusText}>
        <span className="sidebar-status-dot" aria-hidden>
          ●
        </span>
        <span className="sidebar-status-label">{STATUS_LABEL[statusKind]}</span>
      </div>
    </aside>
  );
}
