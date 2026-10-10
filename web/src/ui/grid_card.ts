/**
 * OpenSanctuary / OS-Next Universal Grid Card & Selection Controller
 *
 * Provides standardized markup, thumbnail layout, status/duration badges,
 * click/double-click dispatch, and keyboard navigation across media, video,
 * theme, and library catalog grids.
 */

import { formatMediaTime } from '../core/ui_utils.ts';
import { escapeHtml, escapeUserHtml } from '../core/presentation_helpers.ts';

export type ThumbnailType = 'image' | 'video' | 'css' | 'icon';

export interface CardBadge {
  text: string;
  color?: string;
  bg?: string;
  title?: string;
}

export interface GridCardThumbnail {
  type: ThumbnailType;
  value: string; // Image URL, Video URL, CSS background, or Icon emoji
  durationSec?: number | null;
  fontFamily?: string;
}

export interface GridCardOptions {
  id: string;
  title: string;
  subtitle?: string;
  thumbnail: GridCardThumbnail;
  badge?: CardBadge;
  isSelected?: boolean;
  className?: string;
  thumbClassName?: string;
  titleClassName?: string;
  tabIndex?: number;
  onClick?: (e: MouseEvent) => void;
  onDoubleClick?: (e: MouseEvent) => void;
  onContextMenu?: (e: MouseEvent) => void;
}

/**
 * Builds a standardized, accessible grid card element.
 */
export function buildGridCard(options: GridCardOptions): HTMLElement {
  if (typeof document === 'undefined' || typeof document.createElement !== 'function') {
    return null as unknown as HTMLElement;
  }
  const card = document.createElement('div');
  const baseClasses = ['ui-grid-card', options.className || ''];
  if (options.isSelected) baseClasses.push('selected');
  card.className = baseClasses.filter(Boolean).join(' ');
  card.dataset.id = options.id;
  card.tabIndex = options.tabIndex ?? 0;
  card.title = options.title;

  // Thumbnail container
  const thumbEl = document.createElement('div');
  thumbEl.className = `grid-card-thumb ${options.thumbClassName || ''}`.trim();

  if (options.thumbnail.type === 'image') {
    const url = options.thumbnail.value || '';
    const finalUrl = (url.startsWith('http://') || url.startsWith('https://') || url.startsWith('data:') || url.startsWith('blob:') || url.startsWith('/'))
      ? url
      : (url.startsWith('media/') ? `/${url}` : url);
    if (finalUrl) {
      thumbEl.style.backgroundImage = `url("${finalUrl}")`;
      thumbEl.style.backgroundSize = 'cover';
      thumbEl.style.backgroundPosition = 'center';
    }
  } else if (options.thumbnail.type === 'video') {
    const video = document.createElement('video');
    video.src = options.thumbnail.value || '';
    video.muted = true;
    video.playsInline = true;
    video.preload = 'metadata';
    thumbEl.appendChild(video);

    if (options.thumbnail.durationSec && options.thumbnail.durationSec > 0) {
      const durBadge = document.createElement('div');
      durBadge.className = 'media-video-duration';
      durBadge.textContent = formatMediaTime(options.thumbnail.durationSec);
      thumbEl.appendChild(durBadge);
    }
  } else if (options.thumbnail.type === 'css') {
    thumbEl.style.background = options.thumbnail.value || 'transparent';
    if (options.thumbnail.fontFamily) {
      thumbEl.style.fontFamily = options.thumbnail.fontFamily;
      thumbEl.innerHTML = `
        <span class="theme-picker-preview-label">
          Grace &amp; Praise<br>
          <span style="font-size: 8.5px; font-weight: normal; opacity: 0.85;">Sample Typography</span>
        </span>`;
    }
  } else if (options.thumbnail.type === 'icon') {
    thumbEl.style.background = 'linear-gradient(135deg, #1f1c18, #473e34)';
    thumbEl.style.display = 'flex';
    thumbEl.style.alignItems = 'center';
    thumbEl.style.justifyContent = 'center';
    thumbEl.style.fontSize = '24px';
    thumbEl.textContent = options.thumbnail.value || '📄';
  }

  // Top-right status / tag badge
  if (options.badge && options.badge.text) {
    const badgeEl = document.createElement('span');
    badgeEl.className = 'grid-card-badge';
    badgeEl.textContent = options.badge.text;
    if (options.badge.color) badgeEl.style.color = options.badge.color;
    if (options.badge.bg) badgeEl.style.backgroundColor = options.badge.bg;
    if (options.badge.title) badgeEl.title = options.badge.title;
    thumbEl.appendChild(badgeEl);
  }

  card.appendChild(thumbEl);

  // Title label
  const titleEl = document.createElement('div');
  titleEl.className = `grid-card-title ${options.titleClassName || ''}`.trim();
  titleEl.innerHTML = escapeUserHtml(options.title);
  card.appendChild(titleEl);

  // Optional subtitle
  if (options.subtitle) {
    const subEl = document.createElement('div');
    subEl.className = 'grid-card-subtitle';
    subEl.innerHTML = escapeUserHtml(options.subtitle);
    card.appendChild(subEl);
  }

  // Event bindings
  if (options.onClick) {
    card.addEventListener('click', (e) => options.onClick!(e));
  }
  if (options.onDoubleClick) {
    card.addEventListener('dblclick', (e) => options.onDoubleClick!(e));
  }
  if (options.onContextMenu) {
    card.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      options.onContextMenu!(e);
    });
  }

  return card;
}

export interface SelectionControllerOptions<T> {
  getItemId: (item: T) => string;
  onSelect?: (item: T, index: number) => void;
  onActivate?: (item: T, index: number) => void;
}

/**
 * Controller to manage selection state and keyboard navigation across a grid of items.
 */
export class GridSelectionController<T = any> {
  private items: T[] = [];
  private selectedId: string | null = null;
  private options: SelectionControllerOptions<T>;

  constructor(options: SelectionControllerOptions<T>) {
    this.options = options;
  }

  public setItems(items: T[], autoSelectFirst = false): void {
    this.items = items;
    if (autoSelectFirst && items.length > 0 && !this.selectedId) {
      this.selectIndex(0);
    } else if (this.selectedId) {
      const stillExists = items.some(it => this.options.getItemId(it) === this.selectedId);
      if (!stillExists) {
        this.selectedId = null;
      }
    }
  }

  public getSelectedId(): string | null {
    return this.selectedId;
  }

  public getSelectedItem(): T | null {
    if (!this.selectedId) return null;
    return this.items.find(it => this.options.getItemId(it) === this.selectedId) || null;
  }

  public select(id: string | null): void {
    this.selectedId = id;
    if (!id) return;
    const idx = this.items.findIndex(it => this.options.getItemId(it) === id);
    if (idx !== -1 && this.options.onSelect) {
      this.options.onSelect(this.items[idx], idx);
    }
  }

  public selectIndex(index: number): void {
    if (index >= 0 && index < this.items.length) {
      const item = this.items[index];
      this.select(this.options.getItemId(item));
    }
  }

  public activate(id: string): void {
    this.select(id);
    const idx = this.items.findIndex(it => this.options.getItemId(it) === id);
    if (idx !== -1 && this.options.onActivate) {
      this.options.onActivate(this.items[idx], idx);
    }
  }

  public handleKeyDown(e: KeyboardEvent, colsPerRow: number = 4): boolean {
    if (this.items.length === 0) return false;

    const currentIdx = this.selectedId
      ? this.items.findIndex(it => this.options.getItemId(it) === this.selectedId)
      : -1;

    let targetIdx = currentIdx;

    if (e.key === 'ArrowRight') {
      targetIdx = (currentIdx + 1) % this.items.length;
    } else if (e.key === 'ArrowLeft') {
      targetIdx = (currentIdx - 1 + this.items.length) % this.items.length;
    } else if (e.key === 'ArrowDown') {
      targetIdx = Math.min(this.items.length - 1, currentIdx + colsPerRow);
    } else if (e.key === 'ArrowUp') {
      targetIdx = Math.max(0, currentIdx - colsPerRow);
    } else if (e.key === 'Enter') {
      if (currentIdx !== -1) {
        this.activate(this.options.getItemId(this.items[currentIdx]));
        return true;
      }
      return false;
    } else {
      return false;
    }

    if (targetIdx !== currentIdx && targetIdx >= 0 && targetIdx < this.items.length) {
      this.selectIndex(targetIdx);
      return true;
    }

    return false;
  }
}
