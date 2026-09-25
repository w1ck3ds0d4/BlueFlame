// BlueFlame's appearance preference, part of the WickIT design system
// adoption. The system ships three themes (default/dark, light,
// surface-paper); BlueFlame's chrome only ever uses the first two, set
// as `data-theme` on <html> per the system's rule that a component
// never branches on the theme itself, it only reads tokens.
export type Theme = 'default' | 'light';

const STORAGE_KEY = 'blueflame-theme';

export function getStoredTheme(): Theme {
  try {
    const stored = window.localStorage.getItem(STORAGE_KEY);
    return stored === 'light' ? 'light' : 'default';
  } catch {
    // Storage can throw in a locked-down webview; default theme is fine.
    return 'default';
  }
}

export function applyTheme(theme: Theme): void {
  document.documentElement.setAttribute('data-theme', theme);
}

export function setStoredTheme(theme: Theme): void {
  applyTheme(theme);
  try {
    window.localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Preference just won't survive a restart; the app still works.
  }
}
