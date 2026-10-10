import { describe, expect, test } from 'bun:test';
import { DEFAULT_ICON_SET, ICON_SETS, iconUrl, normalizeIconSet } from '../src/core/icon_sets.ts';
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

describe('icon sets', () => {
  test('unknown or missing ids fall back to the default', () => {
    expect(normalizeIconSet(undefined)).toBe(DEFAULT_ICON_SET);
    expect(normalizeIconSet('')).toBe(DEFAULT_ICON_SET);
    expect(normalizeIconSet('../etc')).toBe(DEFAULT_ICON_SET);
    expect(normalizeIconSet('crimson')).toBe('crimson');
  });

  test('iconUrl points into the set folder', () => {
    expect(iconUrl('slate', 'save')).toBe('icons/slate/save.svg');
    expect(iconUrl('nope', 'save')).toBe('icons/orange/save.svg');
  });

  test('every registered set exists on disk with the same icons', () => {
    const root = join(import.meta.dir, '..', 'icons');
    const reference = readdirSync(join(root, DEFAULT_ICON_SET)).filter(f => f.endsWith('.svg')).sort();
    expect(reference.length).toBeGreaterThan(0);
    for (const set of ICON_SETS) {
      expect(existsSync(join(root, set.id))).toBe(true);
      expect(readdirSync(join(root, set.id)).filter(f => f.endsWith('.svg')).sort()).toEqual(reference);
    }
  });
});
