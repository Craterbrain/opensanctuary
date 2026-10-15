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

// ---- Load shedding: finding the plugin that is slowing the console down -----------------------------
//
// A frame that runs a lot of work in small slices keeps answering pings (so the watchdog above never fires)
// while the console, which shares its thread, crawls. The console can't ask a frame how busy it is (they all
// lag together), so it finds the culprit by elimination: while the console's own timer is running late, switch
// plugins off one at a time until the lag clears. The last one switched off is blamed and stays off for the
// session; the innocent ones are started again.

/** A 1-second console tick that arrives this late counts as lag. */
export const LAG_MS = 1000;
/** Lag means this many late ticks out of the last LAG_WINDOW. */
const LAG_TICKS = 3;
const LAG_WINDOW = 4;
/** After switching one off, ticks to wait before asking "did that clear it?" (lets the window refill). */
const SETTLE_TICKS = 4;
/** When switching every plugin off didn't help, the plugins weren't the cause; leave them alone this long. */
const COOLDOWN_TICKS = 60;

export type ShedAction =
  | { kind: 'suspend'; id: string }
  | { kind: 'restore'; ids: string[] }
  | { kind: 'blame'; id: string };

export class LoadShedder {
  private drifts: number[] = [];
  private suspended: string[] = [];
  private testing = false;
  private settle = 0;
  private cooldownUntil = 0;
  private tickNo = 0;
  private blamed = new Set<string>();

  /**
   * One call per console tick. `driftMs` is how late the tick was; `running` lists the plugin ids that are
   * running now, oldest first. Returns what the caller should do.
   */
  tick(driftMs: number, running: string[]): ShedAction[] {
    this.tickNo++;
    this.drifts.push(driftMs);
    if (this.drifts.length > LAG_WINDOW) this.drifts.shift();
    const candidates = () => running.filter((id) => !this.blamed.has(id) && !this.suspended.includes(id)).reverse(); // newest first

    if (!this.testing) {
      const lagging = this.drifts.filter((d) => d > LAG_MS).length >= LAG_TICKS;
      if (!lagging || this.tickNo < this.cooldownUntil) return [];
      const next = candidates()[0];
      if (!next) return [];
      return [this.suspend(next)];
    }

    this.settle++;
    if (this.settle < SETTLE_TICKS) return [];
    const stillLagging = this.drifts.slice(-SETTLE_TICKS).some((d) => d > LAG_MS);
    if (!stillLagging) {
      // Switching off the most recent one cleared it: that's the culprit.
      const culprit = this.suspended[this.suspended.length - 1];
      const innocents = this.suspended.slice(0, -1);
      this.blamed.add(culprit);
      this.reset();
      return [{ kind: 'blame', id: culprit }, ...(innocents.length ? [{ kind: 'restore', ids: innocents } as ShedAction] : [])];
    }
    const next = candidates()[0];
    if (next) return [this.suspend(next)];
    // Nobody left to try and it's still slow: not the plugins' doing.
    const all = this.suspended;
    this.cooldownUntil = this.tickNo + COOLDOWN_TICKS;
    this.reset();
    return [{ kind: 'restore', ids: all }];
  }

  private suspend(id: string): ShedAction {
    this.suspended.push(id);
    this.testing = true;
    this.settle = 0;
    this.drifts = [];
    return { kind: 'suspend', id };
  }

  private reset() {
    this.suspended = [];
    this.testing = false;
    this.settle = 0;
    this.drifts = [];
  }
}
