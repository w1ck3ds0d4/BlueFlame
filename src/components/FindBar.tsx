import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface Props {
  open: boolean;
  onClose: () => void;
}

export function FindBar({ open, onClose }: Props) {
  const [query, setQuery] = useState('');
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    if (open) {
      inputRef.current?.focus();
      inputRef.current?.select();
    } else {
      invoke('browser_find_clear').catch(() => undefined);
    }
  }, [open]);

  async function run(forward: boolean) {
    if (!query.trim()) return;
    try {
      await invoke('browser_find_in_page', { query, forward });
    } catch {
      /* ignore */
    }
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'Escape') {
      e.preventDefault();
      onClose();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      run(!e.shiftKey);
    }
  }

  if (!open) return null;
  return (
    <div className="find-bar" role="search" aria-label="find in page">
      <span className="find-bar-label">find:</span>
      <input
        ref={inputRef}
        type="text"
        className="find-bar-input mono"
        value={query}
        onChange={(e) => setQuery(e.currentTarget.value)}
        onKeyDown={onKeyDown}
        spellCheck={false}
        autoCorrect="off"
        aria-label="find query"
      />
      <button className="find-bar-btn" onClick={() => run(false)} title="previous (shift+enter)">
        ↑
      </button>
      <button className="find-bar-btn" onClick={() => run(true)} title="next (enter)">
        ↓
      </button>
      <button className="find-bar-btn" onClick={onClose} title="close (esc)">
        ×
      </button>
    </div>
  );
}
