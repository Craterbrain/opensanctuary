// Keeps a misbehaving plugin from taking the console with it.
//
// A plugin page that spins forever can freeze the whole console tab when the browser doesn't give
// the frame its own process (the default in some browsers). Two defences, neither of which trusts the plugin:
//
//  * Watchdog: the console pings each running frame (the SDK answers). A frame that was responsive and goes
//    quiet is removed for the rest of the session and the operator is told. This works whenever the console
//    itself is still alive, i.e. when the browser isolated the frame.
//  * Safe mode: when the console itself hangs nothing in-page can help, so the *next* load recovers. Plugin
//    frames are started only after a "pending" marker is set; the marker is cleared on a clean unload or once the
//    console has stayed responsive for a while. A marker still present at boot means the last session ended
//    in a hang: plugins stay paused until the operator resumes them.
//
// Neither revokes anything: the operator's grants are untouched.

export interface GuardStorage {
  getItem(k: string): string | null;
  setItem(k: string, v: string): void;
  removeItem(k: string): void;
}

const KEY = 'os.pluginGuard.pending';
/** A marker older than this is stale (a crashed browser days ago shouldn't pause plugins forever). */
const MAX_AGE_MS = 15 * 60 * 1000;
/** The console must stay responsive this long after plugins start before the marker is cleared. */
export const HEALTHY_AFTER_MS = 8000;
export const PING_EVERY_MS = 4000;
export const SILENT_LIMIT_MS = 15000;
/** A console tick that arrives this late means the *console* stalled; don't blame a plugin for it. */
const STALL_MS = 3000;

function defaultStorage(): GuardStorage | null {
  try { return window.localStorage; } catch { return null; }
}

/** True when the previous session ended without ever becoming healthy (and the marker is recent). */
export function previousSessionHung(now = Date.now(), store: GuardStorage | null = defaultStorage()): boolean {
  try {
    const raw = store?.getItem(KEY);
    if (!raw) return false;
    const at = Number(raw);
    return Number.isFinite(at) && now - at < MAX_AGE_MS && now >= at - 1000;
  } catch { return false; }
}

export function markPluginsStarting(now = Date.now(), store: GuardStorage | null = defaultStorage()): void {
  try { store?.setItem(KEY, String(now)); } catch { /* storage blocked: no safe mode, no harm */ }
}

export function clearPluginMarker(store: GuardStorage | null = defaultStorage()): void {
  try { store?.removeItem(KEY); } catch { /* ignore */ }
}

/** Clears the marker after a clean unload, or once the console has stayed responsive for HEALTHY_AFTER_MS. */
export function watchConsoleHealth(): void {
  window.addEventListener('pagehide', () => clearPluginMarker());
  let last = Date.now();
  const began = last;
  const timer = window.setInterval(() => {
    const now = Date.now();
    if (now - last > STALL_MS) { last = now; return; } // stalled: start the clock over (marker stays set)
    last = now;
    if (now - began >= HEALTHY_AFTER_MS) { clearPluginMarker(); window.clearInterval(timer); }
  }, 500);
}

export interface Watched {
  id: string;
  /** When the frame last proved it was alive (a pong, or any request it made). */
  alive: number;
  /** Has it ever answered a ping? Pages that never load the SDK are left alone. */
  everAlive: boolean;
}

/**
 * Decides which watched frames have gone silent. Pure, so it's testable: `consoleLagged` is true when the
 * console's own timer was late (then nobody is judged this round).
 */
export function silentFrames(frames: Iterable<Watched>, now: number, consoleLagged: boolean): Watched[] {
  if (consoleLagged) return [];
  return Array.from(frames).filter((f) => f.everAlive && now - f.alive > SILENT_LIMIT_MS);
}

/** The banner shown in safe mode. Returns a function that removes it. */
export function showSafeModeBanner(onResume: () => void, onReview: () => void): () => void {
  const bar = document.createElement('div');
  bar.id = 'plugin-safe-mode';
  bar.setAttribute('role', 'alert');
  const text = document.createElement('span');
  text.textContent = 'Plugins are paused: the console stopped responding last time, and a plugin may be the cause.';
  const resume = document.createElement('button');
  resume.type = 'button';
  resume.className = 'btn btn-primary';
  resume.textContent = 'Resume plugins';
  const review = document.createElement('button');
  review.type = 'button';
  review.className = 'btn';
  review.textContent = 'Review plugins';
  const remove = () => bar.remove();
  resume.addEventListener('click', () => { remove(); onResume(); });
  review.addEventListener('click', onReview);
  bar.append(text, resume, review);
  document.body.appendChild(bar);
  return remove;
}
