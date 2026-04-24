import { useEffect, useState } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import logoUrl from '../assets/LOGO-GRADIENT.BF.png';

interface Props {
  /** Whether the shell is in mobile mode. On mobile the titlebar
   *  shrinks to a thin drag region + min/max/close - brand and
   *  navigation move to MobileChrome. */
  mobile: boolean;
}

export function TitleBar({ mobile }: Props) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const win = getCurrentWindow();
    let unlisten: (() => void) | undefined;
    win.isMaximized().then(setMaximized).catch(() => undefined);
    win
      .onResized(() => {
        win.isMaximized().then(setMaximized).catch(() => undefined);
      })
      .then((fn) => {
        unlisten = fn;
      });
    return () => {
      unlisten?.();
    };
  }, []);

  async function onMinimize() {
    await getCurrentWindow().minimize();
  }

  async function onToggleMaximize() {
    await getCurrentWindow().toggleMaximize();
  }

  async function onClose() {
    await getCurrentWindow().close();
  }

  return (
    <div className={`titlebar ${mobile ? 'titlebar-mobile' : ''}`} data-tauri-drag-region>
      {!mobile && (
        <div className="titlebar-brand" data-tauri-drag-region>
          <img className="titlebar-logo" src={logoUrl} alt="" aria-hidden />
          <span className="titlebar-title">blueflame</span>
        </div>
      )}
      <div className="titlebar-spacer" data-tauri-drag-region />
      <div className="titlebar-controls">
        <button
          className="titlebar-btn"
          onClick={onMinimize}
          title="minimize"
          aria-label="minimize"
        >
          _
        </button>
        <button
          className="titlebar-btn"
          onClick={onToggleMaximize}
          title={maximized ? 'restore' : 'maximize'}
          aria-label={maximized ? 'restore' : 'maximize'}
        >
          {maximized ? '▭' : '□'}
        </button>
        <button
          className="titlebar-btn titlebar-close"
          onClick={onClose}
          title="close"
          aria-label="close"
        >
          ×
        </button>
      </div>
    </div>
  );
}
