import { describe, test, expect, beforeEach, afterEach } from 'bun:test';
import { EngineWebSocketClient, createEngineWebSocket } from '../src/core/ws_client.ts';

describe('Unified WebSocket Client: EngineWebSocketClient', () => {
  let originalWebSocket: any;
  let originalFetch: any;
  let originalLocation: any;

  beforeEach(() => {
    originalWebSocket = globalThis.WebSocket;
    originalFetch = globalThis.fetch;
    originalLocation = globalThis.location;
  });

  afterEach(() => {
    globalThis.WebSocket = originalWebSocket;
    globalThis.fetch = originalFetch;
    globalThis.location = originalLocation;
  });

  test('resolves explicit URL or derives ws/wss from location', () => {
    const explicitClient = new EngineWebSocketClient({ url: 'ws://custom-host:9999/ws' });
    expect(explicitClient.resolveUrl()).toBe('ws://custom-host:9999/ws');

    // Test location-based derivation (http -> ws)
    globalThis.location = {
      protocol: 'http:',
      host: '192.168.1.100:8080',
    } as any;
    const httpClient = new EngineWebSocketClient();
    expect(httpClient.resolveUrl()).toBe('ws://192.168.1.100:8080/ws');

    // Test https -> wss
    globalThis.location = {
      protocol: 'https:',
      host: 'secure.church.org',
    } as any;
    const httpsClient = new EngineWebSocketClient();
    expect(httpsClient.resolveUrl()).toBe('wss://secure.church.org/ws');
  });

  test('handles incoming snapshot and preserves schedule when subsequent delta has null schedule', () => {
    let capturedSnapshot: any = null;
    const client = new EngineWebSocketClient({
      autoReconnect: false,
      onSnapshot: (snap) => {
        capturedSnapshot = snap;
      },
    });

    // Mock incoming message 1 with full schedule
    const event1 = {
      data: JSON.stringify({
        protocol_version: 1,
        state: { is_blackout: false, live_slide_index: 0 },
        schedule: { items: [{ id: 'item-1', title: 'Amazing Grace' }] },
      }),
    } as MessageEvent;

    (client as any).handleMessage(event1);
    expect(capturedSnapshot).not.toBeNull();
    expect(capturedSnapshot.schedule.items.length).toBe(1);
    expect(capturedSnapshot.schedule.items[0].title).toBe('Amazing Grace');
    expect(client.getSnapshot().schedule.items[0].id).toBe('item-1');

    // Mock incoming message 2 with schedule = null (live state delta only)
    const event2 = {
      data: JSON.stringify({
        protocol_version: 1,
        state: { is_blackout: true, live_slide_index: 1 },
        schedule: null,
      }),
    } as MessageEvent;

    (client as any).handleMessage(event2);
    expect(capturedSnapshot.state.is_blackout).toBe(true);
    expect(capturedSnapshot.state.live_slide_index).toBe(1);
    // Retained previous schedule!
    expect(capturedSnapshot.schedule).not.toBeNull();
    expect(capturedSnapshot.schedule.items.length).toBe(1);
    expect(capturedSnapshot.schedule.items[0].title).toBe('Amazing Grace');
  });

  test('validates protocol version when validateProtocol is true', () => {
    let mismatchError: string | null = null;
    const client = new EngineWebSocketClient({
      autoReconnect: false,
      validateProtocol: true,
      onProtocolMismatch: (err) => {
        mismatchError = err;
      },
    });

    // Snapshot missing valid protocol_version
    const event = {
      data: JSON.stringify({
        protocol_version: 9999, // unsupported version
        state: {},
      }),
    } as MessageEvent;

    (client as any).handleMessage(event);
    expect(mismatchError).not.toBeNull();
    expect(mismatchError).toContain('version');
  });

  test('sendCommand dispatches via WebSocket when connected', async () => {
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
    expect(sentData).toBe(JSON.stringify({ NextSlide: {} }));
  });

  test('sendCommand falls back to HTTP POST /api/command when WebSocket is disconnected', async () => {
    let httpCall: { url: string; method: string; body: any } | null = null;
    globalThis.fetch = (async (url: string, init: any) => {
      httpCall = { url, method: init.method, body: JSON.parse(init.body) };
      return {
        ok: true,
        json: async () => ({ status: 'executed' }),
      } as any;
    }) as any;

    const client = new EngineWebSocketClient({ autoReconnect: false });
    // WebSocket is null / disconnected

    const res = await client.sendCommand({ ToggleBlackout: {} });
    expect(res.success).toBe(true);
    expect(res.via).toBe('http');
    expect(httpCall).not.toBeNull();
    expect(httpCall?.url).toBe('/api/command');
    expect(httpCall?.method).toBe('POST');
    expect(httpCall?.body).toEqual({ ToggleBlackout: {} });
  });

  test('clean disconnect closes socket and cancels reconnect timer', () => {
    let closed = false;
    const mockSocket = {
      readyState: 1, // OPEN
      close: () => {
        closed = true;
      },
    };

    const client = new EngineWebSocketClient({ autoReconnect: true });
    (client as any).ws = mockSocket;
    (client as any).reconnectTimer = setTimeout(() => {}, 10000);

    client.disconnect();
    expect(closed).toBe(true);
    expect(client.getStatus()).toBe('disconnected');
    expect(client.isConnected()).toBe(false);
    expect((client as any).reconnectTimer).toBeNull();
  });
});
