import { describe, test, expect } from 'bun:test';
import { SETTINGS_SCHEMA } from '../src/core/settings_schema.ts';
import { checkForUpdates } from '../src/ui/settings_dialog.ts';

describe('Settings > About > Check for Updates', () => {
  test('SETTINGS_SCHEMA includes a checkForUpdates action row in the about category', () => {
    const def = SETTINGS_SCHEMA.find(s => s.key === 'checkForUpdates');
    expect(def).toBeDefined();
    expect(def?.category).toBe('about');
    expect(def?.control).toBe('action');
  });

  function withMockDocument<T>(fn: () => Promise<T>): Promise<T> {
    // renderSettingsContent()/showConfirmDialog() both guard on
    // `document.getElementById` existing -- a minimal stub returning null
    // for every id is enough for them to safely no-op, matching how
    // web/tests/network_settings.test.ts stubs `document` for the same reason.
    const original = (globalThis as any).document;
    (globalThis as any).document = { getElementById: () => null };
    return fn().finally(() => {
      (globalThis as any).document = original;
    });
  }

  function withMockFetch<T>(handler: (url: string, init: any) => any, fn: () => Promise<T>): Promise<T> {
    const original = globalThis.fetch;
    globalThis.fetch = (async (input: any, init?: any) => {
      const url = typeof input === 'string' ? input : input.url;
      return handler(url, init);
    }) as any;
    return fn().finally(() => {
      globalThis.fetch = original;
    });
  }

  test('reports "up to date" without throwing when no update is available', async () => {
    await withMockDocument(() => withMockFetch(
      (url) => {
        if (url === '/api/updates/check') {
          return {
            ok: true,
            headers: { get: () => 'application/json' },
            text: async () => JSON.stringify({
              status: { checked_at_ms: 1, current_version: '0.2.0', latest: null, error: null },
            }),
          };
        }
        throw new Error(`unexpected fetch: ${url}`);
      },
      async () => {
        const btn = { disabled: false } as HTMLButtonElement;
        await checkForUpdates(btn);
        // Button must be re-enabled after a successful check either way.
        expect(btn.disabled).toBe(false);
      },
    ));
  });

  test('reports an error toast rather than throwing when the check fails', async () => {
    await withMockDocument(() => withMockFetch(
      (url) => {
        if (url === '/api/updates/check') {
          return {
            ok: true,
            headers: { get: () => 'application/json' },
            text: async () => JSON.stringify({
              status: { checked_at_ms: 1, current_version: '0.2.0', latest: null, error: 'Failed to reach GitHub: timed out' },
            }),
          };
        }
        throw new Error(`unexpected fetch: ${url}`);
      },
      async () => {
        const btn = { disabled: false } as HTMLButtonElement;
        await checkForUpdates(btn);
        expect(btn.disabled).toBe(false);
      },
    ));
  });

  test('re-enables the button even when the request itself throws', async () => {
    await withMockDocument(() => withMockFetch(
      () => { throw new Error('network down'); },
      async () => {
        const btn = { disabled: false } as HTMLButtonElement;
        await checkForUpdates(btn);
        expect(btn.disabled).toBe(false);
      },
    ));
  });
});
