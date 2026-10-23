/**
 * Shared visual building blocks for the Preview and Live slide-deck
 * matrices — the row of numbered slide cards under each panel's main
 * viewport. Both decks look the same (a badge + lyrics card, an empty
 * state, an undo-delete placeholder); what genuinely differs between them
 * is the interaction model — Preview: click-to-stage, double-click-to-Live,
 * drag-to-reorder; Live: click-to-jump-slide, no reorder — which stays in
 * app_core.ts's renderPreviewDeck/renderLiveDeck. Those two call into this
 * module for the shared markup instead of each keeping its own copy, so a
 * design change (spacing, wording, the badge/lyrics layout) is made once
 * here and both panels pick it up.
 *
 * Deliberately free of business logic (no sendCommand, no schedule/state
 * reads) — everything it needs (badge text/color, lyrics HTML, active
 * state) is computed by the caller and passed in, so this module never has
 * to know which panel is calling it.
 */

import { escapeUserHtml, type ThemeDefinition } from '../core/presentation_helpers.ts';
import { renderSlideVisual } from '../core/slide_render.ts';

export interface SlideCardOptions {
  badge: string;
  badgeColor: string;
  lyricsHtml: string;
  isActive: boolean;
  /** CSS class applied when isActive — 'active' for Live, 'active-staged' for Preview. */
  activeClass: string;
  /** Preview's cards get a drag handle for reordering; Live's don't (no live reorder). */
  showDragHandle: boolean;
  /** Extra class for panel-specific styling/selectors, e.g. 'preview-slide-card'. */
  extraClass?: string;
}

export function buildSlideCardElement(opts: SlideCardOptions): HTMLElement {
  const cardEl = document.createElement('div');
  cardEl.className = `slide-card ${opts.extraClass || ''} ${opts.isActive ? opts.activeClass : ''}`.trim();
  cardEl.innerHTML = `
    <div class="slide-badge-column">
      ${opts.showDragHandle
        ? '<span class="preview-card-drag-handle" title="Drag to reorder slide" role="button" tabindex="0" aria-label="Reorder slide. Press Alt+Left or Alt+Right to move.">⠿</span>'
        : ''}
      <span class="slide-badge-pill" style="color: ${opts.badgeColor};">${opts.badge}</span>
    </div>
    <div class="slide-lyrics">${opts.lyricsHtml}</div>
  `;
  return cardEl;
}

export interface DeckEmptyStateOptions {
  /** Present when the deck should show "Removed X — Undo" instead of the generic empty state. */
  undoPlaceholder: { title: string } | null;
  onUndo: () => void;
  emptyTitle: string;
  emptySubtitle: string;
  escapeHtml: (s: string) => string;
}

export function renderDeckEmptyState(matrixEl: HTMLElement, opts: DeckEmptyStateOptions): void {
  if (opts.undoPlaceholder) {
    matrixEl.innerHTML = `
      <div class="deck-undo-placeholder">
        <div class="undo-content">
          <span style="font-size: 12px;">🗑️</span>
          <span>Removed "<strong>${escapeUserHtml(opts.undoPlaceholder.title)}</strong>"</span>
        </div>
        <button class="btn btn-undo" type="button" title="Undo delete (Ctrl+Z)">↩ Undo</button>
      </div>`;
    const btn = matrixEl.querySelector('.btn-undo');
    if (btn) btn.addEventListener('click', opts.onUndo);
    return;
  }
  matrixEl.innerHTML = `
    <div class="deck-empty-state">
      <p class="deck-empty-title">${opts.emptyTitle}</p>
      <p class="deck-empty-subtitle">${opts.emptySubtitle}</p>
    </div>`;
}

/**
 * How a Preview/Live deck draws its slides. The deck's grid view shows each slide as it will look
 * (a rendered thumbnail) for the slides that are text on a background: songs, scriptures and
 * presentations. Everything else (media, headers) has no text slide to show, so keeps the text
 * cards. The deck's own list view always uses text cards -- that is the override.
 */
export type SlideDeckStyle = 'cards' | 'thumbnails';

const THUMBNAIL_ITEM_TYPES = new Set(['song', 'scripture', 'presentation']);

export function slideDeckStyle(itemType: string | null | undefined, listMode: boolean): SlideDeckStyle {
  return !listMode && THUMBNAIL_ITEM_TYPES.has((itemType || '').toLowerCase()) ? 'thumbnails' : 'cards';
}

export interface SlideThumbOptions {
  slide: any;
  theme: ThemeDefinition | null;
  badge: string;
  badgeColor: string;
  isActive: boolean;
  /** CSS class applied when isActive -- 'active' for Live, 'active-staged' for Preview. */
  activeClass: string;
  showDragHandle: boolean;
  extraClass?: string;
  /** Paints the slide's resolved background (image, gradient, video poster) onto the thumbnail. */
  applyBackground: (el: HTMLElement) => void;
}

/**
 * A slide card that shows the slide itself: the same renderer as the Preview/Live canvas
 * (renderSlideVisual), scaled into a 16:9 box with the badge and drag handle overlaid.
 * Still a `.slide-card`, so active-state toggling and the deck's click/drag wiring work
 * unchanged. The slide is drawn once the card has a width (and again if the grid resizes).
 */
export function buildSlideThumbElement(opts: SlideThumbOptions): HTMLElement {
  const cardEl = document.createElement('div');
  cardEl.className = `slide-card slide-thumb-card ${opts.extraClass || ''} ${opts.isActive ? opts.activeClass : ''}`.trim();
  cardEl.innerHTML = `
    <div class="slide-thumb-view">
      <div class="slide-thumb-elements"></div>
      <div class="slide-thumb-lyrics"></div>
      <div class="slide-thumb-overlay">
        ${opts.showDragHandle
          ? '<span class="preview-card-drag-handle" title="Drag to reorder slide" role="button" tabindex="0" aria-label="Reorder slide. Press Alt+Left or Alt+Right to move.">⠿</span>'
          : ''}
        <span class="slide-badge-pill" style="color: ${opts.badgeColor};">${opts.badge}</span>
      </div>
    </div>
  `;
  const view = cardEl.querySelector<HTMLElement>('.slide-thumb-view')!;
  const elementsEl = cardEl.querySelector<HTMLElement>('.slide-thumb-elements')!;
  const lyricsEl = cardEl.querySelector<HTMLElement>('.slide-thumb-lyrics')!;
  opts.applyBackground(view);

  let drawnWidth = 0;
  const draw = () => {
    const w = view.clientWidth;
    if (!w || w === drawnWidth) return;
    drawnWidth = w;
    renderSlideVisual(view, elementsEl, lyricsEl, opts.slide, opts.theme);
  };
  new ResizeObserver(draw).observe(view);
  return cardEl;
}
