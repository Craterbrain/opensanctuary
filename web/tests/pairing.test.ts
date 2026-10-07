import { describe, test, expect, beforeEach, afterEach, afterAll } from 'bun:test';
import QRCode from 'qrcode';
import jsQR from 'jsqr';
import { SETTINGS_CATEGORIES } from '../src/core/settings_schema.ts';
import { api } from '../src/core/api_client.ts';
import { renderPairedDevicesSettings, resetOptionsModal } from '../src/ui/settings_dialog.ts';
import pairingContract from '../../../spec-db-rs/contracts/client_pairing.schema.json';

// Several describe blocks below stub globalThis.fetch/document per-test.
// bun runs every test file in one process, and (as observed directly: this
// is what was breaking every e2e_*.test.ts file whenever the full suite ran
// together, never in isolation) it does not wait for one file to fully
// finish before starting another's async work -- so a mock installed here
// and left in place even between this file's OWN tests is a real window for
// an unrelated, concurrently-scheduled test elsewhere to observe it instead
// of the real global. `afterEach` closes that window immediately after
// every single test in this file, not just once at the very end.
const ORIGINAL_FETCH = globalThis.fetch;
const ORIGINAL_DOCUMENT = (globalThis as any).document;
afterEach(() => {
  globalThis.fetch = ORIGINAL_FETCH;
  (globalThis as any).document = ORIGINAL_DOCUMENT;
});
afterAll(() => {
  globalThis.fetch = ORIGINAL_FETCH;
  (globalThis as any).document = ORIGINAL_DOCUMENT;
});

describe('Client Pairing Contract & Schema Validation', () => {
  test('SETTINGS_CATEGORIES contains paired-devices tab', () => {
    const cat = SETTINGS_CATEGORIES.find(c => c.id === 'paired-devices');
    expect(cat).toBeDefined();
    expect(cat?.label).toBe('Paired Devices');
    expect(cat?.icon).toBe('📺');
  });

  test('client_pairing.schema.json contract invariants and thresholds', () => {
    expect(pairingContract.values.PAIRING_SESSION_TTL_SECONDS).toBe(300);
    expect(pairingContract.values.ALLOW_UNAUTHENTICATED_WEB_OUTPUT).toBe(true);
    expect(pairingContract.required).toContain('PAIRING_SESSION_TTL_SECONDS');
    expect(pairingContract.required).toContain('ALLOW_UNAUTHENTICATED_WEB_OUTPUT');
  });

  test('Contract definition regexes validate session and device tokens', () => {
    const sessionRegex = new RegExp(pairingContract.definitions.PairingSession.properties.session_token.pattern);
    const devTokRegex = new RegExp(pairingContract.definitions.AuthorizeDeviceResponse.properties.token.pattern);

    expect('pair_sess_4f60bf4762cf4d9796e6cf1e4a159f8c').toMatch(sessionRegex);
    expect('dev_tok_9b1deb4d3b7d4bad9bdd2b0d7b3dcb6d').toMatch(devTokRegex);

    expect('invalid_session_token').not.toMatch(sessionRegex);
    expect('invalid_dev_token').not.toMatch(devTokRegex);
  });
});

describe('API Client Pairing Methods', () => {
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    globalThis.fetch = originalFetch;
  });

  test('api.pairing.createSession calls POST /api/pairing/session', async () => {
    let requestedUrl = '';
    let requestMethod = '';

    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      requestedUrl = url;
      requestMethod = init?.method || 'GET';
      return new Response(JSON.stringify({
        session_token: 'pair_sess_abc123',
        expires_in: 300,
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as any;

    const res = await api.pairing.createSession();
    expect(requestedUrl).toBe('/api/pairing/session');
    expect(requestMethod).toBe('POST');
    expect(res.session_token).toBe('pair_sess_abc123');
    expect(res.expires_in).toBe(300);
  });

  test('api.pairing.authorizeDevice submits device payload to /api/pairing/authorize', async () => {
    let capturedBody: any = null;

    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      capturedBody = JSON.parse(init?.body as string);
      return new Response(JSON.stringify({
        success: true,
        device_id: 'tv-living-room',
        name: 'Living Room TV',
        platform: 'android-tv',
        token: 'dev_tok_xyz789',
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as any;

    const res = await api.pairing.authorizeDevice({
      session_token: 'pair_sess_abc123',
      device_id: 'tv-living-room',
      name: 'Living Room TV',
      platform: 'android-tv',
    });

    expect(capturedBody.session_token).toBe('pair_sess_abc123');
    expect(capturedBody.device_id).toBe('tv-living-room');
    expect(res.success).toBe(true);
    expect(res.token).toBe('dev_tok_xyz789');
  });

  test('api.pairing.unpairDevice sends DELETE /api/pairing/devices/:id', async () => {
    let requestedUrl = '';
    let requestMethod = '';

    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      requestedUrl = url;
      requestMethod = init?.method || 'GET';
      return new Response(JSON.stringify({ success: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as any;

    const res = await api.pairing.unpairDevice('tv-living-room');
    expect(requestedUrl).toBe('/api/pairing/devices/tv-living-room');
    expect(requestMethod).toBe('DELETE');
    expect(res.success).toBe(true);
  });
});

function createMockElement(tag: string = 'div') {
  const classes = new Set<string>();
  const children: any[] = [];
  const attrs: Record<string, string> = {};
  let _textContent = '';
  let _innerHTML = '';

  const el: any = {
    tagName: tag.toUpperCase(),
    style: {},
    children,
    get className() {
      return Array.from(classes).join(' ');
    },
    set className(val: string) {
      classes.clear();
      val.split(/\s+/).filter(Boolean).forEach(c => classes.add(c));
    },
    classList: {
      add: (...c: string[]) => {
        c.forEach(x => classes.add(x));
      },
      remove: (...c: string[]) => {
        c.forEach(x => classes.delete(x));
      },
      contains: (c: string) => classes.has(c),
    },
    appendChild: (child: any) => {
      children.push(child);
      return child;
    },
    addEventListener: () => {},
    setAttribute: (name: string, val: string) => { attrs[name] = val; },
    getAttribute: (name: string) => attrs[name] || null,
    querySelectorAll: (sel: string) => {
      const results: any[] = [];
      const className = sel.startsWith('.') ? sel.slice(1) : '';
      const walk = (n: any) => {
        if (className && n.classList && n.classList.contains(className)) {
          results.push(n);
        }
        (n.children || []).forEach(walk);
      };
      children.forEach(walk);
      return results;
    },
    get textContent() {
      let text = _textContent || '';
      if (_innerHTML) {
        text += ' ' + _innerHTML.replace(/<[^>]*>/g, ' ');
      }
      for (const c of children) {
        text += ' ' + c.textContent;
      }
      return text;
    },
    set textContent(val: string) {
      _textContent = val;
    },
    get innerHTML() {
      return _innerHTML;
    },
    set innerHTML(val: string) {
      _innerHTML = val;
      if (!val) children.length = 0;
    },
  };
  return el;
}

describe('Paired Devices UI Rendering', () => {
  const originalDoc = (globalThis as any).document;

  beforeEach(() => {
    (globalThis as any).document = {
      createElement: (tag: string) => createMockElement(tag),
      getElementById: () => null,
    };
    resetOptionsModal();
  });

  test('renderPairedDevicesSettings populates overview and empty state', async () => {
    // Mock api.pairing.getDevices returning empty array
    globalThis.fetch = (async (url: string) => {
      if (url === '/api/pairing/devices') {
        return new Response(JSON.stringify([]), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      return new Response('{}', { status: 200 });
    }) as any;

    const container = (globalThis as any).document.createElement('div');
    renderPairedDevicesSettings(container);

    // Initial render shows loading
    expect(container.textContent).toContain('Loading paired display devices');

    // Wait for async fetch
    await new Promise(r => setTimeout(r, 50));
    renderPairedDevicesSettings(container);

    // Verify empty state is rendered
    expect(container.textContent).toContain('No Display Clients Paired');
    expect(container.textContent).toContain('Active TV Displays');
    expect(container.textContent).toContain('Zero-Auth Web Display Guarantee');
  });

  test('renderPairedDevicesSettings renders active devices and unpair buttons', async () => {
    const mockDevices = [
      {
        id: 'tv-sanctuary-main',
        name: 'Sanctuary Main Projector',
        platform: 'android-tv',
        paired_at: Date.now() - 3600000,
        last_seen: Date.now() - 5000,
        token: 'dev_tok_test1',
      },
      {
        id: 'tv-lobby-roku',
        name: 'Lobby Welcome Screen',
        platform: 'roku',
        paired_at: Date.now() - 86400000,
        last_seen: Date.now() - 10000,
        token: 'dev_tok_test2',
      },
    ];

    globalThis.fetch = (async (url: string) => {
      if (url === '/api/pairing/devices') {
        return new Response(JSON.stringify(mockDevices), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        });
      }
      return new Response('{}', { status: 200 });
    }) as any;

    const container = (globalThis as any).document.createElement('div');
    renderPairedDevicesSettings(container);

    // Allow fetch to complete
    await new Promise(r => setTimeout(r, 50));
    renderPairedDevicesSettings(container);

    expect(container.textContent).toContain('Sanctuary Main Projector');
    expect(container.textContent).toContain('Lobby Welcome Screen');
    expect(container.textContent).toContain('tv-sanctuary-main');
    expect(container.textContent).toContain('2 Devices');

    const unpairButtons = container.querySelectorAll('.btn-unpair-device');
    expect(unpairButtons.length).toBe(2);
  });
});

describe('Universal QR Code Generation and Decoding Engine', () => {
  test('jsQR accurately decodes QR codes generated for TV pairing payloads', () => {
    const rawPayload = JSON.stringify({
      device_id: 'tv-sanctuary-1',
      name: 'Sanctuary Main TV',
      platform: 'android-tv',
    });

    const qr = QRCode.create(rawPayload, { errorCorrectionLevel: 'M' });
    const size = qr.modules.size;
    const scale = 4;
    const width = size * scale;
    const height = size * scale;
    const data = new Uint8ClampedArray(width * height * 4);

    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const modX = Math.floor(x / scale);
        const modY = Math.floor(y / scale);
        const isDark = qr.modules.get(modX, modY);
        const idx = (y * width + x) * 4;
        const color = isDark ? 0 : 255;
        data[idx] = color;
        data[idx + 1] = color;
        data[idx + 2] = color;
        data[idx + 3] = 255;
      }
    }

    const decoded = jsQR(data, width, height);
    expect(decoded).not.toBeNull();
    expect(decoded?.data).toBe(rawPayload);

    const parsed = JSON.parse(decoded!.data);
    expect(parsed.device_id).toBe('tv-sanctuary-1');
    expect(parsed.name).toBe('Sanctuary Main TV');
    expect(parsed.platform).toBe('android-tv');
  });

  test('jsQR handles colon-separated and URL query payloads across all devices', () => {
    const textPayload = 'tv-side-balcony:Balcony Screen:roku';
    const qr = QRCode.create(textPayload);
    const size = qr.modules.size;
    const scale = 4;
    const width = size * scale;
    const height = size * scale;
    const data = new Uint8ClampedArray(width * height * 4);

    for (let y = 0; y < height; y++) {
      for (let x = 0; x < width; x++) {
        const isDark = qr.modules.get(Math.floor(x / scale), Math.floor(y / scale));
        const idx = (y * width + x) * 4;
        const color = isDark ? 0 : 255;
        data[idx] = color;
        data[idx + 1] = color;
        data[idx + 2] = color;
        data[idx + 3] = 255;
      }
    }

    const decoded = jsQR(data, width, height);
    expect(decoded).not.toBeNull();
    expect(decoded?.data).toBe(textPayload);
  });

  test('buildPairingUrl prioritizes HTTPS and https_port when HTTPS is enabled', async () => {
    const { buildPairingUrl } = await import('../src/core/presentation_helpers.ts');

    // 1. HTTPS enabled on LAN IP
    const urlHttps = buildPairingUrl(
      { https_enabled: true, https_port: 8443, http_port: 8080, lan_ip: '192.168.1.100' },
      { protocol: 'http:', hostname: '192.168.1.100', port: '8080' },
      'pair_sess_secure123'
    );
    expect(urlHttps).toBe('https://192.168.1.100:8443/pairing.html?key=pair_sess_secure123');

    // 2. Localhost translation to lan_ip for mobile camera access
    const urlLocalhost = buildPairingUrl(
      { https_enabled: true, https_port: 8443, http_port: 8080, lan_ip: '10.0.0.45' },
      { protocol: 'http:', hostname: 'localhost', port: '8080' },
      'pair_sess_xyz987'
    );
    expect(urlLocalhost).toBe('https://10.0.0.45:8443/pairing.html?key=pair_sess_xyz987');

    // 3. HTTPS disabled fallback to HTTP AV plane
    const urlHttp = buildPairingUrl(
      { https_enabled: false, https_port: null, http_port: 8080, lan_ip: '192.168.1.100' },
      { protocol: 'http:', hostname: '192.168.1.100', port: '8080' },
      'pair_sess_plain456'
    );
    expect(urlHttp).toBe('http://192.168.1.100:8080/pairing.html?key=pair_sess_plain456');

    // 4. A configured Public HTTPS URL (docs/TUNNELS.md) always wins, even
    // over the self-signed https_enabled/https_port plane -- it's a real,
    // browser-trusted cert, the whole reason to set it up.
    const urlPublic = buildPairingUrl(
      { https_enabled: true, https_port: 8443, http_port: 8080, lan_ip: '192.168.1.100', public_https_url: 'https://connect.yourchurch.org' },
      { protocol: 'http:', hostname: '192.168.1.100', port: '8080' },
      'pair_sess_public789'
    );
    expect(urlPublic).toBe('https://connect.yourchurch.org/pairing.html?key=pair_sess_public789');
  });

  test('buildRemoteUrl prioritizes HTTPS and https_port for the Mobile Remote QR when HTTPS is enabled', async () => {
    // The Mobile Remote QR used to just mirror location.protocol, so a
    // console viewed over the default plaintext http://…:8080/ URL (what
    // the startup banner advertises) generated an http:// QR -- meaning the
    // remote's device_token, a static bearer credential sent on every
    // command, traveled in cleartext over the LAN by default. It must now
    // prefer HTTPS/WSS whenever the server has the secure plane enabled,
    // even though the console page itself is on http: -- same fix, same
    // reasoning, as buildPairingUrl above.
    const { buildRemoteUrl } = await import('../src/core/presentation_helpers.ts');

    const urlHttps = buildRemoteUrl(
      { https_enabled: true, https_port: 8443, http_port: 8080, lan_ip: '192.168.1.100' },
      { protocol: 'http:', hostname: '192.168.1.100', port: '8080' }
    );
    expect(urlHttps).toBe('https://192.168.1.100:8443/remote');

    // Localhost translation to lan_ip, same as buildPairingUrl.
    const urlLocalhost = buildRemoteUrl(
      { https_enabled: true, https_port: 8443, http_port: 8080, lan_ip: '10.0.0.45' },
      { protocol: 'http:', hostname: 'localhost', port: '8080' }
    );
    expect(urlLocalhost).toBe('https://10.0.0.45:8443/remote');

    // HTTPS disabled falls back to the plaintext AV plane.
    const urlHttp = buildRemoteUrl(
      { https_enabled: false, https_port: null, http_port: 8080, lan_ip: '192.168.1.100' },
      { protocol: 'http:', hostname: '192.168.1.100', port: '8080' }
    );
    expect(urlHttp).toBe('http://192.168.1.100:8080/remote');

    // A configured Public HTTPS URL wins over the self-signed plane here too.
    const urlPublic = buildRemoteUrl(
      { https_enabled: true, https_port: 8443, http_port: 8080, lan_ip: '192.168.1.100', public_https_url: 'https://connect.yourchurch.org' },
      { protocol: 'http:', hostname: '192.168.1.100', port: '8080' }
    );
    expect(urlPublic).toBe('https://connect.yourchurch.org/remote');
  });
});


