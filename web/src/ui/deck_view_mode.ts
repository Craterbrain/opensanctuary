/**
 * Which view the Preview and Live decks open in: the grid (⊞, each slide as a rendered thumbnail) or the
 * list (☰, one text row per slide).
 *
 * Songs and scriptures are text, and read best as a list, so they open in the list; everything else
 * (presentations, media, headers) is visual and opens in the grid. Clicking ⊞ or ☰ overrides that for
 * the kind of item being shown (text or visual) on that deck, and the choice is remembered, so a
 * different song or scripture opens the same way. Per deck: Preview and Live are independent.
 */
export type DeckMode = 'matrix' | 'list';
export type DeckName = 'preview' | 'live';
export type ItemKind = 'text' | 'visual';

const STORAGE_KEY = 'os_deck_view_modes';

export function itemKind(itemType: string | null | undefined): ItemKind {
  const t = (itemType || '').toLowerCase();
  return t === 'song' || t === 'scripture' ? 'text' : 'visual';
}

/** What a deck opens in for this kind of item when nothing has been chosen. */
export function defaultDeckMode(itemType: string | null | undefined): DeckMode {
  return itemKind(itemType) === 'text' ? 'list' : 'matrix';
}

let choices: Record<string, DeckMode> | null = null;

function load(): Record<string, DeckMode> {
  if (choices) return choices;
  choices = {};
  try {
    const raw = typeof localStorage !== 'undefined' ? localStorage.getItem(STORAGE_KEY) : null;
    const parsed = raw ? JSON.parse(raw) : {};
    for (const [k, v] of Object.entries(parsed)) if (v === 'matrix' || v === 'list') choices[k] = v;
  } catch (_) { /* storage blocked or unreadable: defaults only */ }
  return choices;
}

/** The mode this deck shows for this item: the operator's choice for its kind, else the default. */
export function deckModeFor(deck: DeckName, itemType: string | null | undefined): DeckMode {
  return load()[`${deck}:${itemKind(itemType)}`] ?? defaultDeckMode(itemType);
}

/** Remembers what the operator picked for this kind of item on this deck. */
export function rememberDeckMode(deck: DeckName, itemType: string | null | undefined, mode: DeckMode): void {
  load()[`${deck}:${itemKind(itemType)}`] = mode;
  try {
    if (typeof localStorage !== 'undefined') localStorage.setItem(STORAGE_KEY, JSON.stringify(choices));
  } catch (_) { /* not persisted this time */ }
}

/** For tests: forget every choice (in memory and in storage). */
export function resetDeckModesForTests(): void {
  choices = {};
  try { if (typeof localStorage !== 'undefined') localStorage.removeItem(STORAGE_KEY); } catch (_) { /* ignore */ }
}
