/**
 * UI icon sets (web/icons/<id>/<name>.svg, see web/icons/README.md).
 *
 * Icons in the markup are `<img data-icon="save" src="icons/orange/save.svg">`;
 * `applyIconSet` re-points every such image at the chosen set, so a set is
 * switched without re-rendering anything. The choice lives in the `iconSet`
 * app option (saved with the rest of the settings).
 */

export interface IconSetDef {
  id: string;
  label: string;
  /** Main, highlight and accent tones -- for the picker's swatch. */
  tones: [string, string, string];
}

export const ICON_SETS: IconSetDef[] = [
  { id: 'orange',  label: 'Orange (default)', tones: ['#FF5722', '#FFA726', '#D84315'] },
  { id: 'slate',   label: 'Slate',            tones: ['#38bdf8', '#bae6fd', '#0284c7'] },
  { id: 'gold',    label: 'Gold',             tones: ['#eab308', '#fde68a', '#ca8a04'] },
  { id: 'white',   label: 'White',            tones: ['#e2e8f0', '#ffffff', '#94a3b8'] },
  { id: 'crimson', label: 'Crimson',          tones: ['#f43f5e', '#fda4af', '#e11d48'] },
];

export const DEFAULT_ICON_SET = 'orange';

/** Icons shown in each picker card's preview. */
export const ICON_SET_PREVIEW_ICONS = ['new-doc', 'open', 'save', 'golive', 'alert', 'logo', 'blackout', 'live-status'];

/** Unknown or missing ids (a stale saved value, a removed set) fall back to the default. */
export function normalizeIconSet(id: unknown): string {
  return typeof id === 'string' && ICON_SETS.some(s => s.id === id) ? id : DEFAULT_ICON_SET;
}

export function iconUrl(setId: unknown, name: string): string {
  return `icons/${normalizeIconSet(setId)}/${name}.svg`;
}

/** Points every `img[data-icon]` under `root` at `setId`. */
export function applyIconSet(setId: unknown, root: ParentNode = document): void {
  const id = normalizeIconSet(setId);
  root.querySelectorAll<HTMLImageElement>('img[data-icon]').forEach(img => {
    const url = iconUrl(id, img.dataset.icon || '');
    if (img.getAttribute('src') !== url) img.setAttribute('src', url);
  });
  if (typeof document !== 'undefined') document.documentElement.dataset.iconSet = id;
}
