/**
 * OpenSanctuary / OS-Next Theme & Background Picker Modal Controller
 *
 * Shares a single "Background" modal (id="media-image-picker-modal", see
 * index.html) with the media image picker (media_image_picker.ts) via two
 * header tabs — "Images" and "CSS" — instead of two separate stacked modals.
 */

import { ThemeDefinition, applyResolvedBackground, isImageBackground } from "../core/presentation_helpers.ts";
import { prepareMediaImagePickerGrid } from "./media_image_picker.ts";
import { buildGridCard } from "./grid_card.ts";
import { renderEmptyState } from "./filter_bar.ts";

export interface ThemePickerContext {
  getAvailableThemes: () => ThemeDefinition[];
  refreshThemes: () => Promise<void>;
  showModal: (el: HTMLElement | null) => void;
  closeModal: (el: HTMLElement | null) => void;
  escapeHtml: (str: any) => string;
}

let selectedThemeForPicker = 'Midnight Ocean';
let onThemePickerApplyCallback: ((theme: ThemeDefinition) => void) | null = null;
let currentContext: ThemePickerContext | null = null;
let currentThemeCategoryFilter: 'all' | 'gradient' | 'solid' | 'pattern' = 'all';

/** Switches the shared "Background" modal between its Images, CSS, and Videos tabs (panels + footer actions). */
export function setBgPickerTab(tab: 'images' | 'css' | 'videos') {
  const isImages = tab === 'images';
  const isCss = tab === 'css';
  const isVideos = tab === 'videos';
  document.getElementById('bg-picker-tab-images')?.classList.toggle('active', isImages);
  document.getElementById('bg-picker-tab-css')?.classList.toggle('active', isCss);
  document.getElementById('bg-picker-tab-videos')?.classList.toggle('active', isVideos);

  const panelImages = document.getElementById('bg-picker-panel-images') as HTMLElement | null;
  const panelCss = document.getElementById('bg-picker-panel-css') as HTMLElement | null;
  const panelVideos = document.getElementById('bg-picker-panel-videos') as HTMLElement | null;
  if (panelImages) panelImages.style.display = isImages ? 'flex' : 'none';
  if (panelCss) panelCss.style.display = isCss ? 'flex' : 'none';
  if (panelVideos) panelVideos.style.display = isVideos ? 'flex' : 'none';

  const footerImages = document.getElementById('bg-picker-footer-actions-images') as HTMLElement | null;
  const footerCss = document.getElementById('bg-picker-footer-actions-css') as HTMLElement | null;
  const footerVideos = document.getElementById('bg-picker-footer-actions-videos') as HTMLElement | null;
  if (footerImages) footerImages.style.display = isImages ? 'flex' : 'none';
  if (footerCss) footerCss.style.display = isCss ? 'flex' : 'none';
  if (footerVideos) footerVideos.style.display = isVideos ? 'flex' : 'none';
}

export function initThemePicker(context: ThemePickerContext) {
  currentContext = context;

  const btnApply = document.getElementById('btn-apply-theme-picker');
  if (btnApply) {
    btnApply.addEventListener('click', () => {
      if (selectedThemeForPicker && onThemePickerApplyCallback && currentContext) {
        const themes = currentContext.getAvailableThemes();
        const found = themes.find(t => t.name === selectedThemeForPicker);
        onThemePickerApplyCallback(found || { name: selectedThemeForPicker, bg: selectedThemeForPicker, font: 'Segoe UI' });
      }
      const modal = document.getElementById('media-image-picker-modal');
      if (modal && currentContext) currentContext.closeModal(modal);
    });
  }

  const tabBtnImages = document.getElementById('bg-picker-tab-images');
  if (tabBtnImages) tabBtnImages.addEventListener('click', () => setBgPickerTab('images'));
  const tabBtnCss = document.getElementById('bg-picker-tab-css');
  if (tabBtnCss) tabBtnCss.addEventListener('click', () => setBgPickerTab('css'));
  const tabBtnVideos = document.getElementById('bg-picker-tab-videos');
  if (tabBtnVideos) tabBtnVideos.addEventListener('click', () => setBgPickerTab('videos'));

  const searchInput = document.getElementById('theme-picker-search') as HTMLInputElement | null;
  if (searchInput) {
    searchInput.addEventListener('input', () => {
      renderThemePickerGrid();
    });
  }

  const catContainer = document.getElementById('theme-picker-categories');
  if (catContainer) {
    catContainer.addEventListener('click', (e) => {
      const btn = (e.target as HTMLElement).closest('.import-tab-btn') as HTMLElement | null;
      if (!btn) return;
      catContainer.querySelectorAll('.import-tab-btn').forEach(b => b.classList.remove('active'));
      btn.classList.add('active');
      currentThemeCategoryFilter = (btn.dataset.cat as any) || 'all';
      renderThemePickerGrid();
    });
  }
}

export function renderThemePickerGrid() {
  if (!currentContext) return;
  const gridEl = document.getElementById('theme-picker-grid');
  if (!gridEl) return;
  gridEl.innerHTML = '';

  const searchInput = document.getElementById('theme-picker-search') as HTMLInputElement | null;
  const query = (searchInput?.value || '').toLowerCase().trim();

  // Photo-backed themes are handled by the Images tab, so this quick grid
  // stays to CSS-only choices (solid, gradient, animated pattern).
  const themes = currentContext.getAvailableThemes().filter(t => !isImageBackground(t.bg));
  const filtered = themes.filter(t => {
    const bg = t.bg || '';
    const isPattern = bg.startsWith('pattern:');
    const isSolid = bg.startsWith('#') || bg.startsWith('rgb');
    const isGradient = bg.startsWith('linear-gradient') || bg.startsWith('radial-gradient');

    if (currentThemeCategoryFilter === 'gradient' && !isGradient) return false;
    if (currentThemeCategoryFilter === 'solid' && !isSolid) return false;
    if (currentThemeCategoryFilter === 'pattern' && !isPattern) return false;

    if (query && !t.name.toLowerCase().includes(query)) return false;

    return true;
  });

  const emptyStateEl = document.getElementById('theme-picker-empty');
  if (emptyStateEl) {
    emptyStateEl.hidden = filtered.length > 0;
    if (filtered.length === 0) {
      renderEmptyState(emptyStateEl, {
        icon: '🎨',
        title: 'No Styles Found',
        description: query ? `No styles matching "${query}".` : 'No styles found in this category.',
        actionLabel: query ? 'Clear Search' : undefined,
        onAction: query ? () => {
          const sInput = document.getElementById('theme-picker-search') as HTMLInputElement | null;
          if (sInput) sInput.value = '';
          renderThemePickerGrid();
        } : undefined
      });
    }
  }
  if (filtered.length === 0) return;

  filtered.forEach(t => {
    const card = buildGridCard({
      id: t.name,
      title: t.name,
      thumbnail: {
        type: 'css',
        value: t.bg,
        fontFamily: t.font || 'Segoe UI'
      },
      isSelected: selectedThemeForPicker === t.name,
      className: 'theme-picker-card',
      thumbClassName: 'theme-picker-preview',
      titleClassName: 'theme-picker-title',
      onClick: () => {
        selectedThemeForPicker = t.name;
        document.querySelectorAll('.theme-picker-card').forEach(c => c.classList.remove('selected'));
        card.classList.add('selected');
      },
      onDoubleClick: () => {
        selectedThemeForPicker = t.name;
        if (onThemePickerApplyCallback && currentContext) {
          const found = themes.find(x => x.name === t.name);
          onThemePickerApplyCallback(found || { name: t.name, bg: t.bg, font: t.font || 'Segoe UI' });
        }
        const modal = document.getElementById('media-image-picker-modal');
        if (modal && currentContext) currentContext.closeModal(modal);
      }
    });

    if (card) {
      const previewEl = card.querySelector('.theme-picker-preview') as HTMLElement | null;
      if (previewEl) applyResolvedBackground(previewEl, t.bg);
      gridEl.appendChild(card);
    }
  });
}

/**
 * Populates the CSS grid and arms its apply callback, without touching tabs
 * or showing the modal — so it can be called both by openThemePicker() (the
 * primary entry point) and, as a bridge, by openMediaImagePicker() (so the
 * CSS tab still works if the operator switches to it after opening via the
 * Images tab instead).
 */
export function prepareThemePickerGrid(
  item: { theme_name?: string } | null,
  callback: ((theme: ThemeDefinition) => void) | null
) {
  if (!currentContext) return;
  selectedThemeForPicker = item ? (item.theme_name || 'Midnight Ocean') : 'Midnight Ocean';
  onThemePickerApplyCallback = callback;

  currentThemeCategoryFilter = 'all';
  const searchInput = document.getElementById('theme-picker-search') as HTMLInputElement | null;
  if (searchInput) searchInput.value = '';
  const catContainer = document.getElementById('theme-picker-categories');
  if (catContainer) {
    catContainer.querySelectorAll('.import-tab-btn').forEach(b => {
      b.classList.toggle('active', (b as HTMLElement).dataset.cat === 'all');
    });
  }

  renderThemePickerGrid();
}

export async function openThemePicker(
  item: { theme_name?: string } | null,
  callback: ((theme: ThemeDefinition) => void) | null = null
) {
  if (!currentContext) return;
  await currentContext.refreshThemes();
  prepareThemePickerGrid(item, callback);
  const btnTheme = document.getElementById('btn-apply-theme-picker');
  if (btnTheme) {
    btnTheme.textContent = 'Apply Theme';
  }

  // Bridge: picking a photo on the Images tab while we got here via the CSS
  // tab's entry point should apply exactly like "Apply Theme" would, since
  // both tabs are now one modal — otherwise switching tabs mid-flow would
  // silently do nothing (the exact bug this modal used to have).
  await prepareMediaImagePickerGrid({
    onSelectSlide: (_filePath, cssBg, name) => {
      if (callback) callback({ name, bg: cssBg, font: 'Segoe UI' });
    }
  });

  setBgPickerTab('css');
  const modalEl = document.getElementById('media-image-picker-modal');
  if (modalEl) currentContext.showModal(modalEl);
}
