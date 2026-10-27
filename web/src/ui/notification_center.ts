/**
 * Notification center: the one place the console keeps and shows notifications.
 *
 * Three things feed it -- the console's own toasts (`showToast` reports to it), the host's
 * notices and notifications posted by approved plugins (both arrive by polling
 * `GET /api/notifications` every couple of seconds -- a short poll rather than a held-open
 * request, so the page still reaches "network idle"). Every entry is shown as plain text, its source named by the host
 * (a plugin can't pick its own name or pass as the host). The operator can mute a source:
 * muted notifications still land in the list but don't pop up.
 */

import { showToast, setToastObserver, type ToastType } from '../core/ui_utils.ts';

export interface Notice {
  /** `s<n>` from the host, `l<n>` raised locally. */
  uid: string;
  serverId?: number;
  ts: number;
  /** `console`, `host`, or `plugin:<id>`. */
  source: string;
  sourceName: string;
  level: ToastType;
  title: string;
  body: string;
  sticky: boolean;
  key?: string;
}

const MAX_KEPT = 150;
const MUTE_KEY = 'os_notifications_muted';
const POLL_MS = 2000;
const POLL_HIDDEN_MS = 6000;

let items: Notice[] = [];
let unread = 0;
let localSeq = 0;
let lastServerId = 0;
let feedStarted = false;
let muted = new Set<string>();
const listeners = new Set<() => void>();

try {
  const raw = localStorage.getItem(MUTE_KEY);
  if (raw) muted = new Set(JSON.parse(raw).filter((s: unknown) => typeof s === 'string'));
} catch { /* storage unavailable: nothing muted */ }

function saveMuted(): void {
  try { localStorage.setItem(MUTE_KEY, JSON.stringify([...muted])); } catch { /* best effort */ }
}

function changed(): void {
  listeners.forEach((fn) => { try { fn(); } catch { /* a bad listener mustn't stop the rest */ } });
}

function push(n: Notice): void {
  if (n.key) items = items.filter((i) => !(i.source === n.source && i.key === n.key));
  items.unshift(n);
  if (items.length > MAX_KEPT) items.length = MAX_KEPT;
  unread++;
  changed();
}

export function isMuted(source: string): boolean {
  return muted.has(source);
}

export function setMuted(source: string, on: boolean): void {
  if (on) muted.add(source); else muted.delete(source);
  saveMuted();
  changed();
}

export function markAllRead(): void {
  if (!unread) return;
  unread = 0;
  changed();
}

export function onNoticesChanged(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export async function dismissNotice(uid: string): Promise<void> {
  const n = items.find((i) => i.uid === uid);
  items = items.filter((i) => i.uid !== uid);
  changed();
  if (n?.serverId !== undefined) {
    try {
      await fetch('/api/notifications/dismiss', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ id: n.serverId }) });
    } catch { /* it's gone from this console either way */ }
  }
}

export async function clearNotices(): Promise<void> {
  items = [];
  unread = 0;
  changed();
  try {
    await fetch('/api/notifications/clear', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
  } catch { /* ignore */ }
}

/** A console toast, kept in the history. (The toast itself is already on screen.) */
function recordLocal(message: string, type: ToastType): void {
  push({ uid: `l${++localSeq}`, ts: Date.now(), source: 'console', sourceName: 'OpenSanctuary', level: type, title: '', body: message, sticky: false });
}

const LEVELS: ToastType[] = ['info', 'success', 'warning', 'error'];

function fromServer(raw: any): Notice | null {
  if (!raw || typeof raw.id !== 'number' || typeof raw.source !== 'string') return null;
  const level: ToastType = LEVELS.includes(raw.level) ? raw.level : 'info';
  return {
    uid: `s${raw.id}`,
    serverId: raw.id,
    ts: Number(raw.ts_ms) || Date.now(),
    source: raw.source,
    sourceName: String(raw.source_name || raw.source),
    level,
    title: String(raw.title ?? ''),
    body: String(raw.body ?? ''),
    sticky: !!raw.sticky,
    key: typeof raw.key === 'string' ? raw.key : undefined,
  };
}

/** Linux: when the host says an installed update is waiting for a restart, the console pops up the question (updates here are
 * always started by the operator, so asking is not a surprise). Set by main.ts; without it the notice is an ordinary toast. */
let restartPrompt: (() => void) | null = null;
export function setRestartPrompt(fn: () => void): void { restartPrompt = fn; }

function ingest(list: unknown[]): void {
  for (const raw of list) {
    const n = fromServer(raw);
    if (!n || (n.serverId !== undefined && n.serverId <= lastServerId)) continue;
    lastServerId = n.serverId ?? lastServerId;
    push(n);
    if (n.source === 'host' && n.key === 'restart-needed' && restartPrompt) {
      restartPrompt(); // the dialog is the notice; the bell keeps the entry
      continue;
    }
    if (muted.has(n.source)) continue;
    // Toast text: title bold, body under it; the host names the source.
    showToast(n.body || n.title, n.level, n.sticky ? 0 : n.level === 'error' ? 8000 : 5000, {
      title: n.body ? n.title : '',
      source: n.source === 'host' ? '' : n.sourceName,
      skipHistory: true,
    });
  }
}

/**
 * Starts the polling loop. Safe to call twice. Starts from what the host already holds so a
 * page reload doesn't replay old notifications as new pop-ups.
 */
export async function startNotificationFeed(): Promise<void> {
  if (feedStarted) return;
  feedStarted = true;
  setToastObserver(recordLocal);
  let backoff = 1000;
  let first = true;
  for (;;) {
    try {
      const res = await fetch(`/api/notifications?after=${first ? 0 : lastServerId}`);
      if (res.status === 401 || res.status === 403) { feedStarted = false; return; } // not a console page
      if (!res.ok) throw new Error(String(res.status));
      const body = await res.json();
      const list: unknown[] = Array.isArray(body?.notifications) ? body.notifications : [];
      if (first) {
        // History only: no pop-ups for things that happened before this page opened.
        for (const raw of list) {
          const n = fromServer(raw);
          if (n) { items.push(n); lastServerId = Math.max(lastServerId, n.serverId ?? 0); }
        }
        items.sort((a, b) => b.ts - a.ts);
        lastServerId = Math.max(lastServerId, Number(body?.latest) || 0);
        changed();
        first = false;
      } else {
        ingest(list);
      }
      backoff = 1000;
      await new Promise((r) => setTimeout(r, document.hidden ? POLL_HIDDEN_MS : POLL_MS));
    } catch {
      await new Promise((r) => setTimeout(r, backoff));
      backoff = Math.min(backoff * 2, 15000);
    }
  }
}

// --- the bell and its panel -------------------------------------------------------------------

function timeAgo(ts: number): string {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return new Date(ts).toLocaleDateString();
}

const LEVEL_ICON: Record<ToastType, string> = { info: '{icon:info}', success: '{icon:check}', warning: '{icon:warning}', error: '{icon:error}' };

function el(tag: string, cls: string, text?: string): HTMLElement {
  const e = document.createElement(tag);
  e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function iconImg(name: string): HTMLElement {
  const span = el('span', 'os-notif-ico');
  span.textContent = `{icon:${name}}`;
  return span;
}

export function initNotificationBell(button: HTMLElement): void {
  const badge = el('span', 'os-notif-badge');
  button.querySelector('.icon')?.appendChild(badge);
  const panel = el('div', 'os-notif-panel');
  panel.hidden = true;
  document.body.appendChild(panel);

  const render = () => {
    badge.textContent = unread > 99 ? '99+' : String(unread);
    badge.hidden = unread === 0;
    if (panel.hidden) return;
    panel.replaceChildren();
    const head = el('div', 'os-notif-head');
    head.appendChild(el('strong', 'os-notif-title', 'Notifications'));
    const clear = el('button', 'os-notif-clear', 'Clear all');
    clear.addEventListener('click', () => void clearNotices());
    head.appendChild(clear);
    panel.appendChild(head);
    if (!items.length) {
      panel.appendChild(el('div', 'os-notif-empty', 'Nothing yet. Plugins and the app post notices here.'));
      return;
    }
    const list = el('div', 'os-notif-list');
    for (const n of items) {
      const row = el('div', `os-notif-row os-notif-${n.level}`);
      const ico = el('span', 'os-notif-level');
      ico.textContent = LEVEL_ICON[n.level];
      const text = el('div', 'os-notif-text');
      if (n.title) text.appendChild(el('div', 'os-notif-row-title', n.title));
      if (n.body) text.appendChild(el('div', 'os-notif-row-body', n.body));
      const meta = el('div', 'os-notif-meta');
      meta.textContent = `${n.sourceName} · ${timeAgo(n.ts)}`;
      text.appendChild(meta);
      const actions = el('div', 'os-notif-actions');
      if (n.source !== 'console') {
        const mute = el('button', 'os-notif-btn');
        const isM = muted.has(n.source);
        mute.title = isM ? `Unmute ${n.sourceName}` : `Mute pop-ups from ${n.sourceName}`;
        mute.appendChild(iconImg(isM ? 'bell-off' : 'alert'));
        mute.addEventListener('click', () => setMuted(n.source, !isM));
        actions.appendChild(mute);
      }
      const x = el('button', 'os-notif-btn');
      x.title = 'Dismiss';
      x.appendChild(iconImg('close'));
      x.addEventListener('click', () => void dismissNotice(n.uid));
      actions.appendChild(x);
      row.append(ico, text, actions);
      list.appendChild(row);
    }
    panel.appendChild(list);
  };

  const close = () => { panel.hidden = true; };
  const place = () => {
    const r = button.getBoundingClientRect();
    panel.style.top = `${Math.round(r.bottom + 6)}px`;
    panel.style.right = `${Math.max(8, Math.round(window.innerWidth - r.right))}px`;
  };
  button.addEventListener('click', (e) => {
    e.stopPropagation();
    panel.hidden = !panel.hidden;
    if (!panel.hidden) { place(); markAllRead(); }
    render();
  });
  document.addEventListener('click', (e) => {
    // composedPath, not contains(): a click on a row button re-renders the list and detaches the target
    const path = e.composedPath();
    if (!panel.hidden && !path.includes(panel) && !path.includes(button)) close();
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') close(); });
  onNoticesChanged(() => {
    if (!panel.hidden) unread = 0; // looking at it
    render();
  });
  render();
}
