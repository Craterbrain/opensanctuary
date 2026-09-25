import { describe, test, expect, beforeAll } from 'bun:test';
import { autoFitLyrics } from '../src/core/autofit';

beforeAll(() => {
  if (typeof (globalThis as any).window === 'undefined') {
    (globalThis as any).window = globalThis;
  }
});

function createMockElement(props: {
  clientWidth?: number;
  clientHeight?: number;
  textContent?: string;
  scrollWidthFactor?: number;
  scrollHeightFactor?: number;
} = {}) {
  const style: Record<string, string> = {};
  return {
    clientWidth: props.clientWidth ?? 960,
    clientHeight: props.clientHeight ?? 540,
    textContent: props.textContent ?? '',
    querySelector: () => null,
    style,
    get scrollWidth() {
      const fs = parseFloat(style.fontSize) || 18;
      return (props.scrollWidthFactor ?? 8) * fs;
    },
    get scrollHeight() {
      const fs = parseFloat(style.fontSize) || 18;
      return (props.scrollHeightFactor ?? 2) * fs;
    }
  } as any;
}

describe('Canonical autoFitLyrics Shared Autofit Engine', () => {
  test('returns fallback font size when elements are missing or empty', () => {
    expect(autoFitLyrics(null, null)).toBe(18);

    const container = createMockElement();
    const target = createMockElement({ textContent: '' });
    expect(autoFitLyrics(container, target)).toBe(18);
  });

  test('converges within min and max font size bounds', () => {
    const container = createMockElement({ clientWidth: 960, clientHeight: 540 });
    const target = createMockElement({
      textContent: 'Bless the Lord O My Soul',
      scrollWidthFactor: 10,
      scrollHeightFactor: 2
    });

    const fitted = autoFitLyrics(container, target, { minFontSize: 20, maxFontSize: 80 });
    expect(fitted).toBeGreaterThanOrEqual(20);
    expect(fitted).toBeLessThanOrEqual(80);
    expect(target.style.fontSize).toContain('px');
  });

  test('yields larger font size for short text than long text', () => {
    const container = createMockElement({ clientWidth: 960, clientHeight: 540 });

    // Short text target (low width and height factors)
    const targetShort = createMockElement({
      textContent: 'Amen',
      scrollWidthFactor: 3,
      scrollHeightFactor: 1.2
    });

    // Long text target (high width and height factors)
    const targetLong = createMockElement({
      textContent: 'The Lord is my shepherd I shall not want He maketh me to lie down in green pastures He leadeth me beside still waters',
      scrollWidthFactor: 22,
      scrollHeightFactor: 8
    });

    const fittedShort = autoFitLyrics(container, targetShort, { minFontSize: 16, maxFontSize: 100 });
    const fittedLong = autoFitLyrics(container, targetLong, { minFontSize: 16, maxFontSize: 100 });

    expect(fittedShort).toBeGreaterThan(fittedLong);
  });

  test('exposes autoFitLyrics globally on window for live.html compatibility', () => {
    expect((globalThis as any).autoFitLyrics).toBeDefined();
    expect(typeof (globalThis as any).autoFitLyrics).toBe('function');
  });
});
