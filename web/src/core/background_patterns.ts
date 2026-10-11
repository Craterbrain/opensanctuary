/**
 * Procedural slide backgrounds ("pattern:<key>" markers).
 *
 * A plain CSS background value can't express motion, so each pattern is a
 * small spec -- a stack of background layers plus an optional animation -- and
 * `applyResolvedBackground` (presentation_helpers.ts) turns it into inline
 * styles. The matching @keyframes are generated here and injected once per
 * document (`ensurePatternKeyframes`), so there is a single source of truth
 * for both the in-app pages and live.html (a separate document).
 *
 * Why the old "one dot per tile" fields looked like wallpaper: a tiny tile
 * with one gradient dot repeats every few hundred pixels, and several of them
 * at unrelated tile sizes beat against each other into a moire grid. Particle
 * fields here are instead SVG tiles that are large (>= a screen) and hold many
 * dots placed by seeded best-candidate sampling on a torus -- evenly spread
 * (no clumps, no rows) and wrap-safe across the tile edge. Layers use
 * different tile sizes so the repeats of one never line up with another.
 *
 * Looping drifts shift each layer by a whole number of its own tile sizes, so
 * the last frame is pixel-identical to the first (no visible jump). Soft
 * gradient layers instead ping-pong (`alternate`) with easing.
 */

export interface PatternLayer {
    /** CSS <image>: a gradient or url("data:image/svg+xml,...") */
    image: string;
    size: string;
    repeat: 'repeat' | 'no-repeat' | 'repeat-x';
    /** Initial background-position. Default '0% 0%'. */
    from?: string;
    /** Animated end position; omit for a layer that doesn't move. */
    to?: string;
    /** Animated end size, for pulsing layers. */
    sizeTo?: string;
}

export interface BackgroundPatternSpec {
    layers: PatternLayer[];
    /** Length of one leg (or whole loop) in seconds; 0 = static. */
    durationSec: number;
    timing: 'linear' | 'ease-in-out';
    /** Ping-pong instead of restarting. */
    alternate: boolean;
}

export interface BackgroundPatternMeta {
    key: string;
    name: string;
    /** Static CSS swatch for contexts that can't run the real pattern. */
    preview: string;
    animated: boolean;
    build: () => BackgroundPatternSpec;
}

// ---------------------------------------------------------------------------
// Generators
// ---------------------------------------------------------------------------

function seeded(seed: number): () => number {
    let a = seed >>> 0;
    return () => {
        a = (a + 0x6d2b79f5) >>> 0;
        let t = a;
        t = Math.imul(t ^ (t >>> 15), t | 1);
        t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
}

/** n evenly spread points in a w*h torus (Mitchell's best-candidate). */
function scatter(w: number, h: number, n: number, rand: () => number, tries = 18): Array<[number, number]> {
    const pts: Array<[number, number]> = [];
    for (let i = 0; i < n; i++) {
        let best: [number, number] = [rand() * w, rand() * h];
        let bestD = -1;
        for (let t = 0; t < tries; t++) {
            const c: [number, number] = [rand() * w, rand() * h];
            let d = Infinity;
            for (const p of pts) {
                const dx = Math.min(Math.abs(c[0] - p[0]), w - Math.abs(c[0] - p[0]));
                const dy = Math.min(Math.abs(c[1] - p[1]), h - Math.abs(c[1] - p[1]));
                d = Math.min(d, dx * dx + dy * dy);
            }
            if (d > bestD) { bestD = d; best = c; }
        }
        pts.push(best);
    }
    return pts;
}

const r1 = (n: number): number => Math.round(n * 10) / 10;

function svgUrl(w: number, h: number, defs: string, body: string, extraAttrs = ''): string {
    const svg =
        `<svg xmlns='http://www.w3.org/2000/svg' width='${w}' height='${h}' viewBox='0 0 ${w} ${h}'${extraAttrs}>` +
        (defs ? `<defs>${defs}</defs>` : '') + body + '</svg>';
    return `url("data:image/svg+xml,${encodeURIComponent(svg)}")`;
}

/** [r,g,b] -> radial gradient def. `solid` = fraction kept opaque before the fade. */
function radialDef(id: string, rgb: string, solid: number, edge = 0): string {
    return `<radialGradient id='${id}'>` +
        `<stop offset='0' stop-color='rgb(${rgb})' stop-opacity='1'/>` +
        `<stop offset='${solid}' stop-color='rgb(${rgb})' stop-opacity='${solid > 0.5 ? 0.9 : 0.4}'/>` +
        `<stop offset='1' stop-color='rgb(${rgb})' stop-opacity='${edge}'/></radialGradient>`;
}

/** Emits the circle at (x,y) plus any copies that wrap over the tile edge. */
function wrapCircle(w: number, h: number, x: number, y: number, r: number, fill: string, opacity: number): string {
    let s = '';
    for (const dx of [-w, 0, w]) {
        for (const dy of [-h, 0, h]) {
            const px = x + dx, py = y + dy;
            if (px + r < 0 || px - r > w || py + r < 0 || py - r > h) continue;
            s += `<circle cx='${r1(px)}' cy='${r1(py)}' r='${r1(r)}' fill='${fill}' opacity='${r1(opacity * 100) / 100}'/>`;
        }
    }
    return s;
}

interface FieldOpts {
    w: number; h: number; n: number; seed: number;
    rMin: number; rMax: number; opMin: number; opMax: number;
    /** One or more "r,g,b" colours, picked per dot. */
    colors: string[];
    /** Opaque core fraction: ~0.15 = soft glow, ~0.7 = flat flake. */
    solid: number;
    /** Extra bright pinpoint at the centre (stars, fireflies). */
    core?: string;
    /** Bokeh: dim middle, brighter rim. */
    ring?: boolean;
}

/** A large non-repeating-looking tile of soft dots. */
function dotField(o: FieldOpts): string {
    const rand = seeded(o.seed);
    const pts = scatter(o.w, o.h, o.n, rand);
    const defs = o.colors.map((c, i) => o.ring
        ? `<radialGradient id='c${i}'><stop offset='0' stop-color='rgb(${c})' stop-opacity='.55'/>` +
          `<stop offset='.7' stop-color='rgb(${c})' stop-opacity='.7'/>` +
          `<stop offset='.9' stop-color='rgb(${c})' stop-opacity='.85'/>` +
          `<stop offset='1' stop-color='rgb(${c})' stop-opacity='0'/></radialGradient>`
        : radialDef(`c${i}`, c, o.solid)).join('');
    let body = '';
    for (const [x, y] of pts) {
        const r = o.rMin + (o.rMax - o.rMin) * Math.pow(rand(), 1.6);
        const op = o.opMin + (o.opMax - o.opMin) * rand();
        const ci = Math.floor(rand() * o.colors.length);
        body += wrapCircle(o.w, o.h, x, y, r, `url(#c${ci})`, op);
        if (o.core) body += wrapCircle(o.w, o.h, x, y, Math.max(0.7, r * 0.22), o.core, Math.min(1, op + 0.15));
    }
    return svgUrl(o.w, o.h, defs, body);
}

/** Rain streaks, slightly slanted. */
function streakField(w: number, h: number, n: number, seed: number, lenMin: number, lenMax: number, width: number, rgb: string, opMin: number, opMax: number): string {
    const rand = seeded(seed);
    const pts = scatter(w, h, n, rand);
    let body = '';
    for (const [x, y] of pts) {
        const len = lenMin + (lenMax - lenMin) * rand();
        const op = opMin + (opMax - opMin) * rand();
        const x2 = x - len * 0.18, y2 = y + len;
        // wrapped copies (vertical wrap matters most; horizontal for the slant)
        for (const dx of [-w, 0, w]) {
            for (const dy of [-h, 0, h]) {
                const ax = x + dx, ay = y + dy, bx = x2 + dx, by = y2 + dy;
                if (Math.max(ax, bx) < 0 || Math.min(ax, bx) > w || Math.max(ay, by) < 0 || Math.min(ay, by) > h) continue;
                body += `<line x1='${r1(ax)}' y1='${r1(ay)}' x2='${r1(bx)}' y2='${r1(by)}' stroke='rgba(${rgb},${r1(op * 100) / 100})' stroke-width='${width}' stroke-linecap='round'/>`;
            }
        }
    }
    return svgUrl(w, h, '', body);
}

/** A horizontally seamless wave band; `cycles` whole periods across the tile. */
function waveBand(w: number, h: number, cycles: Array<[number, number, number]>, fill: string): string {
    const steps = 96;
    let d = `M0,${h} `;
    for (let i = 0; i <= steps; i++) {
        const x = (i / steps) * w;
        let y = h * 0.5;
        for (const [n, amp, phase] of cycles) y += Math.sin((i / steps) * Math.PI * 2 * n + phase) * amp * h;
        d += `L${r1(x)},${r1(y)} `;
    }
    d += `L${w},${h} Z`;
    return svgUrl(w, h, '', `<path d='${d}' fill='${fill}'/>`, " preserveAspectRatio='none'");
}

/** Film-grain style noise, tileable (stitchTiles). */
function grain(size: number, alpha: number, rgb = '255,235,210'): string {
    const [r, g, b] = rgb.split(',').map(v => r1(Number(v) / 255));
    return svgUrl(size, size,
        `<filter id='n' x='0' y='0' width='100%' height='100%'><feTurbulence type='fractalNoise' baseFrequency='.85' numOctaves='2' seed='7' stitchTiles='stitch'/>` +
        `<feColorMatrix type='matrix' values='0 0 0 0 ${r} 0 0 0 0 ${g} 0 0 0 0 ${b} ${alpha} 0 0 0 0'/></filter>`,
        `<rect width='100%' height='100%' filter='url(#n)'/>`);
}

/** Irregular fan of light shafts as a conic gradient (hand-picked, never regular). */
function rayFan(seed: number, from: number, to: number, count: number, rgb: string, aMin: number, aMax: number): string {
    const rand = seeded(seed);
    const stops: string[] = ['transparent 0deg'];
    let angle = from;
    const span = (to - from) / count;
    for (let i = 0; i < count; i++) {
        const center = angle + span * (0.25 + rand() * 0.5);
        const half = span * (0.08 + rand() * 0.2);
        const a = aMin + (aMax - aMin) * rand();
        stops.push(`transparent ${r1(center - half * 1.8)}deg`, `rgba(${rgb},${r1(a * 100) / 100}) ${r1(center)}deg`, `transparent ${r1(center + half * 1.8)}deg`);
        angle += span;
    }
    stops.push('transparent 360deg');
    return `conic-gradient(from 0deg at 50% -12%, ${stops.join(', ')})`;
}

// ---- layer constructors ----------------------------------------------------

/** Seamless drifting tile: shift is a whole number of tiles. */
function tile(image: string, w: number, h: number, tilesX: number, tilesY: number): PatternLayer {
    return { image, size: `${w}px ${h}px`, repeat: 'repeat', from: '0px 0px', to: `${tilesX * w}px ${tilesY * h}px` };
}
/** Static layer (full-bleed or tiled). */
function fixed(image: string, size = '100% 100%', repeat: PatternLayer['repeat'] = 'no-repeat'): PatternLayer {
    return { image, size, repeat, from: '0% 0%' };
}
/** Soft layer that glides between two positions (needs alternate). */
function glide(image: string, size: string, from: string, to: string): PatternLayer {
    return { image, size, repeat: 'no-repeat', from, to };
}

const blob = (x: number, y: number, rgb: string, a: number, rx = 55, ry = 55): string =>
    `radial-gradient(ellipse ${rx}% ${ry}% at ${x}% ${y}%, rgba(${rgb},${a}), transparent 100%)`;

/** A flat colour as an <image> (a bare colour isn't valid in background-image). */
const flat = (c: string): string => `linear-gradient(${c}, ${c})`;

const drift = (layers: PatternLayer[], durationSec: number): BackgroundPatternSpec =>
    ({ layers, durationSec, timing: 'linear', alternate: false });
const sway = (layers: PatternLayer[], durationSec: number): BackgroundPatternSpec =>
    ({ layers, durationSec, timing: 'ease-in-out', alternate: true });
const still = (layers: PatternLayer[]): BackgroundPatternSpec =>
    ({ layers, durationSec: 0, timing: 'linear', alternate: false });

// ---------------------------------------------------------------------------
// Pattern catalogue
// ---------------------------------------------------------------------------

export const BACKGROUND_PATTERNS: BackgroundPatternMeta[] = [
    // ---- animated: the original five, rebuilt --------------------------------
    {
        key: 'floating-dust', name: 'Floating Dust', animated: true,
        preview: 'linear-gradient(160deg, #2b2013, #4a3820, #6b5227)',
        build: () => drift([
            tile(dotField({ w: 1500, h: 1100, n: 38, seed: 11, rMin: 4, rMax: 11, opMin: .25, opMax: .55, colors: ['255,226,160', '255,240,205'], solid: .12 }), 1500, 1100, 1, -1),
            tile(dotField({ w: 1100, h: 1100, n: 70, seed: 12, rMin: 2, rMax: 5, opMin: .45, opMax: .9, colors: ['255,235,180', '255,246,215'], solid: .15, core: '#fff6dc' }), 1100, 1100, 1, -1),
            fixed('radial-gradient(ellipse 70% 60% at 50% 100%, rgba(255,190,90,.28), transparent 100%)'),
            fixed('linear-gradient(160deg, #2b2013, #4a3820, #6b5227)'),
        ], 90)
    },
    {
        key: 'drifting-clouds', name: 'Drifting Clouds', animated: true,
        preview: 'linear-gradient(180deg, #3a5a8a, #6683b0, #9db4d6)',
        build: () => {
            const cloud = (w: number, h: number, n: number, seed: number, opMax: number): string => {
                const rand = seeded(seed);
                const pts = scatter(w, h, n, rand, 24);
                let body = '';
                for (const [x, y] of pts) {
                    const rx = 160 + rand() * 300, ry = 40 + rand() * 90;
                    // a cloud is a few overlapping puffs
                    for (let k = 0; k < 4; k++) {
                        const px = x + (k - 1.5) * rx * 0.45, py = y + (rand() - 0.5) * ry * 0.5;
                        for (const dx of [-w, 0, w]) {
                            body += `<ellipse cx='${r1(px + dx)}' cy='${r1(py)}' rx='${r1(rx * (0.55 + rand() * 0.3))}' ry='${r1(ry * (0.8 + rand() * 0.5))}' fill='url(#p)' opacity='${r1((0.4 + rand() * 0.6) * opMax * 100) / 100}'/>`;
                        }
                    }
                }
                const puff = `<radialGradient id='p'><stop offset='0' stop-color='#fff' stop-opacity='.85'/>` +
                    `<stop offset='.5' stop-color='#fff' stop-opacity='.4'/><stop offset='1' stop-color='#fff' stop-opacity='0'/></radialGradient>`;
                return svgUrl(w, h, puff, body);
            };
            return drift([
                tile(cloud(2600, 1000, 7, 21, .55), 2600, 1000, 1, 0),
                tile(cloud(3400, 1100, 6, 22, .38), 3400, 1100, 1, 0),
                fixed('linear-gradient(180deg, #3a5a8a, #6683b0, #9db4d6)'),
            ], 220);
        }
    },
    {
        key: 'gentle-snowfall', name: 'Gentle Snowfall', animated: true,
        preview: 'linear-gradient(180deg, #0d1b2a, #1b3a5c, #2c5480)',
        build: () => drift([
            tile(dotField({ w: 2000, h: 1300, n: 26, seed: 31, rMin: 5, rMax: 12, opMin: .5, opMax: .9, colors: ['255,255,255', '226,240,255'], solid: .45 }), 2000, 1300, 1, 1),
            tile(dotField({ w: 1300, h: 900, n: 55, seed: 32, rMin: 2.5, rMax: 6, opMin: .55, opMax: .95, colors: ['255,255,255'], solid: .6 }), 1300, 900, 0, 1),
            tile(dotField({ w: 900, h: 700, n: 70, seed: 33, rMin: 1.2, rMax: 3, opMin: .4, opMax: .8, colors: ['255,255,255', '210,230,255'], solid: .7 }), 900, 700, 0, 1),
            fixed('linear-gradient(180deg, #0d1b2a, #1b3a5c, #2c5480)'),
        ], 70)
    },
    {
        key: 'aurora-waves', name: 'Aurora Waves', animated: true,
        preview: 'linear-gradient(180deg, #050814, #0a1128, #030509)',
        build: () => sway([
            glide(blob(50, 50, '0,255,157', .42, 50, 30), '170% 150%', '0% 0%', '100% 60%'),
            glide(blob(50, 50, '123,47,247', .36, 45, 32), '190% 160%', '100% 20%', '0% 80%'),
            glide(blob(50, 50, '0,200,255', .32, 42, 28), '160% 140%', '20% 100%', '90% 10%'),
            glide(blob(50, 50, '0,255,200', .22, 30, 18), '140% 120%', '90% 40%', '10% 60%'),
            fixed('linear-gradient(180deg, #050814, #0a1128, #030509)'),
        ], 38)
    },
    {
        key: 'starfield-drift', name: 'Starfield Drift', animated: true,
        preview: 'radial-gradient(ellipse at center, #0a0e27, #000000)',
        build: () => drift([
            tile(dotField({ w: 2700, h: 1900, n: 40, seed: 41, rMin: 3, rMax: 7, opMin: .55, opMax: 1, colors: ['255,236,200', '255,255,255'], solid: .1, core: '#ffffff' }), 2700, 1900, 1, 1),
            tile(dotField({ w: 1900, h: 1500, n: 80, seed: 42, rMin: 2, rMax: 4.5, opMin: .45, opMax: .9, colors: ['200,220,255', '255,255,255'], solid: .1, core: '#f2f6ff' }), 1900, 1500, 1, 1),
            tile(dotField({ w: 1300, h: 1000, n: 120, seed: 43, rMin: 1, rMax: 2.6, opMin: .35, opMax: .8, colors: ['180,200,255', '255,255,255'], solid: .2 }), 1300, 1000, 1, 1),
            fixed(blob(72, 28, '78,52,140', .34, 55, 45)),
            fixed(blob(20, 82, '24,70,130', .3, 55, 45)),
            fixed('radial-gradient(ellipse at center, #0a0e27, #000000)'),
        ], 420)
    },

    // ---- animated: new -------------------------------------------------------
    {
        key: 'rising-embers', name: 'Rising Embers', animated: true,
        preview: 'linear-gradient(0deg, #4a1507, #1a0a08 55%, #0b0505)',
        build: () => drift([
            tile(dotField({ w: 2100, h: 1100, n: 30, seed: 51, rMin: 4, rMax: 10, opMin: .5, opMax: .95, colors: ['255,138,61', '255,179,71'], solid: .12, core: '#fff1c9' }), 2100, 1100, 1, -1),
            tile(dotField({ w: 1300, h: 1000, n: 48, seed: 52, rMin: 2, rMax: 5, opMin: .45, opMax: .9, colors: ['255,110,40', '255,160,60'], solid: .15, core: '#ffe2a8' }), 1300, 1000, 0, -1),
            fixed('radial-gradient(ellipse 90% 45% at 50% 112%, rgba(255,90,20,.5), transparent 100%)'),
            fixed('linear-gradient(0deg, #3a0f05 0%, #1a0a08 55%, #0b0505 100%)'),
        ], 50)
    },
    {
        key: 'drifting-bokeh', name: 'Drifting Bokeh', animated: true,
        preview: 'linear-gradient(135deg, #0d2330, #1a1b3f 60%, #2a1740)',
        build: () => drift([
            tile(dotField({ w: 2600, h: 1500, n: 14, seed: 61, rMin: 60, rMax: 150, opMin: .12, opMax: .3, colors: ['255,200,120', '255,150,170', '120,210,255'], solid: .5, ring: true }), 2600, 1500, 1, -1),
            tile(dotField({ w: 1800, h: 1100, n: 24, seed: 62, rMin: 28, rMax: 90, opMin: .15, opMax: .38, colors: ['255,210,140', '255,170,190', '140,220,255', '200,170,255'], solid: .5, ring: true }), 1800, 1100, 1, -1),
            tile(dotField({ w: 1300, h: 800, n: 30, seed: 63, rMin: 14, rMax: 40, opMin: .2, opMax: .45, colors: ['255,225,170', '255,190,210'], solid: .5, ring: true }), 1300, 800, 1, -1),
            fixed('linear-gradient(135deg, #0d2330, #1a1b3f 60%, #2a1740)'),
        ], 180)
    },
    {
        key: 'fireflies', name: 'Fireflies', animated: true,
        preview: 'linear-gradient(180deg, #05110d, #0b2a1e 60%, #12392a)',
        build: () => drift([
            tile(dotField({ w: 1900, h: 1200, n: 16, seed: 71, rMin: 6, rMax: 15, opMin: .45, opMax: .9, colors: ['216,255,106', '240,255,150'], solid: .1, core: '#f6ffc4' }), 1900, 1200, 1, -1),
            tile(dotField({ w: 1400, h: 900, n: 22, seed: 72, rMin: 4, rMax: 10, opMin: .4, opMax: .85, colors: ['200,255,120', '230,255,140'], solid: .1, core: '#efffb8' }), 1400, 900, -1, -1),
            tile(dotField({ w: 1000, h: 700, n: 26, seed: 73, rMin: 3, rMax: 7, opMin: .3, opMax: .7, colors: ['190,255,110'], solid: .12, core: '#e6ffa8' }), 1000, 700, 1, -1),
            fixed('radial-gradient(ellipse 80% 50% at 50% 110%, rgba(60,140,80,.35), transparent 100%)'),
            fixed('linear-gradient(180deg, #05110d, #0b2a1e 60%, #12392a)'),
        ], 140)
    },
    {
        key: 'soft-rain', name: 'Soft Rain', animated: true,
        preview: 'linear-gradient(180deg, #0f1a26, #1c2b3c 60%, #26394d)',
        build: () => drift([
            tile(streakField(1500, 1500, 34, 81, 70, 140, 1.8, '205,225,255', .22, .5), 1500, 1500, 0, 1),
            tile(streakField(1100, 1100, 46, 82, 40, 90, 1.3, '195,215,250', .18, .4), 1100, 1100, 0, 1),
            tile(streakField(800, 800, 44, 83, 22, 50, 1, '190,210,245', .14, .3), 800, 800, 0, 1),
            fixed('radial-gradient(ellipse 100% 40% at 50% 110%, rgba(150,180,220,.22), transparent 100%)'),
            fixed('linear-gradient(180deg, #0f1a26, #1c2b3c 60%, #26394d)'),
        ], 4.5)
    },
    {
        key: 'slow-waves', name: 'Slow Waves', animated: true,
        preview: 'linear-gradient(180deg, #07203a, #0b3a5c 60%, #0e5a7a)',
        build: () => drift([
            { image: waveBand(1800, 300, [[1, .1, 0], [3, .035, 1.3]], 'rgba(255,255,255,.085)'), size: '1800px 34%', repeat: 'repeat-x', from: '0px 100%', to: '1800px 100%' },
            { image: waveBand(2400, 300, [[1, .12, 2], [4, .03, .4]], 'rgba(120,200,255,.13)'), size: '2400px 44%', repeat: 'repeat-x', from: '0px 100%', to: '-2400px 100%' },
            { image: waveBand(1500, 300, [[1, .09, 4], [2, .05, 2.2]], 'rgba(8,40,70,.55)'), size: '1500px 56%', repeat: 'repeat-x', from: '0px 100%', to: '1500px 100%' },
            fixed('linear-gradient(180deg, #07203a, #0b3a5c 60%, #0e5a7a)'),
        ], 70)
    },
    {
        key: 'liquid-gradient', name: 'Liquid Gradient', animated: true,
        preview: 'linear-gradient(120deg, #1b0a3c, #4a1670, #8a2a6d, #2a3f8f)',
        build: () => sway([
            glide('linear-gradient(120deg, #1b0a3c 0%, #4a1670 22%, #8a2a6d 42%, #2a3f8f 62%, #123a63 80%, #1b0a3c 100%)', '400% 400%', '0% 50%', '100% 50%'),
        ], 32)
    },
    {
        key: 'swaying-rays', name: 'Swaying Rays', animated: true,
        preview: 'linear-gradient(180deg, #0d1020, #1a1f3c 60%, #2a2a4a)',
        build: () => sway([
            glide(rayFan(91, 150, 210, 7, '255,236,190', .07, .16), '150% 100%', '10% 0%', '90% 0%'),
            glide(rayFan(92, 155, 205, 9, '255,244,215', .05, .12), '190% 100%', '85% 0%', '15% 0%'),
            fixed('radial-gradient(ellipse 60% 35% at 50% -5%, rgba(255,240,200,.35), transparent 100%)'),
            fixed('linear-gradient(180deg, #0d1020, #1a1f3c 60%, #2a2a4a)'),
        ], 22)
    },
    {
        key: 'gold-shimmer', name: 'Gold Shimmer', animated: true,
        preview: 'linear-gradient(135deg, #1c1407, #3a2a0c 50%, #1c1407)',
        build: () => sway([
            glide('linear-gradient(105deg, transparent 36%, rgba(255,215,130,.07) 44%, rgba(255,232,170,.24) 50%, rgba(255,215,130,.07) 56%, transparent 64%)', '320% 100%', '100% 0%', '0% 0%'),
            glide('linear-gradient(112deg, transparent 40%, rgba(255,200,110,.05) 47%, rgba(255,222,150,.14) 50%, rgba(255,200,110,.05) 53%, transparent 60%)', '460% 100%', '0% 0%', '100% 0%'),
            fixed('linear-gradient(135deg, #1c1407, #3a2a0c 50%, #1c1407)'),
        ], 16)
    },
    {
        key: 'living-mesh', name: 'Living Mesh', animated: true,
        preview: 'linear-gradient(135deg, #1a0d33, #3b1a5a 50%, #14305a)',
        build: () => sway([
            glide(blob(50, 50, '219,39,119', .5, 36, 36), '190% 190%', '0% 0%', '100% 70%'),
            glide(blob(50, 50, '79,70,229', .55, 38, 38), '200% 200%', '100% 10%', '10% 90%'),
            glide(blob(50, 50, '20,184,166', .4, 32, 32), '180% 180%', '30% 100%', '80% 0%'),
            glide(blob(50, 50, '245,158,11', .3, 26, 26), '170% 170%', '90% 90%', '0% 20%'),
            fixed('linear-gradient(135deg, #120a24, #1a0d33)'),
        ], 26)
    },
    {
        key: 'candle-pulse', name: 'Candlelight', animated: true,
        preview: 'linear-gradient(180deg, #0a0604, #3a1a08)',
        build: () => sway([
            { image: 'radial-gradient(ellipse 50% 58% at 50% 100%, rgba(255,175,70,.55), rgba(255,110,25,.2) 48%, transparent 78%)', size: '95% 95%', sizeTo: '125% 125%', repeat: 'no-repeat', from: '50% 100%', to: '50% 100%' },
            { image: 'radial-gradient(ellipse 28% 34% at 50% 100%, rgba(255,225,150,.5), transparent 100%)', size: '100% 100%', sizeTo: '80% 86%', repeat: 'no-repeat', from: '50% 100%', to: '50% 100%' },
            fixed('linear-gradient(180deg, #0a0604, #1a0d06 70%, #26120a)'),
        ], 3.6)
    },

    // ---- still: new ----------------------------------------------------------
    {
        key: 'mesh-dusk', name: 'Mesh Dusk', animated: false,
        preview: 'linear-gradient(135deg, #3a1d63, #1a1033 50%, #7a2d5e)',
        build: () => still([
            fixed(blob(14, 18, '112,52,170', .8, 60, 55)),
            fixed(blob(88, 14, '196,70,110', .55, 55, 50)),
            fixed(blob(78, 90, '40,80,170', .7, 60, 55)),
            fixed(blob(8, 92, '140,45, 100', .55, 50, 50)),
            fixed(flat('#1a1033')),
        ])
    },
    {
        key: 'mesh-ember', name: 'Mesh Ember', animated: false,
        preview: 'linear-gradient(135deg, #5a1a0a, #1c0c0a 50%, #7a2a10)',
        build: () => still([
            fixed(blob(85, 95, '255,98,30', .6, 62, 55)),
            fixed(blob(12, 80, '190,40,40', .55, 55, 55)),
            fixed(blob(55, 5, '120,30,20', .6, 60, 45)),
            fixed(blob(95, 20, '250,170,60', .22, 40, 40)),
            fixed(flat('#1c0c0a')),
        ])
    },
    {
        key: 'mesh-ocean', name: 'Mesh Ocean', animated: false,
        preview: 'linear-gradient(135deg, #0b3a5c, #0a1830 50%, #0e6a78)',
        build: () => still([
            fixed(blob(10, 12, '20,120,170', .7, 60, 55)),
            fixed(blob(92, 30, '16,150,150', .5, 55, 55)),
            fixed(blob(55, 100, '30,70,170', .65, 70, 55)),
            fixed(blob(30, 70, '10,50,100', .6, 40, 40)),
            fixed(flat('#0a1830')),
        ])
    },
    {
        key: 'mesh-rose', name: 'Mesh Rose', animated: false,
        preview: 'linear-gradient(135deg, #5a2144, #1d0f26 50%, #8a5a3a)',
        build: () => still([
            fixed(blob(18, 12, '178,70,120', .6, 58, 52)),
            fixed(blob(92, 78, '210,150,90', .38, 55, 55)),
            fixed(blob(40, 96, '90,40,120', .7, 62, 50)),
            fixed(blob(98, 8, '70,50,140', .45, 45, 45)),
            fixed(flat('#1d0f26')),
        ])
    },
    {
        key: 'cathedral-rays', name: 'Cathedral Rays', animated: false,
        preview: 'linear-gradient(180deg, #0d1020, #1e2344 60%, #2c2c50)',
        build: () => still([
            fixed(rayFan(101, 148, 212, 8, '255,238,195', .09, .2)),
            fixed(rayFan(102, 152, 208, 11, '255,246,222', .05, .12)),
            fixed('radial-gradient(ellipse 65% 40% at 50% -4%, rgba(255,240,205,.4), transparent 100%)'),
            fixed('linear-gradient(180deg, #0d1020, #1a1f3c 60%, #2a2a4a)'),
        ])
    },
    {
        key: 'stained-glass', name: 'Stained Glass', animated: false,
        preview: 'linear-gradient(135deg, #5a1030, #14204a 50%, #0e4a3a)',
        build: () => still([
            fixed(blob(22, 28, '200,30,70', .55, 38, 46)),
            fixed(blob(70, 22, '40,80,210', .5, 36, 44)),
            fixed(blob(50, 75, '20,160,110', .42, 40, 40)),
            fixed(blob(86, 80, '230,170,40', .4, 30, 36)),
            fixed(blob(10, 85, '130,50,190', .4, 30, 34)),
            fixed('radial-gradient(ellipse 85% 85% at 50% 50%, transparent 55%, rgba(0,0,0,.55) 100%)'),
            fixed(flat('#0c0a14')),
        ])
    },
    {
        key: 'spotlight', name: 'Spotlight', animated: false,
        preview: 'radial-gradient(ellipse at 50% 40%, #3a3f55, #1a1d2b 55%, #07080d)',
        build: () => still([
            fixed('radial-gradient(ellipse 62% 74% at 50% 40%, #3c4259 0%, #1c2031 52%, #07080d 100%)'),
        ])
    },
    {
        key: 'paper-grain', name: 'Paper Grain', animated: false,
        preview: 'linear-gradient(180deg, #2a241c, #1d1812)',
        build: () => still([
            fixed(grain(512, .14), '512px 512px', 'repeat'),
            fixed('radial-gradient(ellipse 85% 85% at 50% 45%, transparent 45%, rgba(0,0,0,.5) 100%)'),
            fixed('linear-gradient(180deg, #2c261d, #1d1812)'),
        ])
    },
    {
        key: 'soft-bokeh', name: 'Soft Bokeh', animated: false,
        preview: 'linear-gradient(135deg, #12263a, #2a1740)',
        build: () => still([
            fixed(dotField({ w: 2400, h: 1400, n: 22, seed: 111, rMin: 30, rMax: 120, opMin: .12, opMax: .34, colors: ['255,200,120', '255,150,170', '120,210,255', '200,170,255'], solid: .5, ring: true }), '2400px 1400px', 'repeat'),
            fixed('linear-gradient(135deg, #12263a, #1a1b3f 55%, #2a1740)'),
        ])
    },
    {
        key: 'horizon-glow', name: 'Horizon Glow', animated: false,
        preview: 'linear-gradient(180deg, #0b1030, #2a2060 45%, #b4506a 78%, #f2a65a)',
        build: () => still([
            fixed('linear-gradient(180deg, transparent 0%, transparent 86%, #140a1c 86.4%, #0c0612 100%)'),
            fixed('radial-gradient(ellipse 55% 22% at 50% 86%, rgba(255,200,120,.75), rgba(255,140,90,.25) 50%, transparent 100%)'),
            fixed('linear-gradient(180deg, #0b1030 0%, #2a2060 38%, #8a3f78 62%, #d9667a 78%, #f2a65a 86%)'),
        ])
    },
];

const BY_KEY = new Map(BACKGROUND_PATTERNS.map(p => [p.key, p]));
const specCache = new Map<string, BackgroundPatternSpec>();

export function getPatternMeta(key: string): BackgroundPatternMeta | null {
    return BY_KEY.get(key) ?? null;
}

/** Built lazily (SVG generation) and memoised. */
export function getPatternSpec(key: string): BackgroundPatternSpec | null {
    const meta = BY_KEY.get(key);
    if (!meta) return null;
    let spec = specCache.get(key);
    if (!spec) { spec = meta.build(); specCache.set(key, spec); }
    return spec;
}

export function patternAnimationName(key: string, k = 1): string {
    return 'bgPattern_' + key.replace(/[^a-z0-9]+/gi, '_') + scaleTag(k);
}

/** Multiplies every px length in a CSS value list (tile sizes / offsets). */
export function scalePx(v: string, k: number): string {
    if (k === 1) return v;
    return v.replace(/(-?\d+(?:\.\d+)?)px/g, (_, n) => `${r1(parseFloat(n) * k)}px`);
}

/** Pixel-sized tiles are designed for a 1920-wide screen; scale for smaller/larger boxes. */
export function patternScaleFor(width: number): number {
    if (!(width > 0)) return 1;
    return Math.max(0.1, Math.round((width / 1920) * 20) / 20);
}

const scaleTag = (k: number): string => (k === 1 ? '' : '_s' + Math.round(k * 100));

/** @keyframes text for one pattern; '' when it doesn't move. */
export function patternKeyframes(key: string, k = 1): string {
    const spec = getPatternSpec(key);
    if (!spec || spec.durationSec <= 0) return '';
    const resizes = spec.layers.some(l => l.sizeTo);
    const sc = (v: string): string => scalePx(v, k);
    const from = spec.layers.map(l => sc(l.from ?? '0% 0%')).join(', ');
    const to = spec.layers.map(l => sc(l.to ?? l.from ?? '0% 0%')).join(', ');
    const body = `from { background-position: ${from}; ${resizes ? `background-size: ${spec.layers.map(l => sc(l.size)).join(', ')};` : ''} } ` +
        `to { background-position: ${to}; ${resizes ? `background-size: ${spec.layers.map(l => sc(l.sizeTo ?? l.size)).join(', ')};` : ''} }`;
    return `@keyframes ${patternAnimationName(key, k)} { ${body} }`;
}

const injected = new WeakMap<Document, Set<string>>();

/** Adds the pattern's @keyframes to `doc` once. */
export function ensurePatternKeyframes(doc: Document, key: string, k = 1): void {
    if (!doc || typeof doc !== 'object' || !doc.head) return; // headless/stub documents
    const id = key + scaleTag(k);
    let done = injected.get(doc);
    if (!done) { done = new Set(); injected.set(doc, done); }
    if (done.has(id)) return;
    const css = patternKeyframes(key, k);
    done.add(id);
    if (!css) return;
    let style = doc.getElementById('os-bg-pattern-keyframes') as HTMLStyleElement | null;
    if (!style) {
        style = doc.createElement('style');
        style.id = 'os-bg-pattern-keyframes';
        doc.head.appendChild(style);
    }
    style.appendChild(doc.createTextNode(css + '\n'));
}
