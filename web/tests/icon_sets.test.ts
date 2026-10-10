import { describe, expect, test } from 'bun:test';
import {
  DEFAULT_ICON_SET, ICON_SETS, getThemeColors, hexToRgbTriplet, iconUrl, normalizeIconSet, resolveTheme,
} from '../src/core/icon_sets.ts';
import { SETTINGS_CATEGORIES, SETTINGS_SCHEMA } from '../src/core/settings_schema.ts';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
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

describe('color themes', () => {
  test('linked is the default: the color theme follows the icon set', () => {
    expect(resolveTheme({})).toEqual({ iconSet: 'orange', colorTheme: 'orange', linked: true });
    expect(resolveTheme({ iconSet: 'slate', colorTheme: 'crimson' })).toEqual({ iconSet: 'slate', colorTheme: 'slate', linked: true });
    expect(resolveTheme({ iconSet: 'slate', colorTheme: 'crimson', themeLinked: 'true' }).colorTheme).toBe('slate');
  });

  test('unlinked: icon set and color theme are independent', () => {
    expect(resolveTheme({ iconSet: 'slate', colorTheme: 'crimson', themeLinked: 'false' }))
      .toEqual({ iconSet: 'slate', colorTheme: 'crimson', linked: false });
  });

  test('unlinked with no or a bad color theme falls back safely', () => {
    expect(resolveTheme({ iconSet: 'gold', themeLinked: 'false' }).colorTheme).toBe('gold');
    expect(resolveTheme({ iconSet: 'gold', colorTheme: 'bogus', themeLinked: 'false' }).colorTheme).toBe('orange');
  });

  test('the orange theme matches the stylesheet defaults', () => {
    const css = readFileSync(join(import.meta.dir, '..', 'style.css'), 'utf8');
    const c = getThemeColors('orange');
    expect(css).toContain(`--os-brand-primary: ${c.primary};`);
    expect(css).toContain(`--os-brand-amber: ${c.highlight};`);
    expect(css).toContain(`--os-brand-flame: ${c.deep};`);
    expect(css).toContain(`--os-brand-bright: ${c.bright};`);
    expect(css).toContain(`--os-brand-primary-rgb: ${hexToRgbTriplet(c.primary)};`);
    expect(css).toContain(`--os-brand-amber-rgb: ${hexToRgbTriplet(c.highlight)};`);
  });

  test('every theme has valid colors and white text stays readable on its primary', () => {
    const lum = (hex: string) => {
      const [r, g, b] = hexToRgbTriplet(hex).split(', ').map(n => {
        const v = Number(n) / 255;
        return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
      });
      return 0.2126 * r + 0.7152 * g + 0.0722 * b;
    };
    for (const set of ICON_SETS) {
      for (const hex of Object.values(set.colors)) expect(() => hexToRgbTriplet(hex)).not.toThrow();
      expect(1.05 / (lum(set.colors.primary) + 0.05)).toBeGreaterThanOrEqual(3);
    }
  });

  test('hexToRgbTriplet rejects non-#rrggbb input', () => {
    expect(hexToRgbTriplet('#FF5722')).toBe('255, 87, 34');
    expect(() => hexToRgbTriplet('red')).toThrow();
  });
});

describe('settings search covers the bespoke panels', () => {
  test('theme and display each have searchable panel rows', () => {
    for (const cat of ['theme', 'display']) {
      expect(SETTINGS_CATEGORIES.some(c => c.id === cat)).toBe(true);
      const rows = SETTINGS_SCHEMA.filter(d => d.category === cat);
      expect(rows.length).toBeGreaterThan(0);
      expect(rows.every(d => d.control === 'panel')).toBe(true);
    }
  });
});
