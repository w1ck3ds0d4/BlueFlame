import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { useWebviewOverlay } from '../useWebviewOverlay';

interface CaTrustStatus {
  cert_path: string;
  trusted: boolean;
  auto_install_supported: boolean;
}

interface Props {
  browsing: boolean;
  onDismissed: () => void;
}

export function CaTrustModal({ browsing, onDismissed }: Props) {
  const [status, setStatus] = useState<CaTrustStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useWebviewOverlay(true, browsing);

  async function refresh() {
    try {
      const s = await invoke<CaTrustStatus>('get_ca_trust_status');
      setStatus(s);
      setError(null);
      if (s.trusted) {
        onDismissed();
      }
    } catch (e) {
      setError(String(e));
    }
  }

  async function onInstall() {
    setBusy(true);
    setError(null);
    try {
      await invoke('install_ca');
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function onReveal() {
    setBusy(true);
    setError(null);
    try {
      await invoke('reveal_ca');
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, 3000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  if (!status) {
    return null;
  }

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true" aria-labelledby="ca-trust-title">
      <div className="modal">
        <h2 id="ca-trust-title">Trust the BlueFlame CA</h2>

        <p className="modal-lead">
          BlueFlame decrypts HTTPS locally to filter trackers. To avoid certificate warnings,
          your OS needs to recognize the BlueFlame root CA.
        </p>

        <div className="callout callout-warn">
          <strong>Installing a root CA is a serious action.</strong> Anyone with the CA private
          key could impersonate HTTPS sites on this machine. BlueFlame keeps the key locally
          in <code className="mono">{status.cert_path.replace(/\.crt$/, '.key')}</code>. Uninstall
          the CA if you stop using BlueFlame.
        </div>

        <p>
          Cert file:
          <br />
          <code className="mono">{status.cert_path}</code>
        </p>

        {error && <div className="error">{error}</div>}

        <div className="modal-actions">
          {status.auto_install_supported && (
            <button className="primary" onClick={onInstall} disabled={busy}>
              {busy ? 'Installing...' : 'Install for current user'}
            </button>
          )}
          <button className="secondary" onClick={onReveal} disabled={busy}>
            Show cert file
          </button>
          <button className="link" onClick={onDismissed}>
            I'll do this later
          </button>
        </div>

        {!status.auto_install_supported && (
          <p className="modal-hint">
            Auto-install is only wired for Windows today. On macOS/Linux, use the manual
            command shown in the README after clicking <em>Show cert file</em>.
          </p>
        )}
      </div>
    </div>
  );
}
