/**
 * OpenSanctuary / OS-Next Presentation & Media Helpers
 * Pure business logic and resolution algorithms for worship slides, media, and scripture.
 */

import { getReorderDestinationIndex } from "@atlaskit/pragmatic-drag-and-drop-hitbox/util/get-reorder-destination-index";
import { GRADIENT_PRESETS, ANIMATED_PATTERN_PRESETS, type SlideBackground, type BackgroundPreset } from "../editor/types.ts";

export type Edge = 'top' | 'bottom' | 'left' | 'right';

export interface ThemeDefinition {
    /** Only present for backend-loaded (user-saved) themes; DEFAULT_THEMES entries have none. */
    id?: string;
    name: string;
    bg: string;
    font?: string;
    color?: string;
    /** "song" | "scripture" | "presentation" — see src/core/models.rs Theme::category. */
    category?: string;
    isDefault?: boolean;
    fontSize?: string;
    textShadow?: string;
    alignment?: string;
    lineHeight?: string;
    letterSpacing?: string;
    opacity?: string;
    marginTop?: string;
    marginBottom?: string;
    marginLeft?: string;
    marginRight?: string;
    verticalAlign?: string;
    /** Scripture themes only: "inline" | "top" | "bottom". */
    referencePosition?: string;
    chromaKeyEnabled?: boolean;
    chromaKeyColor?: string;
    /** 0 = full screen; >0 = bottom N% lower-third band. */
    safeAreaPercent?: number;
}

export interface ScriptureReferenceResult {
    isReference: boolean;
    book?: string;
    chapter?: number | null;
    verseStart?: number | null;
    verseEnd?: number | null;
    hasSpecificVerse?: boolean;
    formatted?: string;
}

export interface ArrangementEntry {
    section_id: string;
    source_slide_index: number;
    background_override?: string | null;
}

/**
 * Resolves the effective arrangement for an item.
 * If `item.arrangement` exists and is non-empty, returns it.
 * Otherwise returns an identity mapping over `item.slides`.
 */
export function effectiveArrangement(item: { slides?: any[]; arrangement?: ArrangementEntry[] | null } | null | undefined): ArrangementEntry[] {
    if (!item) return [];
    if (item.arrangement && Array.isArray(item.arrangement) && item.arrangement.length > 0) {
        return item.arrangement;
    }
    const slides = item.slides || [];
    return slides.map((s, i) => ({
        section_id: s.tag || s.label || getSlideBadge(s, i),
        source_slide_index: i,
        background_override: null,
    }));
}

/**
 * Resolves the master slide referenced at a given arrangement play position,
 * applying any per-play background override.
 */
export function resolveSlideAt(item: { slides?: any[]; arrangement?: ArrangementEntry[] | null } | null | undefined, playPos: number): any | null {
    if (!item) return null;
    const arr = effectiveArrangement(item);
    const entry = arr[playPos];
    if (!entry) return null;
    const slides = item.slides || [];
    const masterSlide = slides[entry.source_slide_index];
    if (!masterSlide) return null;
    if (entry.background_override && entry.background_override.trim()) {
        return { ...masterSlide, background: entry.background_override.trim() };
    }
    return masterSlide;
}

/**
 * 7-Tier Background & Theme Resolution Cascade:
 * 1. Arrangement Per-Play Background Override
 * 2. Slide Background Override
 * 3. Item Background Override
 * 4. Item Theme Background Preset
 * 5. Active Engine Theme Background
 * 6. Global Fallback Theme Background
 * 7. Default Gradient
 */
export function resolveSlideBackground(
    slide: { background?: string | null } | null | undefined,
    item: { background?: string | null; theme_name?: string | null } | null | undefined,
    availableThemes: ThemeDefinition[] = [],
    activeTheme: { background_value?: string | null } | null | undefined = null,
    globalThemeName: string | null | undefined = null,
    defaultGradient: string = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)',
    arrangementOverride?: string | null
): string {
    // 1. Arrangement per-play background override (Top Tier)
    if (arrangementOverride && arrangementOverride.trim()) {
        return arrangementOverride.trim();
    }
    // 2. Per-slide background override
    if (slide && slide.background && slide.background.trim()) {
        return slide.background.trim();
    }
    // 3. Per-item background override
    if (item && item.background && item.background.trim()) {
        return item.background.trim();
    }
    // 4. Item-specific theme preset
    if (item && item.theme_name) {
        const theme = availableThemes.find(t => t.name.toLowerCase() === item.theme_name!.toLowerCase());
        if (theme && theme.bg && theme.bg.trim()) {
            return theme.bg.trim();
        }
    }
    // 5. Engine active theme
    if (activeTheme && activeTheme.background_value && activeTheme.background_value.trim()) {
        return activeTheme.background_value.trim();
    }
    // 6. Global fallback theme
    if (globalThemeName) {
        const theme = availableThemes.find(t => t.name.toLowerCase() === globalThemeName.toLowerCase());
        if (theme && theme.bg && theme.bg.trim()) {
            return theme.bg.trim();
        }
    }
    // 7. Default gradient fallback
    return defaultGradient;
}

/**
 * Resolves the background for an item at a specific arrangement play position through the 7-tier cascade.
 */
export function resolveBackgroundAt(
    item: { slides?: any[]; arrangement?: ArrangementEntry[] | null; background?: string | null; theme_name?: string | null } | null | undefined,
    playPos: number,
    availableThemes: ThemeDefinition[] = [],
    activeTheme: { background_value?: string | null } | null | undefined = null,
    globalThemeName: string | null | undefined = null,
    defaultGradient: string = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)'
): string {
    if (!item) return defaultGradient;
    const arr = effectiveArrangement(item);
    const entry = arr[playPos];
    const slide = resolveSlideAt(item, playPos);
    return resolveSlideBackground(
        slide,
        item,
        availableThemes,
        activeTheme,
        globalThemeName,
        defaultGradient,
        entry?.background_override
    );
}

/**
 * Resolves the full `ThemeDefinition` (typography, not just background) applicable to
 * an item: `item.theme_name` lookup, falling back to the engine's `global_theme` name,
 * falling back to `null` (callers apply their own sane built-in defaults). Unlike
 * `resolveSlideBackground`'s cascade, there is no per-item/per-slide typography
 * override concept — a theme's font/shadow/alignment/etc. only ever come from a named
 * Theme, so this is a simple two-step lookup rather than a multi-tier cascade.
 */
export function resolveThemeAt(
    item: { theme_name?: string | null } | null | undefined,
    availableThemes: ThemeDefinition[] = [],
    globalThemeName: string | null | undefined = null
): ThemeDefinition | null {
    if (item && item.theme_name) {
        const theme = availableThemes.find(t => t.name.toLowerCase() === item.theme_name!.toLowerCase());
        if (theme) return theme;
    }
    if (globalThemeName) {
        const theme = availableThemes.find(t => t.name.toLowerCase() === globalThemeName.toLowerCase());
        if (theme) return theme;
    }
    return null;
}

/** Applies a resolved theme's typography as inline styles onto a slide-text container. */
export function applyThemeTypography(el: HTMLElement, theme: ThemeDefinition | null): void {
    if (!theme) {
        el.style.fontFamily = '';
        el.style.fontSize = '';
        el.style.color = '';
        el.style.textShadow = '';
        el.style.lineHeight = '';
        el.style.letterSpacing = '';
        el.style.textAlign = '';
        el.style.padding = '';
        return;
    }
    el.style.fontFamily = theme.font || '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
    if (theme.fontSize) el.style.fontSize = theme.fontSize;
    el.style.color = theme.color || '#ffffff';
    el.style.textShadow = (theme.textShadow && theme.textShadow !== 'none') ? theme.textShadow : 'none';
    if (theme.lineHeight) el.style.lineHeight = theme.lineHeight;
    if (theme.letterSpacing) el.style.letterSpacing = theme.letterSpacing;
    if (theme.alignment) el.style.textAlign = theme.alignment;
    if (theme.marginTop || theme.marginBottom || theme.marginLeft || theme.marginRight) {
        el.style.padding = `${theme.marginTop || '0'} ${theme.marginRight || '0'} ${theme.marginBottom || '0'} ${theme.marginLeft || '0'}`;
    }
}

export const DEFAULT_THEMES: ThemeDefinition[] = [
    { name: 'Midnight Ocean', bg: 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)', font: 'Segoe UI' },
    { name: 'Golden Sanctuary', bg: 'linear-gradient(135deg, #1f1c18, #473e34, #1f1c18)', font: 'Georgia' },
    { name: 'Deep Worship', bg: 'linear-gradient(135deg, #200122, #6f0000)', font: 'Segoe UI' },
    { name: 'Celestial Blue', bg: 'linear-gradient(135deg, #000428, #004e92)', font: 'Arial' },
    { name: 'Forest Grace', bg: 'linear-gradient(135deg, #134e5e, #71b280)', font: 'Verdana' },
    { name: 'Solid Dark', bg: '#1a1a1a', font: 'Segoe UI' },
    { name: 'Ember Fire', bg: 'linear-gradient(135deg, #1c100e, #36150b, #1c100e)', font: 'Segoe UI' }
];

// Auto-populate the theme list from the slide editor's own background presets
// (GRADIENT_PRESETS / ANIMATED_PATTERN_PRESETS in editor/types.ts) so a preset
// added there shows up everywhere DEFAULT_THEMES is used, instead of needing a
// second hand-written entry kept in sync by hand. Solid-kind presets carry a
// "pattern:<name>" marker (resolved by applyResolvedBackground at render
// time), so use it as `bg` for pattern presets; gradient presets have no
// marker, so use their already-CSS-valid `preview` string instead.
{
    const existingThemeNames = new Set(DEFAULT_THEMES.map(t => t.name.toLowerCase()));
    for (const preset of [...GRADIENT_PRESETS, ...ANIMATED_PATTERN_PRESETS]) {
        if (existingThemeNames.has(preset.name.toLowerCase())) continue;
        existingThemeNames.add(preset.name.toLowerCase());
        DEFAULT_THEMES.push({
            name: preset.name,
            bg: preset.background.kind === 'Solid' ? (preset.background.data as string) : preset.preview,
            font: 'Segoe UI'
        });
    }
}

/**
 * Extracts raw image URL from CSS background string or image filename.
 */
export function extractImageUrl(bg: string | null | undefined, availableThemes: ThemeDefinition[] = DEFAULT_THEMES): string | null {
    if (!bg || typeof bg !== 'string') return null;
    const trimmed = bg.trim();
    if (trimmed.startsWith('linear-gradient(') || trimmed.startsWith('radial-gradient(')) return null;
    if (trimmed.startsWith('#') || trimmed.startsWith('rgb(') || trimmed.startsWith('rgba(')) return null;

    // 1. Extract from url('...') or url("...") or url(...)
    const urlMatch = trimmed.match(/url\(\s*['"]?([^'")]+)['"]?\s*\)/i);
    if (urlMatch && urlMatch[1]) {
        return urlMatch[1].trim();
    }

    // 2. Data URLs for images
    if (trimmed.startsWith('data:image/')) {
        return trimmed;
    }

    // 3. Plain image file paths or extensions
    const lower = trimmed.toLowerCase();
    if (lower.endsWith('.png') || lower.endsWith('.jpg') || lower.endsWith('.jpeg') ||
        lower.endsWith('.webp') || lower.endsWith('.gif') || lower.endsWith('.bmp') || lower.endsWith('.svg') ||
        lower.includes('/media/images/') || lower.includes('/images/') || lower.includes('/backgrounds/')) {
        return trimmed;
    }

    // 4. If it matches a named theme, check the theme's background
    const theme = availableThemes.find(t => t.name.toLowerCase() === trimmed.toLowerCase());
    if (theme && theme.bg && theme.bg !== bg) {
        return extractImageUrl(theme.bg, availableThemes);
    }

    return null;
}

export function isImageBackground(bg: string | null | undefined, availableThemes: ThemeDefinition[] = DEFAULT_THEMES): boolean {
    return !!extractImageUrl(bg, availableThemes);
}

export function isVideoBackground(bg: string | null | undefined): boolean {
    if (!bg || typeof bg !== 'string') return false;
    const lower = bg.toLowerCase();
    return lower.endsWith('.mp4') || lower.endsWith('.webm') || lower.endsWith('.mkv') ||
           lower.endsWith('.mov') || lower.endsWith('.m4v') || lower.includes('/media/videos/');
}

export function isAudioMedia(bg: string | null | undefined): boolean {
    if (!bg || typeof bg !== 'string') return false;
    const lower = bg.toLowerCase();
    return lower.endsWith('.mp3') || lower.endsWith('.wav') || lower.endsWith('.m4a') ||
           lower.endsWith('.aac') || lower.endsWith('.flac') || lower.endsWith('.ogg') || lower.includes('/media/audio/');
}

export function formatCssBackground(
    bg: string | null | undefined,
    defaultGradient: string = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)',
    availableThemes: ThemeDefinition[] = DEFAULT_THEMES
): string {
    if (!bg || typeof bg !== 'string') return defaultGradient;
    const trimmed = bg.trim();
    const imgUrl = extractImageUrl(trimmed, availableThemes);
    if (imgUrl) {
        return `url("${imgUrl}") center/cover no-repeat`;
    }
    if (trimmed.startsWith('linear-gradient(') || trimmed.startsWith('radial-gradient(') || trimmed.startsWith('#') || trimmed.startsWith('rgb')) {
        return trimmed;
    }
    const theme = availableThemes.find(t => t.name.toLowerCase() === trimmed.toLowerCase());
    if (theme && theme.bg && theme.bg !== bg) {
        return formatCssBackground(theme.bg, defaultGradient, availableThemes);
    }
    return defaultGradient;
}

/**
 * Animated slide backgrounds: a handful of layered radial-gradient "particle"
 * fields (dust, snow, stars, clouds, aurora bands) over a base gradient, each
 * drifting slowly via a CSS keyframe animation on background-position. A
 * plain CSS background *value* (what formatCssBackground returns) can't
 * express motion — animation is a separate style property tied to a
 * @keyframes rule that has to exist in whatever document renders it — so
 * these are addressed by a "pattern:<name>" marker stored in the ordinary
 * flat `background` string field (it round-trips through everything that
 * already treats that field as an opaque CSS-ish string, including the
 * backend, with zero format changes) and resolved by applyResolvedBackground
 * below, which is the one place that knows how to turn the marker into the
 * actual layered background-image + animation. The @keyframes themselves are
 * defined once in style.css (in-app pages) and once in live.html's own
 * <style> block (a separate document with no access to style.css).
 */
export interface AnimatedBackgroundPattern {
    name: string;
    backgroundImage: string;
    backgroundSize: string;
    animationName: string;
    durationSec: number;
}

export const ANIMATED_BACKGROUND_PATTERNS: Record<string, AnimatedBackgroundPattern> = {
    'floating-dust': {
        name: 'Floating Dust',
        backgroundImage:
            'radial-gradient(circle at 20% 30%, rgba(255,235,180,0.9) 0px, rgba(255,235,180,0.9) 1.5px, transparent 2px), ' +
            'radial-gradient(circle at 60% 70%, rgba(255,235,180,0.7) 0px, rgba(255,235,180,0.7) 1px, transparent 1.6px), ' +
            'radial-gradient(circle at 80% 15%, rgba(255,245,210,0.8) 0px, rgba(255,245,210,0.8) 1.2px, transparent 1.8px), ' +
            'radial-gradient(circle at 35% 85%, rgba(255,235,180,0.6) 0px, rgba(255,235,180,0.6) 1px, transparent 1.5px), ' +
            'radial-gradient(circle at 90% 55%, rgba(255,245,210,0.7) 0px, rgba(255,245,210,0.7) 1.3px, transparent 1.9px), ' +
            'linear-gradient(160deg, #2b2013, #4a3820, #6b5227)',
        backgroundSize: '220px 220px, 180px 180px, 260px 260px, 200px 200px, 240px 240px, 100% 100%',
        animationName: 'bgPatternFloatingDust',
        durationSec: 50
    },
    'drifting-clouds': {
        name: 'Drifting Clouds',
        backgroundImage:
            'radial-gradient(ellipse 220px 90px at 20% 30%, rgba(255,255,255,0.35), transparent 70%), ' +
            'radial-gradient(ellipse 300px 110px at 65% 55%, rgba(255,255,255,0.28), transparent 70%), ' +
            'radial-gradient(ellipse 260px 100px at 40% 75%, rgba(255,255,255,0.22), transparent 70%), ' +
            'radial-gradient(ellipse 200px 80px at 85% 20%, rgba(255,255,255,0.3), transparent 70%), ' +
            'linear-gradient(180deg, #3a5a8a, #6683b0, #9db4d6)',
        backgroundSize: '1400px 800px, 1400px 800px, 1400px 800px, 1400px 800px, 100% 100%',
        animationName: 'bgPatternDriftingClouds',
        durationSec: 90
    },
    'gentle-snowfall': {
        name: 'Gentle Snowfall',
        backgroundImage:
            'radial-gradient(circle, rgba(255,255,255,0.9) 1px, transparent 1.6px), ' +
            'radial-gradient(circle, rgba(255,255,255,0.75) 1.2px, transparent 1.8px), ' +
            'radial-gradient(circle, rgba(255,255,255,0.6) 0.8px, transparent 1.3px), ' +
            'linear-gradient(180deg, #0d1b2a, #1b3a5c, #2c5480)',
        backgroundSize: '140px 140px, 200px 200px, 100px 100px, 100% 100%',
        animationName: 'bgPatternGentleSnowfall',
        durationSec: 25
    },
    'aurora-waves': {
        name: 'Aurora Waves',
        backgroundImage:
            'radial-gradient(ellipse 500px 250px at 30% 20%, rgba(0,255,157,0.35), transparent 70%), ' +
            'radial-gradient(ellipse 550px 280px at 70% 40%, rgba(123,47,247,0.3), transparent 70%), ' +
            'radial-gradient(ellipse 480px 240px at 50% 65%, rgba(0,200,255,0.28), transparent 70%), ' +
            'linear-gradient(180deg, #050814, #0a1128, #030509)',
        backgroundSize: '1600px 900px, 1600px 900px, 1600px 900px, 100% 100%',
        animationName: 'bgPatternAuroraWaves',
        durationSec: 70
    },
    'starfield-drift': {
        name: 'Starfield Drift',
        backgroundImage:
            'radial-gradient(circle, rgba(255,255,255,0.95) 0.8px, transparent 1.3px), ' +
            'radial-gradient(circle, rgba(255,255,255,0.7) 1px, transparent 1.6px), ' +
            'radial-gradient(circle, rgba(200,220,255,0.5) 0.6px, transparent 1px), ' +
            'radial-gradient(ellipse at center, #0a0e27, #000000)',
        backgroundSize: '160px 160px, 240px 240px, 100px 100px, 100% 100%',
        animationName: 'bgPatternStarfieldDrift',
        durationSec: 140
    }
};

export function parseAnimatedPatternMarker(bg: string | null | undefined): AnimatedBackgroundPattern | null {
    if (!bg || typeof bg !== 'string' || !bg.startsWith('pattern:')) return null;
    return ANIMATED_BACKGROUND_PATTERNS[bg.slice('pattern:'.length)] || null;
}

/**
 * Applies a resolved (already flat-string) background to an element,
 * transparently handling the "pattern:<name>" animated marker in addition to
 * everything formatCssBackground already understands (colors, gradients,
 * theme names, image paths). This is the one place that should ever set
 * background/backgroundImage/animation on a slide-rendering element — call
 * it instead of `el.style.background = formatCssBackground(...)` so an
 * element that previously had an animated pattern applied gets its
 * animation properly cleared when the background changes to something else.
 */
/**
 * Every entry in GRADIENT_PRESETS/ANIMATED_PATTERN_PRESETS happens to be a
 * `{ kind: 'Solid', data: <css-or-pattern-marker string> }` background (see
 * ANIMATED_PATTERN_PRESETS's own comment) -- but `BackgroundPreset.background`
 * is typed as the full `SlideBackground` union, so TS can't narrow it to
 * 'Solid' at a call site without a cast. This does the narrowing once,
 * falling back to '' for a preset that's ever changed to a non-Solid kind
 * instead of silently rendering `[object Object]`.
 */
export function backgroundPresetDataString(preset: BackgroundPreset): string {
    return preset.background.kind === 'Solid' ? preset.background.data : '';
}

export function applyResolvedBackground(
    el: HTMLElement,
    bg: string | null | undefined,
    defaultGradient: string = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)',
    availableThemes: ThemeDefinition[] = DEFAULT_THEMES
): void {
    const pattern = parseAnimatedPatternMarker(bg);
    if (pattern) {
        el.style.background = '';
        el.style.backgroundImage = pattern.backgroundImage;
        el.style.backgroundSize = pattern.backgroundSize;
        el.style.backgroundRepeat = 'repeat';
        el.style.animation = `${pattern.animationName} ${pattern.durationSec}s linear infinite`;
        return;
    }
    el.style.animation = '';
    el.style.backgroundImage = '';
    el.style.background = formatCssBackground(bg, defaultGradient, availableThemes);
}

/**
 * Resolves a slide's background onto `el`, preferring the structured
 * `background_v2` (Solid/Gradient/Image/Video) over the legacy flattened
 * `background` string. Unlike `applyResolvedBackground`, this is for contexts
 * that only have the raw, unresolved slide record (the Slide Editor filmstrip,
 * the Resources tab preview) rather than an engine-resolved `background` string
 * — Preview/Live already get the latter and don't need this.
 */
export function resolveSlideBackgroundElement(
    el: HTMLElement,
    slide: { background?: string | null; background_v2?: SlideBackground | null }
): void {
    const bgV2 = slide.background_v2;
    const legacyBg = slide.background;
    if (bgV2) {
        switch (bgV2.kind) {
            case 'Solid':
                // Also covers the "pattern:<name>" animated marker — gets the
                // real (tiny) drifting animation too.
                applyResolvedBackground(el, bgV2.data);
                break;
            case 'Gradient': {
                el.style.animation = '';
                el.style.backgroundImage = '';
                const { kind, stops, angle_deg } = bgV2.data;
                const stopStr = stops.map(s => `${s.color} ${s.offset * 100}%`).join(', ');
                el.style.background = kind === 'radial'
                    ? `radial-gradient(circle, ${stopStr})`
                    : `linear-gradient(${angle_deg ?? 180}deg, ${stopStr})`;
                break;
            }
            case 'Image':
                el.style.animation = '';
                el.style.backgroundImage = '';
                el.style.background = `url("${bgV2.data.file_path}") center/cover no-repeat`;
                break;
            case 'Video':
                el.style.animation = '';
                el.style.backgroundImage = '';
                el.style.background = '#0d1117';
                break;
            default:
                el.style.animation = '';
                el.style.backgroundImage = '';
                el.style.background = '#0a0a0c';
                break;
        }
    } else if (legacyBg) {
        if (isVideoBackground(legacyBg)) {
            el.style.animation = '';
            el.style.backgroundImage = '';
            el.style.background = '#0d1117';
        } else {
            applyResolvedBackground(el, legacyBg);
        }
    } else {
        applyResolvedBackground(el, null);
    }
}

/**
 * Fast synchronous scripture reference parser.
 */
export function parseScriptureReference(input: string | null | undefined): ScriptureReferenceResult {
    if (!input || !input.trim()) return { isReference: false };
    const str = input.trim();

    const m = str.match(/^([1-3]?\s*[a-zA-Z]+(?:\s+of\s+[a-zA-Z]+)?)\s*(\d+)?(?::(\d+)(?:-(\d+))?)?/);
    if (m && m[1]) {
        const bookGuess = m[1].trim();
        const ch = m[2] ? parseInt(m[2], 10) : null;
        const vs = m[3] ? parseInt(m[3], 10) : null;
        const ve = m[4] ? parseInt(m[4], 10) : vs;
        return {
            isReference: true,
            book: bookGuess,
            chapter: ch,
            verseStart: vs,
            verseEnd: ve,
            hasSpecificVerse: !!vs,
            formatted: str
        };
    }

    return { isReference: false };
}

/**
 * Maps single keyboard shortcut keys to standard liturgical song section codes.
 * Returns section prefix string or null if not a liturgical key.
 */
export function matchLiturgicalSectionKey(key: string): string | null {
    if (!key || typeof key !== 'string') return null;
    const lower = key.toLowerCase();
    const valid = ['v', 'c', 'b', 'e', 'p', 'i', 'o'];
    if (valid.includes(lower)) {
        return lower.toUpperCase();
    }
    return null;
}

/**
 * Sanitize strings against HTML injection
 */
const ESCAPE_HTML_MAP: Record<string, string> = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' };
const ESCAPE_HTML_RE = /[&<>"']/g;
// Called on every rendered slide badge/lyric/title on every WS message — one regex pass
// over the string with a replacer function does the same job as five sequential
// .replace() calls (each of which rescans the ENTIRE string, including characters the
// previous pass already handled) in a single scan.
export function escapeHtml(str: any): string {
    if (str === null || str === undefined) return '';
    return String(str).replace(ESCAPE_HTML_RE, (ch) => ESCAPE_HTML_MAP[ch]);
}

/**
 * Returns the URL unchanged if it is an absolute http(s) URL, else null. Use before
 * assigning data-derived values to navigable sinks (iframe.src, a.href) so that
 * `javascript:` / `data:` / `vbscript:` URLs are never loaded.
 */
export function safeHttpUrl(url: any): string | null {
    if (typeof url !== 'string') return null;
    try {
        const u = new URL(url.trim());
        return u.protocol === 'http:' || u.protocol === 'https:' ? u.href : null;
    } catch {
        return null;
    }
}

/**
 * Make a URL safe to embed inside a CSS `url('...')` that is itself placed in an
 * HTML `style="..."` attribute: percent-encodes quotes/parens/backslashes/whitespace/
 * control chars (so it cannot terminate the url() token) and HTML-escapes the result.
 */
export function escapeCssUrl(url: any): string {
    const encoded = String(url ?? '').replace(/['"()\\\s<>&\x00-\x1f]/g, (ch) => '%' + ch.charCodeAt(0).toString(16).toUpperCase().padStart(2, '0'));
    return escapeHtml(encoded);
}

/**
 * Computes destination index using Pragmatic DnD's getReorderDestinationIndex
 * with edge normalization for multi-directional / grid matrix layouts.
 */
export function computeSafeDestinationIndex(
    startIndex: number,
    indexOfTarget: number,
    closestEdge: Edge | null,
    axis: 'vertical' | 'horizontal' = 'vertical'
): number {
    if (!closestEdge) return indexOfTarget;
    let normalizedEdge = closestEdge;
    if (axis === 'horizontal') {
        if (closestEdge === 'bottom') normalizedEdge = 'right';
        else if (closestEdge === 'top') normalizedEdge = 'left';
    } else if (axis === 'vertical') {
        if (closestEdge === 'right') normalizedEdge = 'bottom';
        else if (closestEdge === 'left') normalizedEdge = 'top';
    }
    return getReorderDestinationIndex({ startIndex, indexOfTarget, closestEdgeOfTarget: normalizedEdge, axis });
}

export interface KeyboardEventLike {
    key: string;
    ctrlKey?: boolean;
    shiftKey?: boolean;
    altKey?: boolean;
    metaKey?: boolean;
    targetTagName?: string;
    isContentEditable?: boolean;
    isEditorOpen?: boolean;
}

export interface ResolvedKeyboardAction {
    action: 'search_focus' | 'close_modals' | 'undo' | 'redo' | 'guide' | 'shortcuts' |
            'options' | 'blackout' | 'clear_text' | 'logo' | 'alert' | 'go_live' |
            'next_slide' | 'prev_slide' | 'next_item' | 'prev_item' | 'jump_slide' |
            'jump_section' | 'new_schedule' | 'open_schedule' | 'save_schedule' | 'save_schedule_as' |
            'import_modal' | 'create_song' | 'delete_selected_item';
    command?: any;
    preventDefault?: boolean;
}

/**
 * Pure deterministic keyboard shortcut resolver matching the complete OpenSanctuary matrix.
 */
export function resolveKeyboardShortcut(e: KeyboardEventLike): ResolvedKeyboardAction | null {
    const isEditorOpen = e.isEditorOpen ?? (typeof document !== 'undefined' && document?.body?.classList?.contains('editor-open'));
    if (isEditorOpen) {
        if (e.key === 'Escape') {
            return { action: 'close_modals', preventDefault: false };
        }
        return null;
    }

    if (e.ctrlKey && e.key && e.key.toLowerCase() === 'f') {
        return { action: 'search_focus', preventDefault: true };
    }

    const tagName = (e.targetTagName || '').toLowerCase();
    if (['input', 'textarea', 'select'].includes(tagName) || e.isContentEditable) {
        if (e.key === 'Escape') {
            return { action: 'close_modals', preventDefault: false };
        }
        return null;
    }

    // Ctrl+Z: Undo
    if (e.ctrlKey && !e.shiftKey && e.key && e.key.toLowerCase() === 'z') {
        return { action: 'undo', command: 'Undo', preventDefault: true };
    }
    // Ctrl+Y or Ctrl+Shift+Z: Redo
    if ((e.ctrlKey && e.key && e.key.toLowerCase() === 'y') ||
        (e.ctrlKey && e.shiftKey && e.key && e.key.toLowerCase() === 'z')) {
        return { action: 'redo', command: 'Redo', preventDefault: true };
    }

    // F1: Schedule & Service Articles (User Guide)
    if (e.key === 'F1') {
        return { action: 'guide', preventDefault: true };
    }
    // ?: Keyboard Shortcuts Dialog
    if (e.key === '?') {
        return { action: 'shortcuts', preventDefault: true };
    }
    // F2: Options & Settings Dialog
    if (e.key === 'F2' || (e.ctrlKey && e.key === ',')) {
        return { action: 'options', preventDefault: true };
    }
    // F5 / Ctrl+B: Blackout Toggle
    if (e.key === 'F5' || (e.ctrlKey && e.key && e.key.toLowerCase() === 'b')) {
        return { action: 'blackout', command: 'ToggleBlackout', preventDefault: true };
    }
    // F6 / Ctrl+Shift+C: Clear Text Toggle
    if (e.key === 'F6' || (e.ctrlKey && e.shiftKey && e.key && e.key.toLowerCase() === 'c')) {
        return { action: 'clear_text', command: 'ToggleClearText', preventDefault: true };
    }
    // F7 / Ctrl+L: Logo Toggle
    if (e.key === 'F7' || (e.ctrlKey && e.key && e.key.toLowerCase() === 'l')) {
        return { action: 'logo', command: 'ToggleLogo', preventDefault: true };
    }
    // F8: Alert Modal
    if (e.key === 'F8') {
        return { action: 'alert', preventDefault: true };
    }
    // Enter / PageDown: Go Live
    if (e.key === 'Enter' || e.key === 'PageDown') {
        return {
            action: 'go_live',
            command: { GoLive: { item_index: null, slide_index: null } },
            preventDefault: true
        };
    }
    // Space / Right Arrow: Next Slide
    if (e.key === ' ' || e.key === 'ArrowRight') {
        return { action: 'next_slide', command: 'NextSlide', preventDefault: true };
    }
    // Left Arrow: Prev Slide (Backspace is claimed below for delete-selected-item)
    if (e.key === 'ArrowLeft') {
        return { action: 'prev_slide', command: 'PrevSlide', preventDefault: true };
    }
    // Down Arrow: Next Schedule Item
    if (e.key === 'ArrowDown') {
        return { action: 'next_item', command: 'NextItem', preventDefault: true };
    }
    // Up Arrow: Prev Schedule Item
    if (e.key === 'ArrowUp') {
        return { action: 'prev_item', command: 'PrevItem', preventDefault: true };
    }
    // Number keys 1..9: Direct slide jump
    if (!e.ctrlKey && !e.altKey && !e.metaKey && e.key && e.key >= '1' && e.key <= '9') {
        const num = parseInt(e.key, 10) - 1;
        return { action: 'jump_slide', command: { JumpSlide: num }, preventDefault: false };
    }
    // Liturgical section jumps (V, C, B, E, P, I, O) - only when unmodified by Ctrl/Alt/Meta
    if (!e.ctrlKey && !e.altKey && !e.metaKey && e.key) {
        const section = matchLiturgicalSectionKey(e.key);
        if (section) {
            return { action: 'jump_section', command: { JumpSection: section }, preventDefault: false };
        }
    }
    // Ctrl+N: New Schedule
    if (e.ctrlKey && !e.shiftKey && e.key && e.key.toLowerCase() === 'n') {
        return { action: 'new_schedule', command: 'NewSchedule', preventDefault: true };
    }
    // Ctrl+O: Open Schedule
    if (e.ctrlKey && e.key && e.key.toLowerCase() === 'o') {
        return { action: 'open_schedule', preventDefault: true };
    }
    // Ctrl+S: quick-save with the last title+format. Ctrl+Shift+S: always
    // prompt via the Save As modal -- previously indistinguishable, both
    // fell into the same 'save_schedule' action, which app_ui.ts's dispatch
    // pointed at the modal, so plain Ctrl+S never actually quick-saved.
    if (e.ctrlKey && e.key && e.key.toLowerCase() === 's') {
        return { action: e.shiftKey ? 'save_schedule_as' : 'save_schedule', preventDefault: true };
    }
    // Ctrl+I: Import Modal
    if (e.ctrlKey && e.key && e.key.toLowerCase() === 'i') {
        return { action: 'import_modal', preventDefault: true };
    }
    // Ctrl+Shift+N: New Slide / Song
    if (e.ctrlKey && e.shiftKey && e.key && e.key.toLowerCase() === 'n') {
        return { action: 'create_song', preventDefault: true };
    }
    // Escape: Close all modals & context menu
    if (e.key === 'Escape') {
        return { action: 'close_modals', preventDefault: false };
    }
    // Delete / Backspace: Delete selected item from schedule, preview, or live with undo
    // placeholder. Backspace lives here (not on Prev Slide, despite matching most
    // presentation software) because that's the behavior already shipped and relied on.
    if (e.key === 'Delete' || e.key === 'Backspace') {
        return { action: 'delete_selected_item', preventDefault: true };
    }

    return null;
}

/**
 * Deduplication state for rapid schedule item additions (e.g. multi-listener drop race conditions).
 * Prevents identical items from being added multiple times within a short time window (default 400ms).
 */
export class ScheduleAddGuard {
    private lastAddedId: string = '';
    private lastAddedTime: number = 0;
    private readonly windowMs: number;

    constructor(windowMs: number = 400) {
        this.windowMs = windowMs;
    }

    shouldAllow(itemId: string, now: number = Date.now()): boolean {
        if (this.lastAddedId === itemId && (now - this.lastAddedTime) < this.windowMs) {
            return false;
        }
        this.lastAddedId = itemId;
        this.lastAddedTime = now;
        return true;
    }

    reset() {
        this.lastAddedId = '';
        this.lastAddedTime = 0;
    }
}

/**
 * Determines whether a drop target should process the drop, preventing nested drop target double-processing.
 * In Pragmatic DnD, location.current.dropTargets contains targets ordered from innermost to outermost.
 */
export function isPrimaryDropTarget(targetElement: any, dropTargets: Array<{ element: any }>): boolean {
    if (!dropTargets || dropTargets.length === 0) return true;
    return dropTargets[0].element === targetElement;
}

/**
 * Resolves the target item index to delete given active context, schedule items, and state.
 * Supports Schedule, Preview, and Live selection contexts.
 */
export function resolveTargetItemToDelete(params: {
    activeContext: 'schedule' | 'preview' | 'live';
    scheduleItems: Array<{ id: string; title?: string }>;
    selectedItemIndex?: number | null;
    expandedScheduleIndex?: number | null;
    stagedItemId?: string | null;
    liveItemId?: string | null;
}): { index: number; title: string } | null {
    const { activeContext, scheduleItems, selectedItemIndex, expandedScheduleIndex, stagedItemId, liveItemId } = params;
    if (!scheduleItems || scheduleItems.length === 0) return null;

    let targetIdx = -1;

    // 1. Check active deck context first
    if (activeContext === 'preview' && stagedItemId) {
        targetIdx = scheduleItems.findIndex(it => it.id === stagedItemId);
    } else if (activeContext === 'live' && liveItemId) {
        targetIdx = scheduleItems.findIndex(it => it.id === liveItemId);
    }

    // 2. Fallback to schedule's selection or expanded row
    if (targetIdx === -1) {
        if (selectedItemIndex !== null && selectedItemIndex !== undefined && selectedItemIndex >= 0 && selectedItemIndex < scheduleItems.length) {
            targetIdx = selectedItemIndex;
        } else if (expandedScheduleIndex !== null && expandedScheduleIndex !== undefined && expandedScheduleIndex >= 0 && expandedScheduleIndex < scheduleItems.length) {
            targetIdx = expandedScheduleIndex;
        } else if (stagedItemId) {
            targetIdx = scheduleItems.findIndex(it => it.id === stagedItemId);
        } else if (liveItemId) {
            targetIdx = scheduleItems.findIndex(it => it.id === liveItemId);
        }
    }

    if (targetIdx >= 0 && targetIdx < scheduleItems.length) {
        return { index: targetIdx, title: scheduleItems[targetIdx].title || 'Item' };
    }

    return null;
}

export interface UndoPlaceholderInfo {
    index: number;
    title: string;
    createdAt: number;
    durationMs: number;
}

/**
 * Manages the state and lifecycle of the 1-second undo placeholder.
 */
export class UndoPlaceholderManager {
    private current: UndoPlaceholderInfo | null = null;
    private readonly defaultDurationMs: number;

    constructor(defaultDurationMs: number = 1500) {
        this.defaultDurationMs = defaultDurationMs;
    }

    create(index: number, title: string, now: number = Date.now(), durationMs?: number): UndoPlaceholderInfo {
        this.current = {
            index,
            title,
            createdAt: now,
            durationMs: durationMs ?? this.defaultDurationMs
        };
        return this.current;
    }

    get(): UndoPlaceholderInfo | null {
        return this.current;
    }

    isExpired(now: number = Date.now()): boolean {
        if (!this.current) return true;
        return (now - this.current.createdAt) >= this.current.durationMs;
    }

    clear(): void {
        this.current = null;
    }
}

/**
 * Resolves and preserves permanent slide badges/tags across reordering and edits.
 * Handles scriptures (Genesis 1:1 -> V1), songs (Verse 2 -> V2, Chorus 1 -> C1, Bridge -> B),
 * and assigns permanent badges to slides without badges so reordering never changes verse numbers.
 */
export function getSlideBadge(slide: any, sIdx?: number): string {
    if (!slide) return sIdx !== undefined ? `V${sIdx + 1}` : 'Slide';

    // 1. If explicit cached tag or tag already assigned, preserve it!
    if (slide.tag && typeof slide.tag === 'string' && slide.tag.trim()) {
        return slide.tag.trim();
    }

    // 2. If header looks like a tag (e.g. "V1", "C1", "B", "E1") or has scripture verse (e.g. "Genesis 1:15")
    if (slide.header && typeof slide.header === 'string') {
        const h = slide.header.trim();
        if (/^[vcbepio]\d*$/i.test(h)) {
            slide.tag = h.toUpperCase();
            return slide.tag;
        }
        const verseMatch = h.match(/:(\d+)(?:-(\d+))?/);
        if (verseMatch) {
            slide.tag = `V${verseMatch[1]}`;
            return slide.tag;
        }
    }

    // 3. If label is present, extract or format a badge
    if (slide.label && typeof slide.label === 'string') {
        const l = slide.label.trim();
        if (/^[vcbepio]\d*$/i.test(l)) {
            slide.tag = l.toUpperCase();
            return slide.tag;
        }
        const verseMatch = l.match(/verse\s*(\d+)/i);
        if (verseMatch) {
            slide.tag = `V${verseMatch[1]}`;
            return slide.tag;
        }
        const chorusMatch = l.match(/chorus\s*(\d*)/i);
        if (chorusMatch) {
            slide.tag = chorusMatch[1] ? `C${chorusMatch[1]}` : 'C';
            return slide.tag;
        }
        const bridgeMatch = l.match(/bridge\s*(\d*)/i);
        if (bridgeMatch) {
            slide.tag = bridgeMatch[1] ? `B${bridgeMatch[1]}` : 'B';
            return slide.tag;
        }
        if (/ending|outro/i.test(l)) {
            slide.tag = 'E';
            return slide.tag;
        }
        if (l.length <= 6) {
            slide.tag = l;
            return slide.tag;
        }
    }

    // 4. If text starts with a verse number (e.g. "1. In the beginning..." or "15 In the...")
    if (slide.text && typeof slide.text === 'string') {
        const textVerseMatch = slide.text.trim().match(/^(\d+)[\.\s]/);
        if (textVerseMatch) {
            slide.tag = `V${textVerseMatch[1]}`;
            return slide.tag;
        }
    }

    // 5. Fallback to index if provided, BUT store it on slide.tag so reordering NEVER changes it later!
    if (sIdx !== undefined && sIdx >= 0) {
        slide.tag = `V${sIdx + 1}`;
        return slide.tag;
    }

    return 'Slide';
}

export function getSlideBadgeColor(badge: string): string {
    const upper = (badge || '').toUpperCase();
    if (upper.startsWith('C')) return 'var(--badge-chorus)';
    if (upper.startsWith('B')) return 'var(--badge-bridge)';
    if (upper.startsWith('E')) return 'var(--badge-ending)';
    return 'var(--badge-verse)';
}

/**
 * Result of parsing a dual-column parallel slide.
 */
export interface ParallelSlideContent {
    isParallel: boolean;
    leftText: string;
    rightText: string;
    primaryText: string;
    secondaryText: string;
    primaryHeader?: string;
    secondaryHeader?: string;
}

function extractHeaderAndBody(col: string): { header?: string; body: string } {
    const trimmed = col.trim();
    const match = trimmed.match(/^\[([^\]]+)\]\n?([\s\S]*)$/);
    if (match) {
        return { header: match[1].trim(), body: match[2].trim() };
    }
    return { header: undefined, body: trimmed };
}

/**
 * Parses slide content for parallel / dual translation presentation using the '|||' column delimiter.
 * Returns { isParallel: true, leftText, rightText, primaryText, secondaryText, primaryHeader, secondaryHeader } when '|||' is present,
 * otherwise { isParallel: false, leftText: text, rightText: '', primaryText: text, secondaryText: '' }.
 */
export function parseParallelSlide(text: string | null | undefined): ParallelSlideContent {
    if (!text || typeof text !== 'string') {
        return { isParallel: false, leftText: '', rightText: '', primaryText: '', secondaryText: '' };
    }
    if (text.includes('|||')) {
        const parts = text.split('|||');
        const left = (parts[0] || '').trim();
        const right = (parts[1] || '').trim();
        const leftExtracted = extractHeaderAndBody(left);
        const rightExtracted = extractHeaderAndBody(right);
        return {
            isParallel: true,
            leftText: left,
            rightText: right,
            primaryText: leftExtracted.body,
            secondaryText: rightExtracted.body,
            primaryHeader: leftExtracted.header,
            secondaryHeader: rightExtracted.header,
        };
    }
    return {
        isParallel: false,
        leftText: text.trim(),
        rightText: '',
        primaryText: text.trim(),
        secondaryText: '',
    };
}

/**
 * Formats primary and secondary Bible verse text into the canonical dual-column slide format separated by '|||'.
 */
export function formatParallelSlide(
    primaryHeader: string,
    primaryText: string,
    secondaryHeader: string,
    secondaryText: string
): string {
    const pContent = primaryHeader ? `[${primaryHeader}]\n${primaryText}` : primaryText;
    const sContent = secondaryHeader ? `[${secondaryHeader}]\n${secondaryText}` : secondaryText;
    return `${pContent}\n|||\n${sContent}`;
}

/**
 * Compares two translation identifiers, abbreviations, or display names for equivalence,
 * case-insensitively and ignoring punctuation/whitespace/parentheses.
 */
export function areTranslationsEquivalent(t1: string | null | undefined, t2: string | null | undefined): boolean {
    if (!t1 || !t2) return false;
    const s1 = String(t1).toLowerCase().replace(/[\s\-_()]/g, '');
    const s2 = String(t2).toLowerCase().replace(/[\s\-_()]/g, '');
    if (s1 === s2) return true;

    // World English Bible / WEB
    const isWeb1 = s1.includes('worldenglish') || s1 === 'web';
    const isWeb2 = s2.includes('worldenglish') || s2 === 'web';
    if (isWeb1 && isWeb2) return true;

    // King James Version / KJV (handling Red Letter distinction)
    const isKjv1 = s1.includes('kingjames') || s1.includes('kjv');
    const isKjv2 = s2.includes('kingjames') || s2.includes('kjv');
    if (isKjv1 && isKjv2) {
        const red1 = s1.includes('redletter') || s1.includes('rl');
        const red2 = s2.includes('redletter') || s2.includes('rl');
        return red1 === red2;
    }

    // American Standard Version / ASV
    const isAsv1 = s1.includes('americanstandard') || s1.includes('asv');
    const isAsv2 = s2.includes('americanstandard') || s2.includes('asv');
    if (isAsv1 && isAsv2) return true;

    // Bible in Basic English / BBE
    const isBbe1 = s1.includes('basicenglish') || s1 === 'bbe';
    const isBbe2 = s2.includes('basicenglish') || s2 === 'bbe';
    if (isBbe1 && isBbe2) return true;

    // Holman Christian Standard / HCSB
    const isHcsb1 = s1.includes('holman') || s1.includes('hcsb');
    const isHcsb2 = s2.includes('holman') || s2.includes('hcsb');
    if (isHcsb1 && isHcsb2) return true;

    return false;
}

/**
 * Constructs a pairing URL for the mobile phone pairing scanner.
 * Enforces HTTPS / Secure Context when https_enabled is true, allowing
 * mobile browsers (iOS Safari, Android Chrome) to access camera APIs.
 */
export function buildPairingUrl(
  info: { https_enabled?: boolean; https_port?: number | null; http_port?: number; port?: number; lan_ip?: string; public_https_url?: string | null },
  currentLocation: { protocol?: string; hostname?: string; port?: string },
  sessionToken?: string | null
): string {
  const tokenParam = sessionToken ? `?key=${encodeURIComponent(sessionToken)}` : '';

  // A configured Public HTTPS URL (docs/TUNNELS.md) always wins: a real,
  // browser-trusted cert removes any ambiguity around camera-API
  // secure-context requirements on stricter mobile browsers, which is the
  // whole reason this function prefers HTTPS at all.
  if (info.public_https_url) {
    return `${info.public_https_url}/pairing.html${tokenParam}`;
  }

  const host = currentLocation.hostname || '127.0.0.1';
  const activeHost = (host === 'localhost' || host === '127.0.0.1')
    ? (info.lan_ip || host)
    : host;

  let protocol = currentLocation.protocol || 'http:';
  let activePort = currentLocation.port || '8080';

  if (info.https_enabled && info.https_port) {
    protocol = 'https:';
    activePort = String(info.https_port);
  } else {
    activePort = currentLocation.port || (info.http_port ? String(info.http_port) : (info.port ? String(info.port) : '8080'));
  }

  return `${protocol}//${activeHost}:${activePort}/pairing.html${tokenParam}`;
}

/**
 * Constructs the Mobile Remote QR's base URL (before `app_ui.ts` appends the
 * `#token=...` fragment once a device token is minted). Same HTTPS/WSS
 * preference as `buildPairingUrl` above, for the same reason: without it,
 * this used to just mirror `currentLocation.protocol`, so a console viewed
 * over the default plaintext `http://…:8080/` URL generated an `http://`
 * remote QR -- meaning `device_token`, a static bearer credential sent on
 * every command the remote issues, traveled in cleartext over the LAN by
 * default, replayable by anyone who sniffed one packet.
 */
export function buildRemoteUrl(
  info: { https_enabled?: boolean; https_port?: number | null; http_port?: number; port?: number; lan_ip?: string; public_https_url?: string | null },
  currentLocation: { protocol?: string; hostname?: string; port?: string }
): string {
  // Same Public HTTPS URL preference as buildPairingUrl above, for the same
  // reason: a real, browser-trusted cert instead of a self-signed warning.
  if (info.public_https_url) {
    return `${info.public_https_url}/remote`;
  }

  const host = currentLocation.hostname || '127.0.0.1';
  const activeHost = (host === 'localhost' || host === '127.0.0.1')
    ? (info.lan_ip || host)
    : host;

  let protocol = currentLocation.protocol || 'http:';
  let activePort = currentLocation.port || '8080';

  if (info.https_enabled && info.https_port) {
    protocol = 'https:';
    activePort = String(info.https_port);
  } else {
    activePort = currentLocation.port || (info.http_port ? String(info.http_port) : (info.port ? String(info.port) : '8080'));
  }

  return `${protocol}//${activeHost}:${activePort}/remote`;
}


