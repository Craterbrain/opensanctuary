/**
 * Draws the small subset of CSS background layers the procedural patterns use
 * onto a 2D canvas: linear/radial/conic gradients and SVG-image tiles.
 *
 * Why this exists: the obvious way to rasterise a CSS background -- an SVG
 * <foreignObject> drawn to a canvas -- taints the canvas in Chromium (so
 * toBlob() throws), and WebView2 is Chromium. Drawing the layers ourselves keeps
 * the canvas exportable. The grammar handled is exactly what
 * background_patterns.ts emits, not general CSS.
 */

type RGBA = [number, number, number, number];

interface Stop { color: RGBA; pos: number | null; }

export interface ParsedGradient {
    kind: 'linear' | 'radial' | 'conic';
    angleDeg: number;
    /** radial: explicit radii in % of the tile, or null for farthest-corner */
    rx: number | null;
    ry: number | null;
    /** centre in % of the tile */
    cx: number;
    cy: number;
    stops: Stop[];
    /** conic stop positions are in degrees (else %) */
    stopsInDeg: boolean;
}

const NAMED: Record<string, RGBA> = { transparent: [0, 0, 0, 0], black: [0, 0, 0, 1], white: [255, 255, 255, 1] };

export function parseColor(text: string): RGBA {
    const t = text.trim().toLowerCase();
    if (NAMED[t]) return NAMED[t];
    if (t[0] === '#') {
        const h = t.slice(1);
        const full = h.length === 3 ? h.split('').map(c => c + c).join('') : h;
        return [parseInt(full.slice(0, 2), 16), parseInt(full.slice(2, 4), 16), parseInt(full.slice(4, 6), 16), 1];
    }
    const m = /^rgba?\(([^)]*)\)$/.exec(t);
    if (m) {
        const p = (m[1] ?? '').split(',').map(s => parseFloat(s));
        return [p[0] ?? NaN, p[1] ?? NaN, p[2] ?? NaN, p.length > 3 ? (p[3] ?? 1) : 1];
    }
    throw new Error(`unsupported colour: ${text}`);
}

/** Splits on commas that are not inside parentheses. */
function splitTop(s: string): string[] {
    const out: string[] = [];
    let depth = 0, cur = '';
    for (const ch of s) {
        if (ch === '(') depth++;
        if (ch === ')') depth--;
        if (ch === ',' && depth === 0) { out.push(cur.trim()); cur = ''; } else cur += ch;
    }
    if (cur.trim()) out.push(cur.trim());
    return out;
}

export function parseGradient(css: string): ParsedGradient | null {
    const m = /^(linear|radial|conic)-gradient\((.*)\)$/s.exec(css.trim());
    if (!m) return null;
    const kind = m[1] as ParsedGradient['kind'];
    const parts = splitTop(m[2] ?? '');
    const g: ParsedGradient = { kind, angleDeg: kind === 'linear' ? 180 : 0, rx: null, ry: null, cx: 50, cy: 50, stops: [], stopsInDeg: kind === 'conic' };

    const first = parts[0] ?? '';
    const isStop = (p: string): boolean => /^(#|rgb|transparent)/i.test(p);
    if (!isStop(first)) {
        parts.shift();
        if (kind === 'linear') {
            g.angleDeg = parseFloat(first);
        } else {
            const at = /at\s+(.+)$/.exec(first);
            if (at) {
                const where = (at[1] ?? '').trim();
                if (where === 'center') { g.cx = 50; g.cy = 50; } else {
                    const [x, y] = where.split(/\s+/);
                    g.cx = parseFloat(x ?? ''); g.cy = parseFloat(y ?? '');
                }
            }
            if (kind === 'conic') {
                const from = /from\s+(-?[\d.]+)deg/.exec(first);
                g.angleDeg = from ? parseFloat(from[1] ?? '') : 0;
            } else {
                const size = /ellipse\s+([\d.]+)%\s+([\d.]+)%/.exec(first);
                if (size) { g.rx = parseFloat(size[1] ?? ''); g.ry = parseFloat(size[2] ?? ''); }
            }
        }
    }
    for (const p of parts) {
        const m2 = /^(.*?)\s+(-?[\d.]+)(%|deg)$/.exec(p);
        const colorText = (m2 ? m2[1] : undefined) ?? p;
        g.stops.push({ color: parseColor(colorText), pos: m2 ? parseFloat(m2[2] ?? '') : null });
    }
    return g;
}

/** Fills in omitted stop positions (0 first, 100/360 last, even in between). */
function resolvePositions(stops: Stop[], max: number): Array<{ color: RGBA; t: number }> {
    const pos = stops.map(s => s.pos);
    if (pos[0] === null) pos[0] = 0;
    if (pos[pos.length - 1] === null) pos[pos.length - 1] = max;
    let i = 0;
    while (i < pos.length) {
        if (pos[i] !== null) { i++; continue; }
        let j = i;
        while (pos[j] === null) j++;
        const a = pos[i - 1]!, b = pos[j]!;
        for (let k = i; k < j; k++) pos[k] = a + ((b - a) * (k - i + 1)) / (j - i + 1);
        i = j;
    }
    let prev = -Infinity;
    return stops.map((s, k) => {
        const p = Math.max(prev, Math.min(max, pos[k]!));
        prev = p;
        return { color: s.color, t: p / max };
    });
}

const css = (c: RGBA, a = c[3]): string => `rgba(${Math.round(c[0])},${Math.round(c[1])},${Math.round(c[2])},${Math.round(a * 1000) / 1000})`;

/**
 * CSS interpolates in premultiplied space, so `transparent` never greys a fade;
 * canvas does not. Give each fully transparent stop the colour of its visible
 * neighbour(s) (two stops at one position when the neighbours differ).
 */
function addStops(grad: CanvasGradient, resolved: Array<{ color: RGBA; t: number }>): void {
    const visible = (c: RGBA): boolean => c[3] > 0;
    resolved.forEach((s, i) => {
        if (visible(s.color)) { grad.addColorStop(s.t, css(s.color)); return; }
        const prev = [...resolved.slice(0, i)].reverse().find(r => visible(r.color));
        const next = resolved.slice(i + 1).find(r => visible(r.color));
        const colours = [prev, next].filter(Boolean).map(r => r!.color);
        const [first, second] = colours;
        if (!first) { grad.addColorStop(s.t, 'rgba(0,0,0,0)'); return; }
        if (second && (first[0] !== second[0] || first[1] !== second[1] || first[2] !== second[2])) {
            // fade out the previous colour, then fade in the next one, at the same point
            grad.addColorStop(s.t, css(first, 0));
            grad.addColorStop(s.t, css(second, 0));
        } else {
            grad.addColorStop(s.t, css(first, 0));
        }
    });
}

/** Paints a gradient into the rectangle (x, y, w, h). */
export function paintGradient(ctx: CanvasRenderingContext2D, g: ParsedGradient, x: number, y: number, w: number, h: number): void {
    ctx.save();
    ctx.beginPath();
    ctx.rect(x, y, w, h);
    ctx.clip();
    const resolved = resolvePositions(g.stops, g.stopsInDeg ? 360 : 100);
    const cx = x + (g.cx / 100) * w, cy = y + (g.cy / 100) * h;

    if (g.kind === 'linear') {
        const a = (g.angleDeg * Math.PI) / 180;
        const dx = Math.sin(a), dy = -Math.cos(a);
        const len = Math.abs(w * dx) + Math.abs(h * dy);
        const mx = x + w / 2, my = y + h / 2;
        const grad = ctx.createLinearGradient(mx - (dx * len) / 2, my - (dy * len) / 2, mx + (dx * len) / 2, my + (dy * len) / 2);
        addStops(grad, resolved);
        ctx.fillStyle = grad;
        ctx.fillRect(x, y, w, h);
    } else if (g.kind === 'radial') {
        let rx: number, ry: number;
        if (g.rx !== null && g.ry !== null) {
            rx = (g.rx / 100) * w;
            ry = (g.ry / 100) * h;
        } else {
            // farthest-corner ellipse: farthest-side ratio scaled by sqrt(2)
            rx = Math.max(cx - x, x + w - cx) * Math.SQRT2;
            ry = Math.max(cy - y, y + h - cy) * Math.SQRT2;
        }
        ctx.translate(cx, cy);
        ctx.scale(rx, ry);
        const grad = ctx.createRadialGradient(0, 0, 0, 0, 0, 1);
        addStops(grad, resolved);
        ctx.fillStyle = grad;
        ctx.fillRect((x - cx) / rx, (y - cy) / ry, w / rx, h / ry);
    } else {
        const start = ((g.angleDeg - 90) * Math.PI) / 180;
        const grad = ctx.createConicGradient(start, cx, cy);
        addStops(grad, resolved);
        ctx.fillStyle = grad;
        ctx.fillRect(x, y, w, h);
    }
    ctx.restore();
}

// ---------------------------------------------------------------------------
// Layer stacks
// ---------------------------------------------------------------------------

export interface CanvasLayer {
    /** a gradient string, or url("data:...") */
    image: string;
    size: string;
    position: string;
    repeat: string;
}

/** CSS length token against a container extent. */
function px(tok: string, extent: number): number {
    return tok.endsWith('%') ? (parseFloat(tok) / 100) * extent : parseFloat(tok);
}

const decoded = new Map<string, Promise<HTMLImageElement>>();

function loadSvg(src: string): Promise<HTMLImageElement> {
    let p = decoded.get(src);
    if (!p) {
        p = new Promise((resolve, reject) => {
            const img = new Image();
            img.onload = () => resolve(img);
            img.onerror = () => reject(new Error('tile image failed to load'));
            img.src = src;
        });
        decoded.set(src, p);
        if (decoded.size > 64) decoded.delete(decoded.keys().next().value as string);
    }
    return p;
}

/** Drops cached decoded tiles (call between renders). */
export function clearTileCache(): void {
    decoded.clear();
}

/** Paints layers (first = top, like CSS background-image) onto the canvas. */
export async function paintLayers(ctx: CanvasRenderingContext2D, W: number, H: number, layers: CanvasLayer[], base = '#000'): Promise<void> {
    ctx.fillStyle = base;
    ctx.fillRect(0, 0, W, H);
    for (let i = layers.length - 1; i >= 0; i--) {
        const l = layers[i];
        if (!l) continue;
        const [sw = '', sh] = l.size.split(/\s+/);
        const [pxTok = '', pyTok] = l.position.split(/\s+/);
        const w = px(sw, W), h = px(sh ?? sw, H);
        let x = pxTok.endsWith('%') ? ((W - w) * parseFloat(pxTok)) / 100 : parseFloat(pxTok);
        let y = (pyTok ?? '0%').endsWith('%') ? ((H - h) * parseFloat(pyTok ?? '0')) / 100 : parseFloat(pyTok ?? '');
        const repX = l.repeat === 'repeat' || l.repeat === 'repeat-x';
        const repY = l.repeat === 'repeat';
        if (repX && w > 0) x = ((x % w) - w) % w;
        if (repY && h > 0) y = ((y % h) - h) % h;

        const grad = parseGradient(l.image);
        const url = /^url\("(.*)"\)$/s.exec(l.image);
        const img = !grad && url ? await loadSvg(url[1] ?? '') : null;
        if (!grad && !img) throw new Error('unsupported background layer');

        for (let ty = y; ty < (repY ? H : y + 1); ty += repY ? h : 1) {
            for (let tx = x; tx < (repX ? W : x + 1); tx += repX ? w : 1) {
                if (grad) paintGradient(ctx, grad, tx, ty, w, h);
                else ctx.drawImage(img!, tx, ty, w, h);
                if (!repX) break;
            }
            if (!repY) break;
        }
    }
}
