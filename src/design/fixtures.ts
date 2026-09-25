// Fixture data for the design preview mode (see ../../docs/DESIGN_PREVIEW.md).
// Never imported outside src/design/ and src/design-main.tsx, and never
// reachable from the production entry (src/main.tsx, index.html).

function svgFavicon(letter: string, hue: number): string {
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32">` +
    `<rect width="32" height="32" rx="7" fill="hsl(${hue} 70% 40%)"/>` +
    `<text x="16" y="22" font-family="sans-serif" font-size="16" font-weight="700" ` +
    `fill="white" text-anchor="middle">${letter}</text></svg>`;
  return `data:image/svg+xml;base64,${btoa(svg)}`;
}

export interface FixtureTab {
  id: number;
  url: string;
  title: string;
  loading: boolean;
  private: boolean;
}

export const FAVICONS: Record<string, string> = {
  'news.ycombinator.com': svgFavicon('Y', 28),
  'github.com': svgFavicon('G', 265),
  'docs.rs': svgFavicon('R', 18),
  'mail.proton.me': svgFavicon('P', 250),
  'signal.org': svgFavicon('S', 220),
  'wickitlab.com': svgFavicon('W', 14),
};

export const TABS: FixtureTab[] = [
  { id: 1, url: 'https://news.ycombinator.com/', title: 'Hacker News', loading: false, private: false },
  { id: 2, url: 'https://github.com/w1ck3ds0d4/BlueFlame', title: 'w1ck3ds0d4/BlueFlame', loading: false, private: false },
  { id: 3, url: 'https://docs.rs/tauri/latest/tauri/', title: 'tauri - Rust', loading: true, private: false },
  { id: 4, url: 'https://mail.proton.me/u/0/inbox', title: 'Inbox - Proton Mail', loading: false, private: true },
  { id: 5, url: 'https://wickitlab.com/', title: 'WickIT Lab', loading: false, private: false },
];

export const ACTIVE_TAB_ID = 2;
export const CLAUDE_TAB_IDS = [3];

export const PROXY_STATUS_ON = {
  running: true,
  port: 8443,
  filters_enabled: true,
  tor_bootstrap: '',
};

export const PROXY_STATUS_TOR_BOOTSTRAPPING = {
  running: true,
  port: 8443,
  filters_enabled: true,
  tor_bootstrap: 'running',
};

export const STATS = {
  requests_total: 48213,
  requests_blocked: 6104,
  bytes_saved: 214_500_000,
};

export const CA_TRUST_TRUSTED = {
  cert_path: 'C:\\Users\\daniel\\AppData\\Local\\BlueFlame\\ca\\blueflame-root.pem',
  trusted: true,
  auto_install_supported: true,
};

export const CA_TRUST_UNTRUSTED = {
  cert_path: 'C:\\Users\\daniel\\AppData\\Local\\BlueFlame\\ca\\blueflame-root.pem',
  trusted: false,
  auto_install_supported: true,
};

// Dashboard.tsx, Downloads.tsx, Settings.tsx and Debug.tsx all read their
// timestamp fields as epoch seconds (each does its own age math against
// Date.now() / 1000, or new Date(epochSecs * 1000)), so every fixture
// feeding one of those must be built in seconds, not milliseconds.
// created_at/visited_at below stay on `now` (ms): nothing reads them as
// a date, they only ever get compared to each other for sort order.
const now = Date.now();
const nowSec = Math.floor(now / 1000);

export const SYSTEM_SUMMARY = {
  uptime_secs: 5 * 3600 + 12 * 60,
  patterns_active: 184_302,
  lists_total: 9,
  last_refresh_secs: nowSec - 3600 * 2,
};

export const RECENT_BLOCKS = Array.from({ length: 60 }, (_, i) => ({
  ts: nowSec - Math.round(i * 4.3),
  url: [
    'https://ads.trackernet.example/pixel.gif',
    'https://metrics.adservice.example/collect',
    'https://cdn.trackfast.example/beacon.js',
    'https://telemetry.spamnet.example/log',
  ][i % 4],
}));

export const BOOKMARKS = [
  { url: 'https://news.ycombinator.com/', title: 'Hacker News', created_at: now - 86_400_000 * 40, folder: '' },
  { url: 'https://github.com/w1ck3ds0d4/BlueFlame', title: 'w1ck3ds0d4/BlueFlame', created_at: now - 86_400_000 * 20, folder: 'dev' },
  { url: 'https://docs.rs/tauri/latest/tauri/', title: 'tauri - Rust', created_at: now - 86_400_000 * 18, folder: 'dev' },
  { url: 'https://wickitlab.com/', title: 'WickIT Lab', created_at: now - 86_400_000 * 5, folder: 'work' },
  { url: 'https://en.wikipedia.org/wiki/Tor_(network)', title: 'Tor (network) - Wikipedia', created_at: now - 86_400_000 * 3, folder: 'reading' },
];

export const BOOKMARK_FOLDERS = ['dev', 'work', 'reading'];

export const HISTORY = [
  { id: 1, url: 'https://news.ycombinator.com/', title: 'Hacker News', visited_at: now - 60_000, visit_count: 34 },
  { id: 2, url: 'https://github.com/w1ck3ds0d4/BlueFlame', title: 'w1ck3ds0d4/BlueFlame', visited_at: now - 300_000, visit_count: 12 },
  { id: 3, url: 'https://docs.rs/tauri/latest/tauri/', title: 'tauri - Rust', visited_at: now - 900_000, visit_count: 4 },
  { id: 4, url: 'https://wickitlab.com/', title: 'WickIT Lab', visited_at: now - 3_600_000, visit_count: 8 },
  { id: 5, url: 'https://en.wikipedia.org/wiki/Tor_(network)', title: 'Tor (network) - Wikipedia', visited_at: now - 7_200_000, visit_count: 1 },
];

// Finished downloads only. The in-progress row is a separate, purely
// event-driven fixture (see emitDownloadProgress) because the real
// Downloads.tsx only shows an active row while a
// 'blueflame:download-progress' event says so, never from the log list.
export const DOWNLOADS = [
  { id: 2, url: 'https://github.com/w1ck3ds0d4/BlueFlame/archive/refs/heads/main.zip', filename: 'BlueFlame-main.zip', path: 'C:\\Users\\daniel\\Downloads\\BlueFlame-main.zip', size: 3_400_000, ts: nowSec - 3600 },
  { id: 1, url: 'https://docs.rs/-/latest/tauri.pdf', filename: 'tauri-notes.pdf', path: 'C:\\Users\\daniel\\Downloads\\tauri-notes.pdf', size: 812_000, ts: nowSec - 86400 },
];

export const DOWNLOAD_IN_PROGRESS_ID = 3;
export const DOWNLOAD_IN_PROGRESS_URL = 'https://cdn.example.com/blueflame-installer.exe';
export const DOWNLOAD_IN_PROGRESS_TOTAL = 41_200_000;
export const DOWNLOAD_IN_PROGRESS_WRITTEN = 18_500_000;

export const FILTER_LISTS = [
  { name: 'EasyList', url: 'https://easylist.to/easylist/easylist.txt', cached: true, cached_at: nowSec - 3600 },
  { name: 'EasyPrivacy', url: 'https://easylist.to/easylist/easyprivacy.txt', cached: true, cached_at: nowSec - 3600 },
  { name: 'URLhaus', url: 'https://urlhaus.abuse.ch/downloads/text/', cached: true, cached_at: nowSec - 7200 },
];

export const SEARCH_ENGINES = [
  { id: 'duckduckgo', name: 'DuckDuckGo' },
  { id: 'brave', name: 'Brave Search' },
  { id: 'startpage', name: 'Startpage' },
];

export const TOR_SETTINGS = {
  enabled: false,
  proxy_addr: '127.0.0.1:9050',
  built_in: true,
  built_in_supported: true,
  applied_mode: 'off',
};

export const REPUTATION_FEEDS = [
  { name: 'URLhaus', url: 'https://urlhaus.abuse.ch/downloads/text/', cached: true, cached_at: nowSec - 7200 },
  { name: 'PhishTank', url: 'https://data.phishtank.com/data/online-valid.json', cached: false, cached_at: null },
];

export const METRICS_SNAPSHOT = {
  ts: now,
  pid: 24680,
  uptime_secs: SYSTEM_SUMMARY.uptime_secs,
  rss_bytes: 312_000_000,
  cpu_percent: 4.2,
  thread_count: 28,
  process_count: 3,
  proxy_requests_total: STATS.requests_total,
  proxy_requests_blocked: STATS.requests_blocked,
  proxy_bytes_saved: STATS.bytes_saved,
  tab_count: TABS.length,
  private_tab_count: TABS.filter((t) => t.private).length,
};

export const DEBUG_LOG = Array.from({ length: 40 }, (_, i) => ({
  ts: nowSec - i * 8,
  level: ['info', 'info', 'warn', 'error'][i % 4],
  target: ['proxy', 'storage', 'frontend:console', 'tor'][i % 4],
  message: [
    'filter list refresh completed (9 lists, 184302 patterns)',
    'sqlite checkpoint ok (2.1mb)',
    'favicon fetch failed for docs.rs, retrying',
    'tor bootstrap stalled at 40 percent, retrying circuit',
  ][i % 4],
}));

export const CLAUDE_LOG: { ts_ms: number; tab_id: number | null; tool: string; detail: string; outcome: string }[] = [
  { ts_ms: now - 12_000, tab_id: 3, tool: 'navigate', detail: 'https://docs.rs/tauri/latest/tauri/', outcome: 'ok' },
  { ts_ms: now - 40_000, tab_id: 3, tool: 'read_page', detail: 'extracted 1.2kb of text', outcome: 'ok' },
  { ts_ms: now - 90_000, tab_id: 3, tool: 'find', detail: 'query: "mockIPC"', outcome: 'ok' },
];

export function trustAssessment(label: 'trusted' | 'ok' | 'suspect' | 'danger') {
  const byLabel = {
    trusted: { score: 96, signals: 1 },
    ok: { score: 78, signals: 2 },
    suspect: { score: 41, signals: 3 },
    danger: { score: 12, signals: 4 },
  } as const;
  const { score } = byLabel[label];
  const cat = (key: 'malware' | 'scam' | 'vuln', name: string) => ({
    key,
    name,
    score,
    label,
    signals: [
      { id: `${key}-1`, message: `no ${name.toLowerCase()} indicators found`, kind: 'positive' as const, category: key },
    ],
  });
  return {
    score,
    label,
    signals: [
      { id: 'overall-1', message: 'certificate chain verified', kind: 'positive' as const, category: 'overview' },
    ],
    categories: {
      malware: cat('malware', 'Malware'),
      scam: cat('scam', 'Scam'),
      vuln: cat('vuln', 'Vulnerability'),
    },
  };
}

export const TRUST_HISTORY = Array.from({ length: 48 }, (_, i) => ({
  score: 70 + Math.round(20 * Math.sin(i / 6)),
  recorded_at: now - (48 - i) * 1_800_000,
}));

export const URL_SUGGESTIONS = [
  { url: 'https://news.ycombinator.com/', title: 'Hacker News', source: 'history' as const, visit_count: 34 },
  { url: 'https://github.com/w1ck3ds0d4/BlueFlame', title: 'w1ck3ds0d4/BlueFlame', source: 'bookmark' as const, visit_count: 12 },
];

export const APPROVAL_REQUEST = { request_id: 1, origin: 'https://docs.rs', tab_id: 3 };
