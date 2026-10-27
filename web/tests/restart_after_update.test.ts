import { describe, test, expect } from 'bun:test';
import { SETTINGS_SCHEMA } from '../src/core/settings_schema.ts';
import { loadSettingsServerInfo, settingRowVisible } from '../src/ui/settings_dialog.ts';

// Linux: after the package manager replaces the program, Settings > About offers "Restart now" (docs/update.md).
describe('Settings > About > Restart to finish updating', () => {
  const def = SETTINGS_SCHEMA.find(s => s.key === 'restartToFinishUpdate')!;

  function withMockFetch<T>(restartNeeded: boolean, fn: () => Promise<T>): Promise<T> {
    const original = globalThis.fetch;
    const json = (body: any) => ({ ok: true, headers: { get: () => 'application/json' }, text: async () => JSON.stringify(body) });
    globalThis.fetch = (async (input: any) => {
      const url = typeof input === 'string' ? input : input.url;
      if (url.includes('/api/updates/status')) return json({ status: null, restart_needed: restartNeeded });
      return json({ version: '0.3.10', instance_id: 'x', data_dir: '/d' });
    }) as any;
    return fn().finally(() => { globalThis.fetch = original; });
  }

  test('the row is in the About category and is an action', () => {
    expect(def).toBeDefined();
    expect(def.category).toBe('about');
    expect(def.control).toBe('action');
  });

  test('the row is hidden until the server says a restart is needed', async () => {
    expect(settingRowVisible(def)).toBe(false);
    await withMockFetch(false, async () => { await loadSettingsServerInfo(); });
    expect(settingRowVisible(def)).toBe(false);
    await withMockFetch(true, async () => { await loadSettingsServerInfo(); });
    expect(settingRowVisible(def)).toBe(true);
    // Other rows are never affected.
    expect(settingRowVisible(SETTINGS_SCHEMA.find(s => s.key === 'checkForUpdates')!)).toBe(true);
    // A later load that says "no longer needed" (the new copy is running) hides it again.
    await withMockFetch(false, async () => { await loadSettingsServerInfo(); });
    expect(settingRowVisible(def)).toBe(false);
  });
});

// The host's "restart-needed" notice pops up the question (updates are started by the operator, so asking is not a surprise).
describe('Restart pop-up from the notification feed', () => {
  test('a host notice keyed restart-needed calls the prompt once; an ordinary notice does not', async () => {
    const { startNotificationFeed, setRestartPrompt } = await import('../src/ui/notification_center.ts');
    const originalFetch = globalThis.fetch;
    const originalDoc = (globalThis as any).document;
    (globalThis as any).document = { hidden: false, getElementById: () => null };
    const note = (id: number, key: string | undefined, title: string) =>
      ({ id, ts_ms: Date.now(), source: 'host', source_name: 'OpenSanctuary', level: 'warning', title, body: 'b', sticky: true, key });
    let call = 0;
    globalThis.fetch = (async () => {
      call++;
      const json = (body: any, status = 200) => ({ ok: status === 200, status, json: async () => body });
      if (call === 1) return json({ notifications: [], latest: 0 });                                              // history at page open
      if (call === 2) return json({ notifications: [note(1, undefined, 'Something else')], latest: 1 });           // ordinary notice
      if (call === 3) return json({ notifications: [note(2, 'restart-needed', 'Restart to finish updating')], latest: 2 });
      return json({}, 401);                                                                                       // ends the feed
    }) as any;
    let prompts = 0;
    setRestartPrompt(() => { prompts++; });
    try {
      await startNotificationFeed();
      expect(prompts).toBe(1);
    } finally {
      globalThis.fetch = originalFetch;
      (globalThis as any).document = originalDoc;
    }
  }, 15000);
});
