import { describe, test, expect } from 'bun:test';
import { addHymnLibrary } from '../src/ui/public_domain_ui.ts';

function withFetch<T>(handler: (url: string, init: any) => any, fn: () => Promise<T>): Promise<T> {
  const original = globalThis.fetch;
  globalThis.fetch = (async (input: any, init?: any) => handler(typeof input === 'string' ? input : input.url, init)) as any;
  return fn().finally(() => { globalThis.fetch = original; });
}
const json = (body: any, status = 200) => ({ ok: status >= 200 && status < 300, status, json: async () => body, text: async () => JSON.stringify(body) });

describe('Public-domain hymns download', () => {
  test('reports what was added and what was left alone', async () => {
    await withFetch((u, init) => {
      expect(u).toBe('/api/songs/public-domain');
      expect(init.method).toBe('POST');
      return json({ success: true, total: 221, added: 200, already_present: 21 });
    }, async () => {
      const r = await addHymnLibrary();
      expect(r.ok).toBe(true);
      expect(r.added).toBe(200);
      expect(r.message).toBe('Added 200 hymns (21 you already had were left as they are).');
    });
    await withFetch(() => json({ success: true, total: 221, added: 0, already_present: 221 }), async () => {
      expect((await addHymnLibrary()).message).toBe('Nothing to add: all 221 hymns are already in your library.');
    });
  });

  test('a failure (no internet, refused download, no token) is a message, not an exception', async () => {
    await withFetch(() => ({ ok: false, status: 502, text: async () => "couldn't reach the hymn library (offline)" }), async () => {
      const r = await addHymnLibrary();
      expect(r.ok).toBe(false);
      expect(r.message).toBe("Could not download the hymns: couldn't reach the hymn library (offline)");
    });
    await withFetch(() => ({ ok: false, status: 401, text: async () => '' }), async () => {
      expect((await addHymnLibrary()).message).toContain('401');
    });
    await withFetch(() => { throw new Error('network down'); }, async () => {
      expect((await addHymnLibrary()).message).toContain('network down');
    });
  });
});
