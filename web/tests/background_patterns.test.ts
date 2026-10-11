import { describe, expect, test } from 'bun:test';
import {
    BACKGROUND_PATTERNS, getPatternSpec, patternKeyframes, patternAnimationName,
} from '../src/core/background_patterns.ts';
import { parseAnimatedPatternMarker } from '../src/core/presentation_helpers.ts';

describe('background patterns', () => {
    test('catalogue has the 5 originals + 10 new animated + 10 new still', () => {
        expect(BACKGROUND_PATTERNS.filter(p => p.animated).length).toBe(15);
        expect(BACKGROUND_PATTERNS.filter(p => !p.animated).length).toBe(10);
        expect(new Set(BACKGROUND_PATTERNS.map(p => p.key)).size).toBe(25);
        expect(new Set(BACKGROUND_PATTERNS.map(p => p.name)).size).toBe(25);
    });

    test('every pattern builds, has a base layer, and matches its animated flag', () => {
        for (const meta of BACKGROUND_PATTERNS) {
            const spec = getPatternSpec(meta.key)!;
            expect(spec.layers.length).toBeGreaterThan(0);
            expect(spec.layers.every(l => l.image && l.size && !l.image.includes('NaN') && !l.image.includes('undefined'))).toBe(true);
            expect(spec.durationSec > 0).toBe(meta.animated);
            expect(parseAnimatedPatternMarker('pattern:' + meta.key)).toBe(meta.key);
        }
    });

    test('looping tile drifts end on a whole number of tiles (seamless)', () => {
        for (const meta of BACKGROUND_PATTERNS.filter(p => p.animated)) {
            const spec = getPatternSpec(meta.key)!;
            if (spec.alternate) continue;
            for (const l of spec.layers) {
                if (!l.to || l.repeat === 'no-repeat') continue;
                const [tw, th] = l.size.split(' ').map(parseFloat);
                const [fx, fy] = l.from!.split(' ');
                const [tx, ty] = l.to.split(' ');
                const dx = parseFloat(tx) - parseFloat(fx);
                if (l.size.split(' ')[0].endsWith('px')) expect(Math.abs(dx) % tw).toBe(0);
                if (l.size.split(' ')[1].endsWith('px') && ty.endsWith('px')) expect(Math.abs(parseFloat(ty) - parseFloat(fy)) % th).toBe(0);
            }
        }
    });

    test('keyframes: one position per layer; none for still patterns', () => {
        for (const meta of BACKGROUND_PATTERNS) {
            const css = patternKeyframes(meta.key);
            if (!meta.animated) { expect(css).toBe(''); continue; }
            expect(css).toContain(`@keyframes ${patternAnimationName(meta.key)}`);
            const n = getPatternSpec(meta.key)!.layers.length;
            const from = css.match(/from \{ background-position: ([^;]+);/)![1];
            expect(from.split(', ').length).toBe(n);
        }
    });

    test('particle tiles are big and carry many dots (no small repeating wallpaper)', () => {
        const spec = getPatternSpec('starfield-drift')!;
        const sizes = spec.layers.filter(l => l.repeat === 'repeat').map(l => l.size);
        expect(new Set(sizes).size).toBe(sizes.length);          // no two layers share a tile size
        for (const l of spec.layers.filter(l => l.repeat === 'repeat')) {
            expect(parseFloat(l.size)).toBeGreaterThanOrEqual(1300);
            const svg = decodeURIComponent(l.image);
            expect((svg.match(/<circle/g) || []).length).toBeGreaterThanOrEqual(40);
        }
    });

    test('generation is deterministic', () => {
        const a = getPatternSpec('rising-embers')!.layers[0].image;
        const b = getPatternSpec('rising-embers')!.layers[0].image;
        expect(a).toBe(b);
    });
});
