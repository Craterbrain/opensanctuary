/**
 * Resolves this console's host session token — the credential that marks a
 * request as coming from the actual trusted console process, not a network
 * peer. There are three ways to obtain it now:
 *
 * 1. The native desktop webview has it injected directly into its own JS
 *    context at window creation (`window.__OS_HOST_TOKEN__`), by the Rust
 *    host itself via wry's initialization-script hook — see
 *    `with_initialization_script` in src/webview/mod.rs. It is never sent
 *    over HTTP or WebSocket at all.
 * 2. A plain browser tab reached at 127.0.0.1/localhost can fetch it from
 *    `GET /api/internal/host-token`, which the server only answers when the
 *    request's own TCP connection originates from loopback — the same
 *    address the console's startup banner already tells operators to use.
 * 3. A console on a *different* machine (docs/CLIENT_PAIRING.md "Remote
 *    console access" — a headless server driven from the operator's own
 *    laptop) carries the token in the URL fragment (`#host_token=...`, the
 *    same way `remote_client.ts` carries a paired device's token), printed
 *    to the server's own terminal (text + QR code) or pasted in manually.
 *    Verified against `POST /api/internal/verify-host-token` before being
 *    trusted, then cached in `sessionStorage` so a reload doesn't need the
 *    link/token again, and stripped from the visible URL immediately so it
 *    isn't left sitting there if the address is shared or bookmarked.
 *
 * A LAN client, even a paired remote-control device, is never loopback and
 * never knows a bare URL fragment it wasn't handed out-of-band, so none of
 * this changes who *can* obtain the token.
 */

const CONSOLE_TOKEN_STORAGE_KEY = 'os_console_host_token';
const CONSOLE_SESSION_ID_STORAGE_KEY = 'os_console_session_id';

let cachedToken: string | null = null;
let resolvePromise: Promise<string | null> | null = null;

function isLoopbackHost(hostname: string): boolean {
  return hostname === '127.0.0.1' || hostname === 'localhost' || hostname === '::1' || hostname === '[::1]';
}

function readTokenFromLocationHash(): string | null {
  if (typeof location === 'undefined') return null;
  const hash = location.hash || '';
  if (hash.length <= 1) return null;
  const params = new URLSearchParams(hash.slice(1));
  return params.get('host_token');
}

async function verifyToken(token: string): Promise<boolean> {
  try {
    const res = await fetch('/api/internal/verify-host-token', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ token }),
    });
    if (!res.ok) return false;
    const data = await res.json();
    return !!(data && data.valid);
  } catch (e) {
    console.warn('[HostSession] Token verification request failed:', e);
    return false;
  }
}

/** Synchronous best-effort read — null until `resolveHostSessionToken()` has completed at least once. */
export function getHostSessionToken(): string | null {
  return cachedToken;
}

/** Resolves (and caches) the host session token. Safe to call repeatedly; only resolves once. */
export function resolveHostSessionToken(): Promise<string | null> {
  if (resolvePromise) return resolvePromise;
  resolvePromise = (async () => {
    if (typeof window === 'undefined') return null;

    const injected = (window as any).__OS_HOST_TOKEN__;
    if (typeof injected === 'string' && injected) {
      cachedToken = injected;
      return cachedToken;
    }

    // A remote console: the token arrived via URL fragment (the printed
    // terminal link/QR) or was manually entered and cached from a previous
    // load of this same tab.
    const fromHash = readTokenFromLocationHash();
    if (fromHash) {
      // Strip it out of the visible URL/address bar immediately, whether or
      // not it turns out valid, so it isn't left sitting there if the URL
      // is shared or bookmarked -- same reasoning as /remote's token.
      try {
        if (typeof history !== 'undefined' && history.replaceState) {
          history.replaceState(null, '', location.pathname + location.search);
        }
      } catch (e) {
        console.warn('[HostSession] Failed to strip host_token from URL:', e);
      }

      if (await verifyToken(fromHash)) {
        cachedToken = fromHash;
        try {
          if (typeof sessionStorage !== 'undefined') sessionStorage.setItem(CONSOLE_TOKEN_STORAGE_KEY, fromHash);
        } catch (e) {
          console.warn('[HostSession] Failed to persist host token to sessionStorage:', e);
        }
        return cachedToken;
      }
      // An invalid/stale link -- fall through to the other resolution paths
      // (a cached token from a prior load, or the loopback fetch) instead
      // of giving up outright.
    }

    try {
      if (typeof sessionStorage !== 'undefined') {
        const cached = sessionStorage.getItem(CONSOLE_TOKEN_STORAGE_KEY);
        if (cached && await verifyToken(cached)) {
          cachedToken = cached;
          return cachedToken;
        }
      }
    } catch (e) {
      console.warn('[HostSession] Failed to read cached host token from sessionStorage:', e);
    }

    // `window` existing doesn't guarantee `window.location` does -- e.g. a
    // test harness (or any other partial DOM polyfill) that stubs `window`
    // without a `location` property. Treat that the same as "can't tell,
    // so don't assume loopback" rather than throwing.
    const hostname = window.location && typeof window.location.hostname === 'string' ? window.location.hostname : null;
    if (!hostname || !isLoopbackHost(hostname)) return null;

    try {
      const res = await fetch('/api/internal/host-token');
      if (res.ok) {
        const data = await res.json();
        if (data && typeof data.host_token === 'string' && data.host_token) {
          cachedToken = data.host_token;
        }
      }
    } catch (e) {
      // Not fatal — callers that need it will just fail their own auth check.
      console.warn('[HostSession] Loopback host-token fetch failed:', e);
    }
    return cachedToken;
  })();
  return resolvePromise;
}

/**
 * Verifies and caches a token an operator pasted in by hand (the "give an
 * option to manually enter the token" flow — see the console's own
 * not-authenticated banner in app_core.ts). Returns whether it was valid.
 */
export async function submitManualHostToken(token: string): Promise<boolean> {
  const trimmed = token.trim();
  if (!trimmed) return false;
  if (!(await verifyToken(trimmed))) return false;
  cachedToken = trimmed;
  try {
    if (typeof sessionStorage !== 'undefined') sessionStorage.setItem(CONSOLE_TOKEN_STORAGE_KEY, trimmed);
  } catch (e) {
    console.warn('[HostSession] Failed to persist manually-entered host token to sessionStorage:', e);
  }
  return true;
}

/** Header object to spread into a fetch call's headers; empty if not yet resolved. */
export function hostTokenHeader(): Record<string, string> {
  return cachedToken ? { 'x-host-token': cachedToken, 'x-console-session-id': getConsoleSessionId() } : {};
}

/**
 * Makes every same-origin `/api/` request from this page carry the host
 * token once it's resolved. The server gates all state-changing routes (and a
 * few sensitive reads) on `x-host-token`, so this keeps the many plain
 * `fetch('/api/...')` call sites working without each one remembering to
 * spread `hostTokenHeader()`. Pages that never obtain a token (live, stage,
 * remote) simply send nothing extra. An explicit `x-host-token` the caller
 * set is left alone, and nothing is ever attached to cross-origin requests.
 */
export function installHostTokenFetch(): void {
  if (typeof window === 'undefined' || typeof window.fetch !== 'function') return;
  const w = window as any;
  if (w.__osHostTokenFetchInstalled) return;
  w.__osHostTokenFetchInstalled = true;
  const originalFetch = window.fetch.bind(window);
  window.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
    if (!cachedToken) return originalFetch(input, init);
    let url: URL;
    try {
      const raw = input instanceof Request ? input.url : String(input);
      url = new URL(raw, window.location.href);
    } catch {
      return originalFetch(input, init);
    }
    if (url.origin !== window.location.origin || !url.pathname.startsWith('/api/')) {
      return originalFetch(input, init);
    }
    const headers = new Headers(init?.headers || (input instanceof Request ? input.headers : undefined));
    if (!headers.has('x-host-token')) {
      for (const [k, v] of Object.entries(hostTokenHeader())) headers.set(k, v);
    }
    return originalFetch(input, { ...init, headers });
  }) as typeof window.fetch;
}

installHostTokenFetch();

let cachedConsoleSessionId: string | null = null;

/**
 * A random id identifying this browser tab as "one console" for the "one
 * console at a time, first one connected wins" rule (see
 * `AppState::try_claim_console` in src/api/ws.rs and
 * docs/CLIENT_PAIRING.md). Persisted in `sessionStorage` so reloading this
 * same tab keeps the same identity instead of contending with its own
 * just-abandoned connection for the lock; a brand new tab/window always
 * gets a fresh one.
 */
export function getConsoleSessionId(): string {
  if (cachedConsoleSessionId) return cachedConsoleSessionId;
  try {
    if (typeof sessionStorage !== 'undefined') {
      const existing = sessionStorage.getItem(CONSOLE_SESSION_ID_STORAGE_KEY);
      if (existing) {
        cachedConsoleSessionId = existing;
        return cachedConsoleSessionId;
      }
    }
  } catch (e) {
    console.warn('[HostSession] Failed to read console session id from sessionStorage:', e);
  }

  const fresh = (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function')
    ? crypto.randomUUID()
    : `console-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  cachedConsoleSessionId = fresh;
  try {
    if (typeof sessionStorage !== 'undefined') sessionStorage.setItem(CONSOLE_SESSION_ID_STORAGE_KEY, fresh);
  } catch (e) {
    console.warn('[HostSession] Failed to persist console session id to sessionStorage:', e);
  }
  return fresh;
}

/**
 * Test-only: clears all cached token/session state, as if this were a fresh
 * page load. `bun test` runs every test file in one process, so this
 * module's cache is otherwise shared across the whole suite — whichever
 * test happens to resolve first (with or without a mocked
 * `window.__OS_HOST_TOKEN__`) would otherwise permanently decide the value
 * every later test observes. Never called from production code.
 */
export function __resetHostSessionTokenForTests(): void {
  cachedToken = null;
  resolvePromise = null;
  cachedConsoleSessionId = null;
  try {
    if (typeof sessionStorage !== 'undefined') {
      sessionStorage.removeItem(CONSOLE_TOKEN_STORAGE_KEY);
      sessionStorage.removeItem(CONSOLE_SESSION_ID_STORAGE_KEY);
    }
  } catch (_) {}
}
