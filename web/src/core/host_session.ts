/**
 * Resolves this console's host session token — the credential that marks a
 * request as coming from the actual trusted console process, not a network
 * peer. There are exactly two ways to obtain it, and both reduce to "you
 * have real access to this machine," never the network:
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
 *
 * A LAN client, even a paired remote-control device, is never loopback from
 * the server's point of view and never gets this token — see
 * docs/GEMINI_COMMIT_REVIEW_2026-09-22.md #2 and #3 for what used to leak
 * before this existed.
 */

let cachedToken: string | null = null;
let resolvePromise: Promise<string | null> | null = null;

function isLoopbackHost(hostname: string): boolean {
  return hostname === '127.0.0.1' || hostname === 'localhost' || hostname === '::1' || hostname === '[::1]';
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

    if (!isLoopbackHost(window.location.hostname)) return null;

    try {
      const res = await fetch('/api/internal/host-token');
      if (res.ok) {
        const data = await res.json();
        if (data && typeof data.host_token === 'string' && data.host_token) {
          cachedToken = data.host_token;
        }
      }
    } catch (_) {
      // Not fatal — callers that need it will just fail their own auth check.
    }
    return cachedToken;
  })();
  return resolvePromise;
}

/** Header object to spread into a fetch call's headers; empty if not yet resolved. */
export function hostTokenHeader(): Record<string, string> {
  return cachedToken ? { 'x-host-token': cachedToken } : {};
}
