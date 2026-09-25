/**
 * OpenSanctuary / OS-Next Universal Media Background Image Picker
 * Extracted from app_core.ts (ongoing modularization pass) — the shared "pick a
 * background photo" grid used by the slide editor, the schedule/preview context
 * menus, and the Theme Picker's "Images" tab.
 *
 * Shares a single "Background" modal (id="media-image-picker-modal", see
 * index.html) with the theme/CSS picker (theme_picker.ts) via two header
 * tabs — "Images" and "CSS".
 */
import { isVideoBackground } from '../core/presentation_helpers.ts';
import { prepareThemePickerGrid, setBgPickerTab } from './theme_picker';
import { prepareVideoPickerGrid } from './video_picker';
import { buildGridCard } from './grid_card';
import { renderEmptyState } from './filter_bar';

export interface MediaImagePickerContext {
  showModal(el: HTMLElement | null): void;
  closeModal(el: HTMLElement | null): void;
  escapeHtml(str: any): string;
}

let ctx: MediaImagePickerContext | null = null;

function inferImageCategory(name: string, explicitCategory?: string): string {
  if (explicitCategory) return explicitCategory.toLowerCase();
  const lower = name.toLowerCase();
  if (lower.includes('mosque') || lower.includes('sanctuary') || lower.includes('church') || lower.includes('cathedral') || lower.includes('worship') || lower.includes('cross') || lower.includes('altar') || lower.includes('temple')) {
    return 'worship';
  }
  if (lower.includes('galaxy') || lower.includes('bubble') || lower.includes('abstract') || lower.includes('texture') || lower.includes('lights') || lower.includes('fractal') || lower.includes('wave') || lower.includes('pattern')) {
    return 'texture';
  }
  return 'nature';
}

let resourcesCache: { media: any[] } = { media: [] };
let selectedImageForPicker: string | null = null;
let onImagePickerSlideCallback: ((filePath: string, cssBg: string, name: string) => void) | null = null;
let onImagePickerAllCallback: ((filePath: string, cssBg: string, name: string) => void) | null = null;
let currentImageCategoryFilter = 'all';

export function getAllAvailableImages() {
  const list: { name: string; file_path: string; category: string }[] = [];
  if (resourcesCache && resourcesCache.media && Array.isArray(resourcesCache.media)) {
    resourcesCache.media.forEach(m => {
      const lowerPath = (m.file_path || '').toLowerCase();
      const isImg = (m.media_type === 'Image' || m.media_type === 'image' || m.media_type === 1 || lowerPath.endsWith('.jpg') || lowerPath.endsWith('.jpeg') || lowerPath.endsWith('.png') || lowerPath.endsWith('.webp') || lowerPath.endsWith('.gif') || lowerPath.endsWith('.bmp') || lowerPath.endsWith('.svg'));
      if (isImg && !list.some(x => x.file_path === m.file_path)) {
        const displayName = m.name || m.file_path.split('/').pop() || 'Untitled Image';
        list.push({
          name: displayName,
          file_path: m.file_path,
          category: inferImageCategory(displayName, m.category)
        });
      }
    });
  }
  return list;
}

/**
 * Populates the Images grid and arms its apply callbacks, without touching
 * tabs or showing the modal — so it can be called both by
 * openMediaImagePicker() (the primary entry point) and, as a bridge, by
 * openThemePicker() (so the Images tab still works if the operator switches
 * to it after opening via the CSS tab instead).
 */
export async function prepareMediaImagePickerGrid({ currentImage = null, onSelectSlide = null, onSelectAll = null, isStudio = false }: { currentImage?: string | null; onSelectSlide?: ((filePath: string, cssBg: string, name: string) => void) | null; onSelectAll?: ((filePath: string, cssBg: string, name: string) => void) | null; isStudio?: boolean } = {}) {
  // Always query fresh media from DB
  try {
    const res = await fetch('/api/media');
    if (res.ok) {
      resourcesCache.media = await res.json();
    }
  } catch (err) {}

  const available = getAllAvailableImages();
  selectedImageForPicker = currentImage || (available.length > 0 ? available[0].file_path : null);
  onImagePickerSlideCallback = onSelectSlide;
  onImagePickerAllCallback = onSelectAll;

  const btnAll = document.getElementById('btn-apply-image-all');
  if (btnAll) {
    (btnAll as HTMLElement).style.display = onSelectAll ? 'inline-flex' : 'none';
  }

  const btnSlide = document.getElementById('btn-apply-image-slide');
  if (btnSlide) {
    btnSlide.textContent = isStudio ? 'Apply to Active Slide' : 'Apply Background Image';
  }
  const btnTheme = document.getElementById('btn-apply-theme-picker');
  if (btnTheme) {
    btnTheme.textContent = isStudio ? 'Apply to Active Slide' : 'Apply Theme';
  }

  const searchEl = document.getElementById('image-picker-search') as HTMLInputElement | null;
  if (searchEl) searchEl.value = '';
  currentImageCategoryFilter = 'all';
  document.querySelectorAll('#image-picker-categories .import-tab-btn').forEach(b => {
    b.classList.toggle('active', b.getAttribute('data-cat') === 'all');
  });

  renderMediaImagePickerGrid();
}

export async function openMediaImagePicker(opts: {
  currentImage?: string | null;
  currentVideo?: string | null;
  initialTab?: 'images' | 'css' | 'videos';
  onSelectSlide?: ((filePath: string, cssBg: string, name: string, isVideo?: boolean, isLooping?: boolean) => void) | null;
  onSelectAll?: ((filePath: string, cssBg: string, name: string, isVideo?: boolean, isLooping?: boolean) => void) | null;
  isStudio?: boolean;
} = {}) {
  const modal = document.getElementById('media-image-picker-modal');
  if (!modal || !ctx) return;

  await prepareMediaImagePickerGrid(opts);

  // Bridge: applying a CSS theme or Video while we got here via the Images tab's entry
  // point should apply cleanly, since all tabs share one modal.
  const onSelectSlide = opts.onSelectSlide;
  const onSelectAll = opts.onSelectAll;

  prepareThemePickerGrid(null, (theme) => {
    if (onSelectSlide) onSelectSlide('', theme.bg, theme.name, false, false);
  });

  await prepareVideoPickerGrid({
    currentVideo: opts.currentVideo || (opts.currentImage && isVideoBackground(opts.currentImage) ? opts.currentImage : null),
    onSelectSlide: (filePath, cssBg, name, isVideo, isLooping) => {
      if (onSelectSlide) onSelectSlide(filePath, cssBg, name, isVideo, isLooping);
    },
    onSelectAll: (filePath, cssBg, name, isVideo, isLooping) => {
      if (onSelectAll) onSelectAll(filePath, cssBg, name, isVideo, isLooping);
    },
    isStudio: opts.isStudio
  });

  setBgPickerTab(opts.initialTab || 'images');
  ctx.showModal(modal);
}

export function renderMediaImagePickerGrid() {
  const gridEl = document.getElementById('media-image-picker-grid');
  if (!gridEl) return;
  gridEl.innerHTML = '';

  const query = ((document.getElementById('image-picker-search') as HTMLInputElement | null)?.value || '').toLowerCase().trim();
  const allImages = getAllAvailableImages();

  const filtered = allImages.filter(img => {
    if (currentImageCategoryFilter !== 'all' && img.category !== currentImageCategoryFilter) return false;
    if (query && !img.name.toLowerCase().includes(query) && !img.file_path.toLowerCase().includes(query)) return false;
    return true;
  });

  const emptyStateEl = document.getElementById('media-image-picker-empty');
  if (emptyStateEl) {
    emptyStateEl.hidden = filtered.length > 0;
    if (filtered.length === 0) {
      renderEmptyState(emptyStateEl, {
        icon: '🖼️',
        title: 'No Photos Found',
        description: query ? `No background photos matching "${query}".` : 'No background photos available in this category.',
        actionLabel: query ? 'Clear Search' : undefined,
        onAction: query ? () => {
          const sInput = document.getElementById('image-picker-search') as HTMLInputElement | null;
          if (sInput) sInput.value = '';
          renderMediaImagePickerGrid();
        } : undefined
      });
    }
  }
  if (filtered.length === 0) return;

  filtered.forEach(img => {
    const isSel = selectedImageForPicker === img.file_path || (selectedImageForPicker && selectedImageForPicker.includes(img.file_path));
    const card = buildGridCard({
      id: img.file_path,
      title: img.name,
      thumbnail: {
        type: 'image',
        value: img.file_path,
      },
      isSelected: !!isSel,
      className: 'media-image-card',
      thumbClassName: 'media-image-thumb',
      titleClassName: 'media-image-info',
      onClick: () => {
        selectedImageForPicker = img.file_path;
        document.querySelectorAll('.media-image-card').forEach(c => c.classList.remove('selected'));
        card.classList.add('selected');
      },
      onDoubleClick: () => {
        selectedImageForPicker = img.file_path;
        const cssBg = `url('${img.file_path}') center/cover no-repeat`;
        if (onImagePickerSlideCallback) {
          onImagePickerSlideCallback(img.file_path, cssBg, img.name);
        }
        ctx!.closeModal(document.getElementById('media-image-picker-modal'));
      }
    });

    if (card) {
      gridEl.appendChild(card);
    }
  });
}

export function initMediaImagePicker(context: MediaImagePickerContext) {
  ctx = context;

  const mediaImagePickerModal = document.getElementById('media-image-picker-modal');

  document.getElementById('btn-close-image-picker')?.addEventListener('click', () => ctx!.closeModal(mediaImagePickerModal));
  document.getElementById('btn-cancel-image-picker')?.addEventListener('click', () => ctx!.closeModal(mediaImagePickerModal));

  document.getElementById('btn-apply-image-slide')?.addEventListener('click', () => {
    if (selectedImageForPicker && onImagePickerSlideCallback) {
      const allImages = getAllAvailableImages();
      const found = allImages.find(x => x.file_path === selectedImageForPicker) || { name: 'Photo Background', file_path: selectedImageForPicker };
      const cssBg = `url('${found.file_path}') center/cover no-repeat`;
      onImagePickerSlideCallback(found.file_path, cssBg, found.name);
    }
    ctx!.closeModal(mediaImagePickerModal);
  });

  document.getElementById('btn-apply-image-all')?.addEventListener('click', () => {
    if (selectedImageForPicker && onImagePickerAllCallback) {
      const allImages = getAllAvailableImages();
      const found = allImages.find(x => x.file_path === selectedImageForPicker) || { name: 'Photo Background', file_path: selectedImageForPicker };
      const cssBg = `url('${found.file_path}') center/cover no-repeat`;
      onImagePickerAllCallback(found.file_path, cssBg, found.name);
    }
    ctx!.closeModal(mediaImagePickerModal);
  });

  document.getElementById('image-picker-search')?.addEventListener('input', () => renderMediaImagePickerGrid());

  // Category filter tabs
  document.addEventListener('DOMContentLoaded', () => {
    const catWrap = document.getElementById('image-picker-categories');
    if (catWrap) {
      catWrap.addEventListener('click', (e: any) => {
        const btn = e.target.closest('.import-tab-btn');
        if (btn) {
          catWrap.querySelectorAll('.import-tab-btn').forEach(b => b.classList.remove('active'));
          btn.classList.add('active');
          currentImageCategoryFilter = btn.getAttribute('data-cat') || 'all';
          renderMediaImagePickerGrid();
        }
      });
    }
  });
}
