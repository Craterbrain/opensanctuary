import { describe, test, expect, beforeEach, afterEach } from 'bun:test';
import { SETTINGS_CATEGORIES, SETTINGS_SCHEMA } from '../src/core/settings_schema.ts';
import { api } from '../src/core/api_client.ts';
import { __resetHostSessionTokenForTests } from '../src/core/host_session.ts';

describe('Storage Settings Schema & Move Data Directory', () => {
  beforeEach(() => {
    // `moveDataDir` resolves a host session token before calling out (see
    // api_client.ts); this module's cache is otherwise shared with every
    // other test file in the same `bun test` process (see
    // host_session.ts's `__resetHostSessionTokenForTests` doc comment).
    __resetHostSessionTokenForTests();
    (globalThis as any).window = { location: { hostname: '192.168.1.50', pathname: '/', search: '', hash: '' } };
  });

  test('SETTINGS_CATEGORIES includes storage category', () => {
    const storageCat = SETTINGS_CATEGORIES.find(c => c.id === 'storage');
    expect(storageCat).toBeDefined();
    expect(storageCat?.label).toBe('Storage');
    expect(storageCat?.icon).toBe('📁');
  });

  test('SETTINGS_SCHEMA includes dataDirectory and moveDataDirectory action', () => {
    const dataDirDef = SETTINGS_SCHEMA.find(s => s.key === 'dataDirectory');
    expect(dataDirDef).toBeDefined();
    expect(dataDirDef?.category).toBe('storage');
    expect(dataDirDef?.control).toBe('readonly');

    const moveDef = SETTINGS_SCHEMA.find(s => s.key === 'moveDataDirectory');
    expect(moveDef).toBeDefined();
    expect(moveDef?.category).toBe('storage');
    expect(moveDef?.control).toBe('action');
    expect(moveDef?.actionLabel).toBe('Move Data Directory…');
  });

  test('api.system.moveDataDir dispatches POST to /api/system/move-data-dir with delete_source flag', async () => {
    const originalFetch = globalThis.fetch;
    let interceptedUrl = '';
    let interceptedMethod = '';
    let interceptedBody: any = null;

    globalThis.fetch = (async (url: string, init?: RequestInit) => {
      interceptedUrl = url;
      interceptedMethod = init?.method || 'GET';
      interceptedBody = init?.body ? JSON.parse(init.body as string) : null;
      return new Response(JSON.stringify({
        ok: true,
        source_dir: '/old/path',
        target_dir: '/new/path',
        files_copied: 5,
        sha_verified: true,
        source_deleted: true,
        restart_required: true,
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }) as any;

    try {
      const res = await api.system.moveDataDir('/new/path', true);
      expect(interceptedUrl).toBe('/api/system/move-data-dir');
      expect(interceptedMethod).toBe('POST');
      expect(interceptedBody).toEqual({ target_dir: '/new/path', delete_source: true });
      expect(res.ok).toBe(true);
      expect(res.files_copied).toBe(5);
      expect(res.sha_verified).toBe(true);
      expect(res.source_deleted).toBe(true);
      expect(res.restart_required).toBe(true);
    } finally {
      globalThis.fetch = originalFetch;
    }
  });
});
