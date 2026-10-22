import { describe, expect, test } from 'bun:test';
import { readFileSync } from 'fs';
import { resolve } from 'path';
import { BACKGROUND_PATTERNS, patternLoopSec, patternRevision } from '../src/core/background_patterns.ts';
import { patternThumbUrl } from '../src/core/pattern_thumbs.ts';

const DIR = resolve(import.meta.dir, '../pattern-thumbs');

describe('pre-rendered pattern previews', () => {
    // Fails when a pattern is added or changed without its preview being rendered again (the
    // pickers would quietly fall back to drawing it live). Fix: cd web && bun scripts/render_pattern_thumbs.ts
    test('every animated pattern has an up-to-date preview', () => {
        const animated = BACKGROUND_PATTERNS.filter(p => p.animated && patternLoopSec(p.key) > 0);
        expect(animated.length).toBeGreaterThan(0);
        const stale = animated.filter(p => patternThumbUrl(p.key) === null).map(p => `${p.key} (now ${patternRevision(p.key)})`);
        expect(stale).toEqual([]);
    });

    test('each preview is an animated WebP that loops forever', () => {
        for (const p of BACKGROUND_PATTERNS.filter(p => p.animated && patternLoopSec(p.key) > 0)) {
            const bytes = readFileSync(resolve(DIR, `${p.key}.webp`));
            expect(bytes.subarray(0, 4).toString('latin1')).toBe('RIFF');
            expect(bytes.subarray(8, 12).toString('latin1')).toBe('WEBP');
            const anim = bytes.indexOf('ANIM', 12, 'latin1');
            expect(anim).toBeGreaterThan(0);
            expect(bytes.readUInt16LE(anim + 8 + 4)).toBe(0); // loop count 0 = forever
            expect(bytes.length).toBeLessThan(600_000);      // a picker loads all of them at once
        }
    });

    test('a pattern that is not animated, or unknown, has no preview', () => {
        const still = BACKGROUND_PATTERNS.find(p => !p.animated)!;
        expect(patternThumbUrl(still.key)).toBeNull();
        expect(patternThumbUrl('no-such-pattern')).toBeNull();
    });
});
