import { describe, test, expect, afterEach } from 'bun:test';
import { SETTINGS_CATEGORIES, SETTINGS_SCHEMA } from '../src/core/settings_schema.ts';
import { api } from '../src/core/api_client.ts';
import { __resetHostSessionTokenForTests } from '../src/core/host_session.ts';
import { saveNetworkSettings, checkForUpdates, loadSettingsServerInfo, settingRowVisible } from '../src/ui/settings_dialog.ts';

// bun runs every test file in one process: whatever a test swaps into the globals is put back
// right after it, so no other file ever sees a stub.
const ORIGINAL = { fetch: globalThis.fetch, document: (globalThis as any).document, window: (globalThis as any).window, hadWindow: 'window' in globalThis };
afterEach(() => {
  globalThis.fetch = ORIGINAL.fetch;
  (globalThis as any).document = ORIGINAL.document;
  if (ORIGINAL.hadWindow) (globalThis as any).window = ORIGINAL.window;
  else delete (globalThis as any).window;
});

const row = (key: string) => SETTINGS_SCHEMA.find((s) => s.key === key);
const jsonResponse = (body: any) => ({ ok: true, headers: { get: () => 'application/json' }, json: async () => body, text: async () => JSON.stringify(body) });

describe('Settings schema', () => {
  test('the bespoke categories exist', () => {
    for (const [id, label] of [['network', 'Network'], ['storage', 'Storage'], ['paired-devices', 'Paired Devices'], ['about', 'About']]) {
      expect(SETTINGS_CATEGORIES.find((c) => c.id === id)?.label).toBe(label);
    }
  });

  test('every row the dialog code relies on is in its category with the right control', () => {
    const expected: Array<[string, string, string?]> = [
      ['networkHostname', 'network'], ['networkPort', 'network'], ['networkDedicatedMac', 'network'], ['networkBroadcastAll', 'network'],
      ['securityCspMode', 'network', 'select'], ['securityCorsMode', 'network', 'select'], ['securityFrameOptions', 'network', 'select'],
      ['dataDirectory', 'storage', 'readonly'], ['moveDataDirectory', 'storage', 'action'],
      ['checkForUpdates', 'about', 'action'], ['restartToFinishUpdate', 'about', 'action'],
    ];
    for (const [key, category, control] of expected) {
      expect({ key, category: row(key)?.category }).toEqual({ key, category });
      if (control) expect({ key, control: row(key)?.control }).toEqual({ key, control });
    }
    const values = (key: string) => row(key)!.options!.map((o) => o.value);
    expect(values('securityCspMode')).toEqual(expect.arrayContaining(['balanced', 'strict', 'disabled']));
    expect(values('securityCorsMode')).toEqual(expect.arrayContaining(['permissive', 'restricted']));
    expect(values('securityFrameOptions')).toEqual(expect.arrayContaining(['sameorigin', 'deny', 'disabled']));
  });
});

describe('Settings > Network', () => {
  test('saving writes the settings and the enabled interfaces', async () => {
    const fields: Record<string, any> = {
      'network-hostname-input': { value: 'sanctuary-main' },
      'network-port-input': { value: '8095' },
      'network-broadcast-all-toggle': { checked: true },
    };
    const boxes = [{ checked: true, dataset: { iface: 'eth0' } }, { checked: false, dataset: { iface: 'wlan0' } }];
    (globalThis as any).document = {
      getElementById: (id: string) => fields[id] || null,
      querySelectorAll: (sel: string) => (sel === '.network-iface-checkbox' ? boxes : []),
      createElement: () => ({ className: '', innerHTML: '', textContent: '', style: {}, appendChild: () => {}, addEventListener: () => {} }),
    };
    const calls: Record<string, { method: string; body: any }> = {};
    globalThis.fetch = (async (url: string, init?: any) => {
      calls[url] = { method: init?.method || 'GET', body: init?.body ? JSON.parse(init.body) : null };
      return jsonResponse({ success: true });
    }) as any;

    expect(await saveNetworkSettings()).toBe(true);
    expect(calls['/api/settings']).toEqual({ method: 'POST', body: expect.objectContaining({ networkHostname: 'sanctuary-main', networkPort: '8095', networkBroadcastAll: 'true' }) });
    expect(calls['/api/network/interfaces']).toEqual({ method: 'POST', body: expect.objectContaining({ enabled_interfaces: ['eth0'], broadcast_all: true }) });
  });
});

describe('Settings > Storage', () => {
  test('moving the data folder posts the target and the delete-source choice', async () => {
    // moveDataDir resolves a host token first; that cache is shared by every file in the process.
    __resetHostSessionTokenForTests();
    (globalThis as any).window = { location: { hostname: '192.168.1.50', pathname: '/', search: '', hash: '' } };
    let sent: any = null;
    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      sent = { url, method: init?.method, body: JSON.parse(init!.body as string) };
      return new Response(JSON.stringify({ ok: true, files_copied: 5, sha_verified: true, source_deleted: true, restart_required: true }), { status: 200, headers: { 'Content-Type': 'application/json' } });
    }) as any;

    const res = await api.system.moveDataDir('/new/path', true);
    expect(sent).toEqual({ url: '/api/system/move-data-dir', method: 'POST', body: { target_dir: '/new/path', delete_source: true } });
    expect(res).toMatchObject({ ok: true, files_copied: 5, restart_required: true });
  });
});

describe('Settings > About', () => {
  test('Check for Updates always re-enables its button: up to date, a failed check, or no network', async () => {
    // renderSettingsContent()/showConfirmDialog() no-op when every element lookup is null.
    (globalThis as any).document = { getElementById: () => null };
    const status = (error: string | null) => async () => jsonResponse({ status: { checked_at_ms: 1, current_version: '0.2.0', latest: null, error } });
    for (const answer of [status(null), status('Failed to reach GitHub: timed out'), async () => { throw new Error('network down'); }]) {
      globalThis.fetch = answer as any;
      const btn = { disabled: false } as HTMLButtonElement;
      await checkForUpdates(btn);
      expect(btn.disabled).toBe(false);
    }
  });

  test('"Restart to finish updating" shows only while the server says a restart is needed', async () => {
    const restart = row('restartToFinishUpdate')!;
    const load = async (restartNeeded: boolean) => {
      globalThis.fetch = (async (url: string) => jsonResponse(url.includes('/api/updates/status') ? { status: null, restart_needed: restartNeeded } : { version: '0.3.10', instance_id: 'x', data_dir: '/d' })) as any;
      await loadSettingsServerInfo();
    };
    await load(false); // start from a known state: the module is shared with every other test file
    expect(settingRowVisible(restart)).toBe(false);
    await load(true);
    expect(settingRowVisible(restart)).toBe(true);
    expect(settingRowVisible(row('checkForUpdates')!)).toBe(true);
    await load(false); // the new copy is running
    expect(settingRowVisible(restart)).toBe(false);
  });

  test('a host notice keyed restart-needed pops up the restart question once; other notices do not', async () => {
    const { startNotificationFeed, setRestartPrompt } = await import('../src/ui/notification_center.ts');
    (globalThis as any).document = { hidden: false, getElementById: () => null };
    const note = (id: number, key: string | undefined, title: string) =>
      ({ id, ts_ms: Date.now(), source: 'host', source_name: 'OpenSanctuary', level: 'warning', title, body: 'b', sticky: true, key });
    const pages = [
      { notifications: [], latest: 0 },                                                                // history at page open
      { notifications: [note(1, undefined, 'Something else')], latest: 1 },
      { notifications: [note(2, 'restart-needed', 'Restart to finish updating')], latest: 2 },
    ];
    let call = 0;
    globalThis.fetch = (async () => {
      const body = pages[call++];
      return body ? { ok: true, status: 200, json: async () => body } : { ok: false, status: 401, json: async () => ({}) }; // 401 ends the feed
    }) as any;
    let prompts = 0;
    setRestartPrompt(() => { prompts++; });
    await startNotificationFeed();
    expect(prompts).toBe(1);
  }, 15000);
});
