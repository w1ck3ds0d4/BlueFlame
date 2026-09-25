import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

interface ApprovalRequest {
  request_id: number;
  origin: string;
  tab_id: number | null;
}

/**
 * Banner for the Claude control channel's per-origin approval gate: the
 * dispatcher blocks a tool call on a new site until Daniel answers here.
 * Requests queue if more than one arrives; only the oldest is shown.
 */
export function ApprovalBar() {
  const [queue, setQueue] = useState<ApprovalRequest[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    listen<ApprovalRequest>('blueflame:claude-approval-request', (e) => {
      setQueue((q) => [...q, e.payload]);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => undefined);
    return () => {
      unlisten?.();
    };
  }, []);

  const current = queue[0];
  if (!current) return null;

  async function respond(allow: boolean) {
    setBusy(true);
    try {
      await invoke('control_respond_approval', { requestId: current.request_id, allow });
    } catch {
      // the request may have already timed out server-side; drop it either way
    } finally {
      setBusy(false);
      setQueue((q) => q.slice(1));
    }
  }

  return (
    <div className="approval-bar" role="alert">
      <span className="approval-bar-text">
        Claude wants to visit <strong>{current.origin}</strong>
      </span>
      <div className="approval-bar-actions">
        <button className="secondary" disabled={busy} onClick={() => respond(true)}>
          Allow
        </button>
        <button className="link" disabled={busy} onClick={() => respond(false)}>
          Deny
        </button>
      </div>
    </div>
  );
}
