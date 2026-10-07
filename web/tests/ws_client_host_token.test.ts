import { describe, test, expect, beforeEach, afterEach } from 'bun:test';
import { EngineWebSocketClient } from '../src/core/ws_client.ts';
import { __resetHostSessionTokenForTests } from '../src/core/host_session.ts';

/**
 * /api/command and /ws now require the host session token for callers with
 * no paired-device token (see docs/CLIENT_PAIRING.md "Console (host)
 * authentication"). `host_session.ts` caches its resolved token in
 * module-level state shared across every test file in one `bun test` run,
 * so each test here resets it first via `__resetHostSessionTokenForTests`
 * rather than depending on being the first thing in the whole suite to
 * touch it.
 */
describe('EngineWebSocketClient.sendCommand: host token attachment', () => {
  const ORIGINAL_FETCH = globalThis.fetch;

  beforeEach(() => {
    __resetHostSessionTokenForTests();
    (globalThis as any).window = { addEventListener: () => {} };
  });

  // One test below replaces globalThis.fetch; bun runs the whole suite in
  // one process, so leaving that in place after the test ends is a real
  // window for an unrelated, concurrently-scheduled test elsewhere to hit
  // the mock instead of the real network (see host_session_remote.test.ts's
  // matching fix for the same hazard, and pairing.test.ts's original
  // comment describing it).
  afterEach(() => {
    globalThis.fetch = ORIGINAL_FETCH;
  });

  test('attaches host_token (WS envelope) and x-host-token (HTTP fallback) when the console has one', async () => {
    (globalThis as any).window.__OS_HOST_TOKEN__ = 'test-host-token-abc';

    let sentData: string | null = null;
    const mockSocket = {
      readyState: 1, // OPEN
      send: (data: string) => {
        sentData = data;
      },
      close: () => {},
    };

    const client = new EngineWebSocketClient({ autoReconnect: false });
    (client as any).ws = mockSocket;

    const res = await client.sendCommand({ NextSlide: {} });
    expect(res.success).toBe(true);
    expect(res.via).toBe('websocket');
    const sent = JSON.parse(sentData!);
    expect(sent.cmd).toEqual({ NextSlide: {} });
    expect(sent.host_token).toBe('test-host-token-abc');
    expect(typeof sent.console_session_id).toBe('string');
    expect(sent.console_session_id.length).toBeGreaterThan(0);
    expect(sent.device_token).toBeUndefined();
  });

  test('a paired-remote client (deviceToken configured) still uses device_token, never host_token', async () => {
    (globalThis as any).window.__OS_HOST_TOKEN__ = 'should-not-be-used';

    let sentData: string | null = null;
    const mockSocket = {
      readyState: 1,
      send: (data: string) => {
        sentData = data;
      },
      close: () => {},
    };

    const client = new EngineWebSocketClient({ autoReconnect: false, deviceToken: 'dev_tok_xyz' });
    (client as any).ws = mockSocket;

    await client.sendCommand({ ToggleBlackout: {} });
    expect(sentData).toBe(JSON.stringify({ cmd: { ToggleBlackout: {} }, device_token: 'dev_tok_xyz' }));
  });

  test('HTTP fallback attaches x-host-token when WebSocket is disconnected', async () => {
    (globalThis as any).window.__OS_HOST_TOKEN__ = 'test-host-token-http';

    let httpHeaders: any = null;
    (globalThis as any).fetch = (async (_url: string, init: any) => {
      httpHeaders = init.headers;
      return { ok: true, json: async () => ({ status: 'executed' }) } as any;
    }) as any;

    const client = new EngineWebSocketClient({ autoReconnect: false });
    // WebSocket stays null/disconnected -- forces the HTTP fallback path.

    const res = await client.sendCommand({ ToggleBlackout: {} });
    expect(res.success).toBe(true);
    expect(res.via).toBe('http');
    expect(httpHeaders['x-host-token']).toBe('test-host-token-http');
    expect(typeof httpHeaders['x-console-session-id']).toBe('string');
    expect(httpHeaders['x-console-session-id'].length).toBeGreaterThan(0);
  });

  test('sends the command with no envelope at all when no token can be resolved (headless/no window)', async () => {
    delete (globalThis as any).window;

    let sentData: string | null = null;
    const mockSocket = {
      readyState: 1,
      send: (data: string) => {
        sentData = data;
      },
      close: () => {},
    };

    const client = new EngineWebSocketClient({ autoReconnect: false });
    (client as any).ws = mockSocket;

    await client.sendCommand({ NextSlide: {} });
    expect(sentData).toBe(JSON.stringify({ NextSlide: {} }));
  });
});
