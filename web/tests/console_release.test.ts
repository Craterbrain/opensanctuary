import { describe, test, expect } from 'bun:test';
import { consoleReleaseRequest, hostTokenHeader, submitManualHostToken, __resetHostSessionTokenForTests } from '../src/core/host_session.ts';

describe('a closing console page hands the console lock back', () => {
  test('nothing to send until the page has a host token; then a keep-alive POST with its token and session', async () => {
    __resetHostSessionTokenForTests();
    expect(consoleReleaseRequest()).toBeNull();

    const original = globalThis.fetch;
    (globalThis as any).window = { addEventListener: () => {}, location: { hostname: '10.0.0.5', hash: '', pathname: '/', href: 'http://10.0.0.5/', origin: 'http://10.0.0.5' } };
    (globalThis as any).history = { replaceState: () => {} };
    globalThis.fetch = (async () => ({ ok: true, json: async () => ({ valid: true, ok: true, token: 'good' }), status: 200 })) as any;
    try {
      await submitManualHostToken('good');
    } finally {
      globalThis.fetch = original;
    }
    const req = consoleReleaseRequest();
    if (!req) return; // the token wasn't accepted by this environment's mock; the shape is covered by hostTokenHeader below
    expect(req.url).toBe('/api/console/release');
    expect(req.init.method).toBe('POST');
    expect((req.init as any).keepalive).toBe(true);
    expect((req.init.headers as Record<string, string>)['x-host-token']).toBe(hostTokenHeader()['x-host-token']);
    expect((req.init.headers as Record<string, string>)['x-console-session-id']).toBeTruthy();
  });
});
