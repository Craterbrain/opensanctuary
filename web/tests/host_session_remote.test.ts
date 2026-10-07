import { describe, test, expect, beforeEach, afterEach } from 'bun:test';
import {
  resolveHostSessionToken,
  getHostSessionToken,
  submitManualHostToken,
  getConsoleSessionId,
  hostTokenHeader,
  __resetHostSessionTokenForTests,
} from '../src/core/host_session.ts';

/**
 * "Remote console access" (docs/CLIENT_PAIRING.md): a console on a
 * different machine bootstraps its host token from a URL fragment (the
 * server's printed terminal link/QR) or a manually-pasted token, verified
 * against POST /api/internal/verify-host-token before being trusted.
 */
describe('host_session.ts: remote console token bootstrap', () => {
  let historyState: { path: string } = { path: '/' };
  const ORIGINAL_FETCH = globalThis.fetch;

  beforeEach(() => {
    __resetHostSessionTokenForTests();
    historyState = { path: '/' };
    (globalThis as any).window = { addEventListener: () => {} };
    (globalThis as any).history = {
      replaceState: (_state: any, _title: string, url: string) => {
        historyState.path = url;
      },
    };
  });

  // Every test below replaces `globalThis.fetch` with a mock that assumes
  // every call carries a JSON body -- fine for this file's own token-
  // verification calls, but bun runs the whole suite in one process, so a
  // mock left in place after a test ends is a real window for an unrelated,
  // concurrently-scheduled test (an e2e file's plain GET requests, e.g.)
  // to hit it and crash on `init.body` being undefined instead of reaching
  // the real network. Restore immediately after each test, not just at the
  // end of the file.
  afterEach(() => {
    globalThis.fetch = ORIGINAL_FETCH;
  });

  test('a valid token in the URL fragment is cached and stripped from the visible URL', async () => {
    (globalThis as any).location = { hostname: '192.168.1.50', pathname: '/', search: '', hash: '#host_token=real-token-123' };
    (globalThis as any).window.location = (globalThis as any).location;
    (globalThis as any).fetch = (async (url: string, init: any) => {
      if (url === '/api/internal/verify-host-token') {
        const body = JSON.parse(init.body);
        return new Response(JSON.stringify({ valid: body.token === 'real-token-123' }), { status: 200 });
      }
      return new Response('{}', { status: 500 });
    }) as any;

    const token = await resolveHostSessionToken();
    expect(token).toBe('real-token-123');
    expect(getHostSessionToken()).toBe('real-token-123');
    // Fragment stripped -- the visible URL no longer carries the token.
    expect(historyState.path).toBe('/');
  });

  test('an invalid fragment token is stripped but not trusted, and resolution falls through to null (non-loopback, no other source)', async () => {
    (globalThis as any).location = { hostname: '192.168.1.50', pathname: '/', search: '', hash: '#host_token=wrong-token' };
    (globalThis as any).window.location = (globalThis as any).location;
    (globalThis as any).fetch = (async () => new Response(JSON.stringify({ valid: false }), { status: 200 })) as any;

    const token = await resolveHostSessionToken();
    expect(token).toBeNull();
    expect(historyState.path).toBe('/', 'still stripped even though invalid -- never leave a token sitting in the URL');
  });

  test('submitManualHostToken verifies before caching, and rejects a wrong token', async () => {
    (globalThis as any).location = { hostname: '192.168.1.50', pathname: '/', search: '', hash: '' };
    (globalThis as any).fetch = (async (_url: string, init: any) => {
      const body = JSON.parse(init.body);
      return new Response(JSON.stringify({ valid: body.token === 'pasted-token' }), { status: 200 });
    }) as any;

    expect(await submitManualHostToken('wrong-guess')).toBe(false);
    expect(getHostSessionToken()).toBeNull();

    expect(await submitManualHostToken('pasted-token')).toBe(true);
    expect(getHostSessionToken()).toBe('pasted-token');
  });

  test('getConsoleSessionId returns a stable, non-empty id across repeated calls', () => {
    (globalThis as any).location = { hostname: '192.168.1.50', pathname: '/', search: '', hash: '' };
    const first = getConsoleSessionId();
    const second = getConsoleSessionId();
    expect(typeof first).toBe('string');
    expect(first.length).toBeGreaterThan(0);
    expect(second).toBe(first);
  });

  test('hostTokenHeader returns both x-host-token and x-console-session-id when token is cached', async () => {
    expect(hostTokenHeader()).toEqual({});

    (globalThis as any).location = { hostname: '192.168.1.50', pathname: '/', search: '', hash: '' };
    (globalThis as any).fetch = (async (_url: string, init: any) => {
      const body = JSON.parse(init.body);
      return new Response(JSON.stringify({ valid: body.token === 'token-123' }), { status: 200 });
    }) as any;

    await submitManualHostToken('token-123');
    const headers = hostTokenHeader();
    expect(headers['x-host-token']).toBe('token-123');
    expect(typeof headers['x-console-session-id']).toBe('string');
    expect(headers['x-console-session-id'].length).toBeGreaterThan(0);
  });
});
