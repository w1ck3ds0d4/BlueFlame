import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

/** Native webviews render on top of React DOM on every platform, so any
 *  full-screen modal or dropdown that extends below the chrome would be
 *  covered by the active tab's webview. This hook shrinks the active
 *  webview to 0x0 while `open` is true, then restores it when closed - as
 *  long as the user was actually browsing when the overlay opened.
 *
 *  Matches the pattern TabSwitcher / the trust-panel child webview use so
 *  every popup behaves consistently. */
export function useWebviewOverlay(open: boolean, browsing: boolean) {
  const wasBrowsing = useRef(false);
  useEffect(() => {
    if (!open) return;
    wasBrowsing.current = browsing;
    if (!browsing) return;
    invoke('browser_hide_all').catch(() => undefined);
    return () => {
      if (wasBrowsing.current) {
        invoke('browser_show_active').catch(() => undefined);
      }
    };
    // Intentionally not depending on `browsing`: we only want to react
    // to `open` transitions so the overlay's lifecycle drives the
    // hide/show, not a mid-overlay navigation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);
}
