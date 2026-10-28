import { describe, test, expect } from 'bun:test';
import { licensedLinkOf, openPublisherLink } from '../src/ui/licensed_link.ts';

describe('Licensed Bible publisher link', () => {
  test('only a licensed passage with an https link has one', () => {
    expect(licensedLinkOf({ licensed: { tag: 'NIV · © Biblica', link: 'https://www.biblica.com' } })).toEqual({ tag: 'NIV · © Biblica', link: 'https://www.biblica.com' });
    expect(licensedLinkOf({ licensed: { version: 'NASB', link: 'https://www.lockman.org' } })?.tag).toBe('NASB');
    expect(licensedLinkOf({ licensed: { tag: 'NIV', link: '' } })).toBeNull();
    expect(licensedLinkOf({ licensed: { tag: 'NIV', link: 'javascript:alert(1)' } })).toBeNull();
    expect(licensedLinkOf({ licensed: { tag: 'NIV', link: 'http://www.biblica.com' } })).toBeNull();
    expect(licensedLinkOf({ title: 'Amazing Grace' })).toBeNull();
    expect(licensedLinkOf(null)).toBeNull();
  });

  test('opening asks the host, and a refusal or failure is a plain false', async () => {
    const original = globalThis.fetch;
    try {
      let sent: any = null;
      globalThis.fetch = (async (_u: any, init: any) => { sent = JSON.parse(init.body); return { ok: true, json: async () => ({ ok: true }) }; }) as any;
      expect(await openPublisherLink('https://www.biblica.com')).toBe(true);
      expect(sent).toEqual({ url: 'https://www.biblica.com' });
      globalThis.fetch = (async () => ({ ok: true, json: async () => ({ ok: false, error: 'no' }) })) as any;
      expect(await openPublisherLink('https://www.biblica.com')).toBe(false);
      globalThis.fetch = (async () => { throw new Error('offline'); }) as any;
      expect(await openPublisherLink('https://www.biblica.com')).toBe(false);
    } finally {
      globalThis.fetch = original;
    }
  });
});
