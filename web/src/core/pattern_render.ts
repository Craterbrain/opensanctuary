/**
 * Pre-renders animated "pattern:<key>" backgrounds to cached VP9 video clips.
 *
 * The browser draws every frame of one seamless loop off-screen (an SVG with a
 * <foreignObject> holding the pattern's layered background, rasterised to a
 * canvas), POSTs the PNGs to the server in order, and the server's bundled
 * ffmpeg encodes them (see src/storage/pattern_cache.rs). The live output then
 * plays the clip as an ordinary looping video background instead of redrawing
 * the procedural pattern every frame -- much cheaper on a TV browser.
 *
 * Nothing here is required for correctness: if rendering is unavailable or
 * fails, slides keep using the live procedural background.
 */

import {
    BACKGROUND_PATTERNS, getPatternSpec, patternAutoPrerender, patternFps, patternFrameCount, patternFrameStyle,
    patternPrerenderable, patternRevision,
} from './background_patterns.ts';
import { hostTokenHeader } from './host_session.ts';
import { paintLayers, clearTileCache } from './gradient_canvas.ts';

export type PatternResolutionSetting = 'auto' | '1440' | '1080';

/**
 * Per-pattern choice: 'auto' renders it when it is demanding to draw live (see
 * PatternCost) and quick to render; 'always' renders it ahead of time even if light
 * or slow to render; 'never' always draws it live.
 */
export type PatternMode = 'auto' | 'always' | 'never';

export interface PrerenderConfig {
    /** Master switch. Off = everything is drawn live, whatever the modes say. */
    enabled: boolean;
    resolution: PatternResolutionSetting;
    modes: Record<string, PatternMode>;
}

let config: PrerenderConfig = { enabled: true, resolution: 'auto', modes: {} };

export function parsePatternModes(text: string | undefined | null): Record<string, PatternMode> {
    try {
        const raw = JSON.parse(text || '{}');
        const out: Record<string, PatternMode> = {};
        for (const [k, v] of Object.entries(raw || {})) {
            if (v === 'always' || v === 'never' || v === 'auto') out[k] = v;
        }
        return out;
    } catch {
        return {};
    }
}

export function modeFor(key: string, modes: Record<string, PatternMode> = config.modes): PatternMode {
    return modes[key] ?? 'auto';
}

/** Should this pattern get a pre-rendered clip? `trigger` 'ahead' (startup) only honours 'always'. */
export function shouldPrerender(key: string, trigger: 'use' | 'ahead' = 'use', cfg: PrerenderConfig = config): boolean {
    if (!cfg.enabled || !patternPrerenderable(key)) return false;
    const mode = modeFor(key, cfg.modes);
    if (mode === 'never') return false;
    if (mode === 'always') return true;
    return trigger === 'use' && patternAutoPrerender(key);
}

export function configurePatternPrerender(next: Partial<PrerenderConfig>): void {
    config = { ...config, ...next };
}

export function getPatternPrerenderConfig(): PrerenderConfig {
    return config;
}

export interface FrameSize { width: number; height: number; }

/** 2560x1440 by default; 1920x1080 when asked, or when auto sees a smaller monitor. */
export function resolutionFor(setting: PatternResolutionSetting, monitorHeights: number[] = []): FrameSize {
    const small = setting === '1080' || (setting === 'auto' && monitorHeights.some(h => h > 0 && h < 1440));
    return small ? { width: 1920, height: 1080 } : { width: 2560, height: 1440 };
}

/**
 * Rasterises one frame to a PNG. The layers are painted onto the canvas directly
 * (see gradient_canvas.ts) rather than via an SVG <foreignObject>, which would
 * taint the canvas in Chromium/WebView2 and make export impossible.
 */
export async function renderPatternFrame(key: string, tSec: number, size: FrameSize, canvas?: HTMLCanvasElement): Promise<Blob> {
    const st = patternFrameStyle(key, tSec, size.width / 1920);
    if (!st) throw new Error(`unknown pattern ${key}`);
    const c = canvas ?? document.createElement('canvas');
    c.width = size.width;
    c.height = size.height;
    const ctx = c.getContext('2d');
    if (!ctx) throw new Error('no 2d context');
    await paintLayers(ctx, size.width, size.height, st.images.map((image, i) => ({
        image, size: st.sizes[i] ?? '', position: st.positions[i] ?? '', repeat: st.repeats[i] ?? 'no-repeat',
    })));
    return new Promise<Blob>((resolve, reject) =>
        c.toBlob(b => (b ? resolve(b) : reject(new Error('canvas export failed'))), 'image/png'));
}

// ---------------------------------------------------------------------------
// Server protocol
// ---------------------------------------------------------------------------

async function postJson<T>(url: string, body: unknown): Promise<T> {
    const res = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...hostTokenHeader() },
        body: JSON.stringify(body),
    });
    if (!res.ok) throw Object.assign(new Error(await res.text()), { status: res.status });
    return res.json() as Promise<T>;
}

const sleep = (ms: number): Promise<void> => new Promise(r => setTimeout(r, ms));

export interface RenderProgress { key: string; done: number; total: number; }

export interface RenderOptions {
    size: FrameSize;
    fps?: number;
    onProgress?: (p: RenderProgress) => void;
    signal?: AbortSignal;
}

/** Renders a whole loop and uploads it; resolves to the clip's URL, or null if not possible. */
export async function renderPatternClip(key: string, opts: RenderOptions): Promise<string | null> {
    if (!patternPrerenderable(key)) return null;
    const fps = opts.fps ?? patternFps(key);
    const frames = patternFrameCount(key);
    const rev = patternRevision(key);

    let started: { state: string; url?: string; job_id?: string };
    try {
        started = await postJson('/api/pattern-cache/start', { key, rev, width: opts.size.width, height: opts.size.height, fps, frames });
    } catch (e) {
        console.warn('pattern pre-render unavailable:', (e as Error).message);
        return null;
    }
    if (started.state === 'ready') return started.url ?? null;
    if (started.state === 'rendering') return null; // someone else is on it; lookups will find the clip
    const jobId = started.job_id!;

    const canvas = document.createElement('canvas');
    clearTileCache();
    try {
        for (let i = 0; i < frames; i++) {
            if (opts.signal?.aborted) throw new Error('aborted');
            const frameStart = performance.now();
            const png = await renderPatternFrame(key, i / fps, opts.size, canvas);
            const res = await fetch(`/api/pattern-cache/jobs/${jobId}/frame`, {
                method: 'POST',
                headers: { 'Content-Type': 'image/png', ...hostTokenHeader() },
                body: png,
            });
            if (!res.ok) throw new Error(`frame upload failed (${res.status})`);
            opts.onProgress?.({ key, done: i + 1, total: frames });
            // Rendering blocks the page for a few hundred ms per frame at 1440p, so while
            // someone is looking at the console rest as long as each frame took (about half
            // duty cycle); in a background tab run flat out.
            await sleep(document.hidden ? 0 : Math.min(400, performance.now() - frameStart));
        }
        const fin = await postJson<{ url: string }>(`/api/pattern-cache/jobs/${jobId}/finish`, {});
        return fin.url;
    } catch (e) {
        console.warn('pattern pre-render failed:', (e as Error).message);
        fetch(`/api/pattern-cache/jobs/${jobId}`, { method: 'DELETE', headers: hostTokenHeader() }).catch(() => {});
        return null;
    }
}

// ---------------------------------------------------------------------------
// Console-side scheduling: one render at a time, de-duplicated
// ---------------------------------------------------------------------------

const attempted = new Set<string>();
let chain: Promise<unknown> = Promise.resolve();
const progressListeners = new Set<(p: RenderProgress | null) => void>();

export function onPatternRenderProgress(fn: (p: RenderProgress | null) => void): () => void {
    progressListeners.add(fn);
    return () => progressListeners.delete(fn);
}

let monitorHeightsProvider: () => Promise<number[]> = async () => [];
export function setMonitorHeightsProvider(fn: () => Promise<number[]>): void {
    monitorHeightsProvider = fn;
}

/**
 * Makes sure a clip exists for `key` (renders it in the background if not).
 * Safe to call on every selection: it is a no-op when pre-rendering is off,
 * the pattern isn't worth rendering, or this session already tried it.
 */
export function ensurePatternClip(key: string, trigger: 'use' | 'ahead' = 'use'): void {
    if (!shouldPrerender(key, trigger) || typeof document === 'undefined') return;
    const spec = getPatternSpec(key);
    if (!spec) return;
    const id = key + ':' + patternRevision(key) + ':' + config.resolution;
    if (attempted.has(id)) return;
    attempted.add(id);
    chain = chain.then(async () => {
        const size = resolutionFor(config.resolution, await monitorHeightsProvider().catch(() => []));
        const url = await renderPatternClip(key, {
            size,
            onProgress: p => progressListeners.forEach(fn => fn(p)),
        });
        progressListeners.forEach(fn => fn(null));
        if (!url) attempted.delete(id); // allow a retry on a later selection
    });
}

/** Queues every pattern set to 'always' that has no clip yet (call once things have settled after startup). */
export function prerenderAlwaysPatterns(): void {
    for (const p of BACKGROUND_PATTERNS) {
        if (p.animated) ensurePatternClip(p.key, 'ahead');
    }
}

// ---------------------------------------------------------------------------
// Playback-side lookup (live output)
// ---------------------------------------------------------------------------

const clipCache = new Map<string, string | null>();
const clipPending = new Set<string>();
const firstLookAt = new Map<string, number>();

// The live output has no console settings in memory, so it reads the (non-secret)
// settings itself, briefly cached, to honour the master switch and 'never'.
let playbackPolicy: { at: number; enabled: boolean; modes: Record<string, PatternMode> } | null = null;

async function clipAllowed(key: string): Promise<boolean> {
    if (!playbackPolicy || Date.now() - playbackPolicy.at > 15_000) {
        try {
            const res = await fetch('/api/settings');
            const j = res.ok ? await res.json() : {};
            playbackPolicy = { at: Date.now(), enabled: j.patternPrerender !== 'off', modes: parsePatternModes(j.patternModes) };
        } catch {
            playbackPolicy = { at: Date.now(), enabled: true, modes: {} };
        }
    }
    return playbackPolicy.enabled && modeFor(key, playbackPolicy.modes) !== 'never';
}

/** Cached clip URL for a pattern, or null. Kicks off a lookup the first time and calls `onFound` if one exists. */
export function lookupPatternClip(key: string, onFound?: () => void): string | null {
    if (!patternPrerenderable(key)) return null;
    const rev = patternRevision(key);
    const id = key + ':' + rev;
    if (clipCache.has(id)) {
        const url = clipCache.get(id) ?? null;
        // A clip found earlier stops being used once the setting says never (as last read).
        if (url && playbackPolicy && !(playbackPolicy.enabled && modeFor(key, playbackPolicy.modes) !== 'never')) return null;
        return url;
    }
    if (!clipPending.has(id)) {
        clipPending.add(id);
        if (!firstLookAt.has(id)) firstLookAt.set(id, Date.now());
        clipAllowed(key)
            .then(ok => (ok ? fetch(`/api/pattern-cache/lookup?key=${encodeURIComponent(key)}&rev=${rev}`) : null))
            .then(r => (r && r.ok ? r.json() : { url: null }))
            .then((j: { url: string | null }) => {
                clipPending.delete(id);
                if (j.url) {
                    clipCache.set(id, j.url);
                    onFound?.();
                } else {
                    // Not rendered (yet): keep the miss for a while, then look again (every
                    // 20s for up to 20 minutes) in case the console finishes a render while
                    // the live pattern is on screen -- that is what triggers the handoff.
                    clipCache.set(id, null);
                    if (Date.now() - firstLookAt.get(id)! < 20 * 60_000) {
                        setTimeout(() => { clipCache.delete(id); lookupPatternClip(key, onFound); }, 20_000);
                    } else {
                        setTimeout(() => { clipCache.delete(id); firstLookAt.delete(id); }, 20_000);
                    }
                }
            })
            .catch(() => { clipPending.delete(id); });
    }
    return null;
}

