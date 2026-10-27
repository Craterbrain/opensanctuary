import { describe, test, expect } from 'bun:test';
import { addHymnLibrary, fetchHymnLibraryState, hymnStateText } from '../src/ui/public_domain_ui.ts';

function withFetch<T>(handler: (url: string, init: any) => any, fn: () => Promise<T>): Promise<T> {
  const original = globalThis.fetch;
  globalThis.fetch = (async (input: any, init?: any) => handler(typeof input === 'string' ? input : input.url, init)) as any;
  return fn().finally(() => { globalThis.fetch = original; });
}
const json = (body: any, status = 200) => ({ ok: status >= 200 && status < 300, status, json: async () => body });

describe('Public-domain hymns card', () => {
  test('says what the library has, in plain words', () => {
    expect(hymnStateText(null)).toBe('');
    expect(hymnStateText({ total: 221, present: 0, missing: 221 })).toBe('221 hymns, none in your library yet.');
    expect(hymnStateText({ total: 221, present: 200, missing: 21 })).toBe('21 of 221 hymns are not in your library yet.');
    expect(hymnStateText({ total: 221, present: 221, missing: 0 })).toBe('All 221 hymns are already in your song library.');
  });

  test('reads the state from the server, and gives null when it cannot', async () => {
    await withFetch(() => json({ total: 221, present: 5, missing: 216 }), async () => {
      expect(await fetchHymnLibraryState()).toEqual({ total: 221, present: 5, missing: 216 });
    });
    await withFetch(() => json({}, 500), async () => { expect(await fetchHymnLibraryState()).toBeNull(); });
    await withFetch(() => { throw new Error('offline'); }, async () => { expect(await fetchHymnLibraryState()).toBeNull(); });
  });

  test('adding reports what was added and what was left alone, and a failure is a message, not an exception', async () => {
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
    await withFetch(() => json({}, 401), async () => {
      const r = await addHymnLibrary();
      expect(r.ok).toBe(false);
      expect(r.message).toContain('401');
    });
  });
});
