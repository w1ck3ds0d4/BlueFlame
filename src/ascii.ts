import { useEffect, useState } from 'react';

/// Classic braille spinner frames (10-step cycle, ~12fps reads smoothly).
export const BRAILLE_FRAMES = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Four-frame text flame cycle for the brand mark.
export const FLAME_FRAMES = ['▲', '▴', '△', '▴'];

/// Step through a frame array at the given interval while `active` is true.
/// Returns the current frame (or `frames[0]` when inactive so callers can still
/// render something static).
export function useAsciiFrames(frames: string[], intervalMs: number, active = true): string {
  const [idx, setIdx] = useState(0);
  useEffect(() => {
    if (!active) {
      setIdx(0);
      return;
    }
    const id = window.setInterval(() => {
      setIdx((i) => (i + 1) % frames.length);
    }, intervalMs);
    return () => window.clearInterval(id);
  }, [frames, intervalMs, active]);
  return frames[idx] ?? frames[0] ?? '';
}

/// Reveal `text` one character at a time. Once fully typed, returns the full
/// string and stops ticking.
export function useTypewriter(text: string, msPerChar = 35): { value: string; done: boolean } {
  const [n, setN] = useState(0);
  useEffect(() => {
    setN(0);
    if (!text) return;
    const id = window.setInterval(() => {
      setN((current) => {
        if (current >= text.length) {
          window.clearInterval(id);
          return current;
        }
        return current + 1;
      });
    }, msPerChar);
    return () => window.clearInterval(id);
  }, [text, msPerChar]);
  return { value: text.slice(0, n), done: n >= text.length };
}
