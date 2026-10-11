import { describe, expect, test } from 'bun:test';
import {
    BACKGROUND_PATTERNS, getPatternSpec, patternFrameStyle, patternLoopSec, patternPrerenderable,
    patternRevision, PRERENDER_MAX_LOOP_SEC, cubicBezier,
} from '../src/core/background_patterns.ts';
import { parseGradient, parseColor } from '../src/core/gradient_canvas.ts';
import { resolutionFor } from '../src/core/pattern_render.ts';

describe('loop length and eligibility', () => {
    test('still patterns have no loop; ping-pong patterns loop at twice one leg', () => {
        expect(patternLoopSec('horizon-glow')).toBe(0);
        expect(patternLoopSec('liquid-gradient')).toBe(64);
        expect(patternLoopSec('soft-rain')).toBe(4.5);
        expect(patternLoopSec('sprouting-leaves')).toBe(28);
        expect(patternPrerenderable('horizon-glow')).toBe(false);
        expect(patternPrerenderable('starfield-drift')).toBe(false);      // 420s: cheap to draw live
        expect(patternPrerenderable('sprouting-leaves')).toBe(true);
        for (const p of BACKGROUND_PATTERNS) {
            if (patternPrerenderable(p.key)) expect(patternLoopSec(p.key)).toBeLessThanOrEqual(PRERENDER_MAX_LOOP_SEC);
        }
    });

    test('revision is stable and changes with the definition', () => {
        expect(patternRevision('soft-rain')).toBe(patternRevision('soft-rain'));
        expect(patternRevision('soft-rain')).not.toBe(patternRevision('rising-embers'));
        expect(patternRevision('soft-rain')).toMatch(/^[0-9a-f]{8}$/);
    });
});

describe('frame styles', () => {
    test('loop frames are seamless: position at t=0 equals position at t=loop', () => {
        for (const p of BACKGROUND_PATTERNS.filter(p => patternPrerenderable(p.key))) {
            const loop = patternLoopSec(p.key);
            const a = patternFrameStyle(p.key, 0, 1)!, b = patternFrameStyle(p.key, loop, 1)!;
            expect(b.positions).toEqual(a.positions);
            expect(b.sizes).toEqual(a.sizes);
        }
    });

    test('drifting tiles move between frames; ping-pong turns around at the half-way point', () => {
        const a = patternFrameStyle('soft-rain', 0, 1)!, b = patternFrameStyle('soft-rain', 1, 1)!;
        expect(b.positions).not.toEqual(a.positions);
        const leg = 32; // liquid-gradient leg length
        const mid = patternFrameStyle('liquid-gradient', leg, 1)!.positions;
        const before = patternFrameStyle('liquid-gradient', leg - 4, 1)!.positions;
        const after = patternFrameStyle('liquid-gradient', leg + 4, 1)!.positions;
        expect(before).toEqual(after);
        expect(mid).not.toEqual(patternFrameStyle('liquid-gradient', 0, 1)!.positions);
    });

    test('pixel offsets scale with the frame size', () => {
        const full = patternFrameStyle('soft-rain', 2, 1)!, big = patternFrameStyle('soft-rain', 2, 4 / 3)!;
        expect(parseFloat(big.sizes[0])).toBeCloseTo(parseFloat(full.sizes[0]) * 4 / 3, 0);
    });

    test('plant frames: empty at the loop start/end, grown and then drifting in between', () => {
        const spec = getPatternSpec('sprouting-leaves')!;
        const frame = spec.layers[0].frameImage!;
        const count = (t: number): number => (decodeURIComponent(frame(t)).match(/<g transform=/g) || []).length;
        expect(count(0)).toBe(0);
        expect(count(0.5)).toBeGreaterThan(40);
        expect(count(0.9)).toBeLessThan(count(0.5));
        expect(count(1)).toBe(0);
        const strokes = (t: number): number => (decodeURIComponent(frame(t)).match(/<path d='M\d/g) || []).length;
        expect(strokes(0.3)).toBeGreaterThan(0);
    });
});

describe('gradient parsing (the grammar the patterns use)', () => {
    test('colours', () => {
        expect(parseColor('#f00')).toEqual([255, 0, 0, 1]);
        expect(parseColor('rgba(10,20,30,.5)')).toEqual([10, 20, 30, 0.5]);
        expect(parseColor('transparent')[3]).toBe(0);
    });

    test('every gradient layer in every pattern parses', () => {
        for (const p of BACKGROUND_PATTERNS) {
            for (const l of getPatternSpec(p.key)!.layers) {
                if (l.image.startsWith('url(')) continue;
                const g = parseGradient(l.image);
                expect(g, `${p.key}: ${l.image}`).not.toBeNull();
                expect(g!.stops.length).toBeGreaterThan(1);
            }
        }
    });

    test('radial/conic/linear details', () => {
        const r = parseGradient('radial-gradient(ellipse 40% 30% at 20% 80%, rgba(1,2,3,.5), transparent 100%)')!;
        expect([r.kind, r.rx, r.ry, r.cx, r.cy]).toEqual(['radial', 40, 30, 20, 80]);
        const c = parseGradient('conic-gradient(from 10deg at 50% -12%, transparent 0deg, rgba(1,1,1,.2) 40deg, transparent 360deg)')!;
        expect([c.kind, c.angleDeg, c.cy, c.stopsInDeg]).toEqual(['conic', 10, -12, true]);
        expect(parseGradient('linear-gradient(135deg, #000, #fff)')!.angleDeg).toBe(135);
    });

    test('cubic-bezier hits its endpoints and is monotonic', () => {
        expect(cubicBezier(0.42, 0, 0.58, 1, 0)).toBe(0);
        expect(cubicBezier(0.42, 0, 0.58, 1, 1)).toBe(1);
        expect(cubicBezier(0.42, 0, 0.58, 1, 0.5)).toBeCloseTo(0.5, 2);
        expect(cubicBezier(0.42, 0, 0.58, 1, 0.25)).toBeLessThan(0.25);
    });
});

describe('resolution choice', () => {
    test('1440p by default; 1080p when asked or when a monitor is smaller', () => {
        expect(resolutionFor('auto', [])).toEqual({ width: 2560, height: 1440 });
        expect(resolutionFor('auto', [1440, 2160])).toEqual({ width: 2560, height: 1440 });
        expect(resolutionFor('auto', [1440, 1080])).toEqual({ width: 1920, height: 1080 });
        expect(resolutionFor('1080', [2160])).toEqual({ width: 1920, height: 1080 });
        expect(resolutionFor('1440', [720])).toEqual({ width: 2560, height: 1440 });
    });
});
