/**
 * Themes: UI icon sets (web/icons/<id>/<name>.svg, see web/icons/README.md) and the
 * interface color theme that goes with each one.
 *
 * Icons in the markup are `<img data-icon="save" src="icons/orange/save.svg">`;
 * `applyIconSet` re-points every such image at the chosen set. `applyColorTheme`
 * overrides the `--os-brand-*` CSS variables that the stylesheet's accents are built
 * from. The two are chosen separately (`iconSet`, `colorTheme`) or, by default, as one
 * cohesive theme (`themeLinked`): `resolveTheme` is the single place that decides which
 * pair of ids is in effect. All three live in the app options (strings, like every
 * setting the server stores).
 */

export interface ThemeColors {
  /** Selection bars, primary buttons, focus rings (--os-brand-primary). */
  primary: string;
  /** Highlights and secondary accents (--os-brand-amber). */
  highlight: string;
  /** Pressed/deep accent (--os-brand-flame). */
  deep: string;
  /** Gradient start / lighter primary (--os-brand-bright). */
  bright: string;
}

export interface IconSetDef {
  id: string;
  label: string;
  /** Main, highlight and accent tones of the icons -- for the picker's swatch. */
  tones: [string, string, string];
  /** The matching interface colors. White text must stay readable on `primary`. */
  colors: ThemeColors;
}

export const ICON_SETS: IconSetDef[] = [
  { id: 'orange',  label: 'Orange (default)', tones: ['#FF5722', '#FFA726', '#D84315'],
    colors: { primary: '#FF5722', highlight: '#FFA726', deep: '#D84315', bright: '#FF7043' } },
  { id: 'slate',   label: 'Slate',            tones: ['#38bdf8', '#bae6fd', '#0284c7'],
    colors: { primary: '#0284c7', highlight: '#38bdf8', deep: '#0369a1', bright: '#0ea5e9' } },
  { id: 'gold',    label: 'Gold',             tones: ['#eab308', '#fde68a', '#ca8a04'],
    colors: { primary: '#b58105', highlight: '#eab308', deep: '#a16207', bright: '#facc15' } },
  { id: 'white',   label: 'White',            tones: ['#e2e8f0', '#ffffff', '#94a3b8'],
    colors: { primary: '#64748b', highlight: '#e2e8f0', deep: '#475569', bright: '#94a3b8' } },
  { id: 'crimson', label: 'Crimson',          tones: ['#f43f5e', '#fda4af', '#e11d48'],
    colors: { primary: '#e11d48', highlight: '#fb7185', deep: '#9f1239', bright: '#f43f5e' } },
];

export const DEFAULT_ICON_SET = 'orange';

/** Icons shown in each picker card's preview. */
export const ICON_SET_PREVIEW_ICONS = ['new-doc', 'open', 'save', 'golive', 'alert', 'logo', 'blackout', 'live-status'];

/** Unknown or missing ids (a stale saved value, a removed set) fall back to the default. */
export function normalizeIconSet(id: unknown): string {
  return typeof id === 'string' && ICON_SETS.some(s => s.id === id) ? id : DEFAULT_ICON_SET;
}

/** Color themes are named after the icon set they pair with, so the ids are shared. */
export const normalizeColorTheme = normalizeIconSet;

export function iconUrl(setId: unknown, name: string): string {
  return `icons/${normalizeIconSet(setId)}/${name}.svg`;
}

export function getThemeColors(id: unknown): ThemeColors {
  const normalized = normalizeColorTheme(id);
  return ICON_SETS.find(s => s.id === normalized)!.colors;
}

export interface ThemeOptions {
  iconSet?: unknown;
  colorTheme?: unknown;
  themeLinked?: unknown;
}

/** Linked is the default; only an explicit 'false' turns it off. */
export function isThemeLinked(options: ThemeOptions): boolean {
  return options.themeLinked !== 'false' && options.themeLinked !== false;
}

/** The icon set and color theme in effect: one cohesive theme when linked, else each as saved. */
export function resolveTheme(options: ThemeOptions): { iconSet: string; colorTheme: string; linked: boolean } {
  const iconSet = normalizeIconSet(options.iconSet);
  const linked = isThemeLinked(options);
  const colorTheme = linked || options.colorTheme == null || options.colorTheme === ''
    ? iconSet
    : normalizeColorTheme(options.colorTheme);
  return { iconSet, colorTheme, linked };
}

/** "#rrggbb" -> "r, g, b" for `rgba(var(--x-rgb), a)`. */
export function hexToRgbTriplet(hex: string): string {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!m) throw new Error(`Not a #rrggbb color: ${hex}`);
  return `${parseInt(m[1], 16)}, ${parseInt(m[2], 16)}, ${parseInt(m[3], 16)}`;
}

let currentIconSet = DEFAULT_ICON_SET;

/** The set most recently applied -- icons created later (see emoji_icons.ts) start out in it. */
export function getCurrentIconSet(): string {
  return currentIconSet;
}

/** Points every `img[data-icon]` under `root` at `setId`. */
export function applyIconSet(setId: unknown, root: ParentNode = document): void {
  const id = normalizeIconSet(setId);
  currentIconSet = id;
  root.querySelectorAll<HTMLImageElement>('img[data-icon]').forEach(img => {
    const url = iconUrl(id, img.dataset.icon || '');
    if (img.getAttribute('src') !== url) img.setAttribute('src', url);
  });
  if (typeof document !== 'undefined') document.documentElement.dataset.iconSet = id;
}

/** Sets the `--os-brand-*` variables (and their rgb triplets for translucent uses) on `<html>`. */
export function applyColorTheme(themeId: unknown, root: HTMLElement = document.documentElement): void {
  const id = normalizeColorTheme(themeId);
  const c = getThemeColors(id);
  const set = (name: string, value: string) => root.style.setProperty(name, value);
  set('--os-brand-primary', c.primary);
  set('--os-brand-amber', c.highlight);
  set('--os-brand-flame', c.deep);
  set('--os-brand-bright', c.bright);
  set('--os-brand-primary-rgb', hexToRgbTriplet(c.primary));
  set('--os-brand-amber-rgb', hexToRgbTriplet(c.highlight));
  set('--os-brand-glow', `rgba(${hexToRgbTriplet(c.primary)}, 0.14)`);
  root.dataset.colorTheme = id;
}

/** Applies whatever `resolveTheme(options)` says; the one call the app makes on load and save. */
export function applyTheme(options: ThemeOptions): void {
  const t = resolveTheme(options);
  applyIconSet(t.iconSet);
  applyColorTheme(t.colorTheme);
}
