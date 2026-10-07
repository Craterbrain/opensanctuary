/**
 * OpenSanctuary / OS-Next Library / Resource Panel Controller
 * Extracted from app_core.ts (ongoing modularization pass) — browsing, searching,
 * and adding songs/scriptures/media/presentations/themes to the schedule: the
 * Bible translation selector, Genius lyrics search, online image search (Pexels/
 * Pixabay), and the table/grid catalog renderers.
 *
 * Cross-cutting state (currentTab, selectedLibraryItem, activeBibleVersion,
 * secondaryBibleVersion, isDualBibleMode, installedBibles) is still owned by
 * app_core.ts — it's also read by code outside this panel (the schedule/library
 * item context-menu dispatcher) — and reached here through the context object,
 * following the same pattern settings_dialog.ts/theme_picker.ts/arrangement_modal.ts
 * already use, so this stays a one-way dependency (app_core.ts imports from here,
 * never the reverse).
 */
import { draggable } from '@atlaskit/pragmatic-drag-and-drop/element/adapter';
import { parseScriptureReference } from '../core/bible_parser';
import { escapeCssUrl, escapeHtml, formatParallelSlide, resolveSlideBackgroundElement } from '../core/presentation_helpers.ts';
import { renderSlideVisual } from '../core/slide_render.ts';
import { openThemeEditor, setThemeAsDefault, duplicateTheme, deleteThemeItem } from './theme_editor.ts';

export interface LibraryPanelContext {
  getCurrentTab(): string;
  setCurrentTab(tab: string): void;
  getSelectedLibraryItem(): any;
  setSelectedLibraryItem(item: any): void;
  getActiveBibleVersion(): string;
  setActiveBibleVersion(v: string): void;
  getSecondaryBibleVersion(): string;
  setSecondaryBibleVersion(v: string): void;
  getIsDualBibleMode(): boolean;
  setIsDualBibleMode(v: boolean): void;
  getInstalledBibles(): any[];
  getAppOptions(): Record<string, any>;
  getAvailableThemes(): any[];
  getCurrentSnapshot(): any;
  showToast(message: string, type?: string): void;
  sendCommand(cmd: any): void;
  escapeHtml(str: any): string;
  formatCssBackground(bg: string | null | undefined, defaultGradient?: string): string;
  showContextMenu(el: HTMLElement | null, x: number, y: number): void;
  setContextMenuTarget(target: any): void;
  setContextMenuTargetBible(bible: any): void;
  setBibleVersionUserSelected(v: boolean): void;
  applyDefaultBibleVersionIfUnset(): void;
  formatMediaTime(sec: number): string;
  setAdhocPreview(item: any, tabName: string): void;
}

let ctx: LibraryPanelContext | null = null;
let currentDualSecondaryVerses: any[] = [];

export function getCurrentDualSecondaryVerses(): any[] {
  return currentDualSecondaryVerses;
}

export function setCurrentDualSecondaryVerses(verses: any[]): void {
  currentDualSecondaryVerses = verses || [];
}

export function initLibraryPanel(context: LibraryPanelContext) {
  ctx = context;

  document.getElementById('ctx-theme-edit')?.addEventListener('click', () => {
    if (contextMenuTargetTheme) openThemeEditor(contextMenuTargetTheme);
  });
  document.getElementById('ctx-theme-set-default')?.addEventListener('click', async () => {
    if (contextMenuTargetTheme) {
      await setThemeAsDefault(contextMenuTargetTheme);
      filterAndRenderCatalog();
    }
  });
  document.getElementById('ctx-theme-duplicate')?.addEventListener('click', async () => {
    if (contextMenuTargetTheme) {
      await duplicateTheme(contextMenuTargetTheme);
      filterAndRenderCatalog();
    }
  });
  document.getElementById('ctx-theme-delete')?.addEventListener('click', async () => {
    if (contextMenuTargetTheme) {
      await deleteThemeItem(contextMenuTargetTheme);
      filterAndRenderCatalog();
    }
  });
}

// Panel-local state. Real exported getters (and a setter for activeCategory) below
// because app_ui.ts reads/writes some of these from outside this module — a bare
// `activeCategory = 'videos'`/reading bare `filteredLibraryItems` there (the
// pre-extraction code) only ever touched a stray globalThis property that never
// stayed in sync with this module's own variable, so e.g. the ribbon's "New Video"/
// "New Camera Feed" quick buttons never actually narrowed the category, and pressing
// Enter in the search box to add the first filtered result never fired (the array
// read back was always the initial empty one). Fixed here rather than carried over.
let activeCategory = 'all';
export function getActiveCategory() { return activeCategory; }
export function setActiveCategory(cat: string) { activeCategory = cat; }

let contextMenuTargetTheme: any = null;

let activeLibraryItems: any[] = [];
export function getActiveLibraryItems() { return activeLibraryItems; }

let filteredLibraryItems: any[] = [];
export function getFilteredLibraryItems() { return filteredLibraryItems; }

let selectedCatalogIndex = 0;
let catalogDndCleanups: Array<() => void> = [];

// 'table' or 'grid'. Exported real getter/setter (below) because app_ui.ts's view-mode
// toggle buttons need to set this — a bare `activeResourceViewMode = 'grid'` assignment
// there (the pre-extraction code) only ever set a stray globalThis property, never this
// module's own variable, so toggling the view without also switching library tabs
// silently didn't work. Fixed here rather than carried over.
let activeResourceViewMode: 'table' | 'grid' = 'table';
export function getActiveResourceViewMode() { return activeResourceViewMode; }
export function setActiveResourceViewMode(mode: 'table' | 'grid') { activeResourceViewMode = mode; }

// Search Modes Configuration
const currentSearchModes: Record<string, string> = {
  songs: 'all',          // 'all', 'title', 'lyrics', 'ccli'
  scriptures: 'reference', // 'reference', 'keyword'
  media: 'all',          // 'all', 'videos', 'images', 'audio'
  presentations: 'all',  // 'all', 'title'
  themes: 'all'          // 'all', 'title'
};

// Search & Catalog Elements
const resourceSearchInput = document.getElementById('resource-search') as HTMLInputElement | null;
const btnSearchClear = document.getElementById('btn-search-clear');
const btnSearchMode = document.getElementById('btn-search-mode');
const searchModeIcon = document.getElementById('search-mode-icon');
const searchModeMenu = document.getElementById('search-mode-menu');
const searchModeDropdownWrap = document.getElementById('search-mode-dropdown-wrap');
const catalogTable = document.getElementById('catalog-table');
const catalogTableBody = document.getElementById('catalog-table-body');
const catalogGrid = document.getElementById('catalog-grid');
const catalogItemCountEl = document.getElementById('catalog-item-count');
const categoryTreeContainer = document.getElementById('category-tree-container');
const resourcePreviewMonitor = document.getElementById('resource-preview-monitor');
const btnToggleDualBible = document.getElementById('btn-toggle-dual-bible');

// BIBLE TRANSLATION SELECTOR & SCRIPTURES ENGINE
// ============================================================================
export function updateSearchModeUI() {
  if (!ctx) return;
  const lp = ctx;
  const mode = currentSearchModes[lp.getCurrentTab()] || 'all';

  if (searchModeIcon) {
    if (lp.getCurrentTab() === 'scriptures') searchModeIcon.textContent = mode === 'reference' ? '📖' : '🔍';
    else if (lp.getCurrentTab() === 'media') searchModeIcon.textContent = '🎬';
    else if (lp.getCurrentTab() === 'presentations') searchModeIcon.textContent = '📊';
    else if (lp.getCurrentTab() === 'themes') searchModeIcon.textContent = '🎨';
    else searchModeIcon.textContent = '🔍';
  }

  if (resourceSearchInput) {
    if (lp.getCurrentTab() === 'songs') {
      if (mode === 'title') resourceSearchInput.placeholder = '🔍 Search Song Titles Only...';
      else if (mode === 'lyrics') resourceSearchInput.placeholder = '🔍 Search Full Lyrics Only...';
      else if (mode === 'ccli') resourceSearchInput.placeholder = '🔍 Search by CCLI # or Song ID...';
      else if (mode === 'genius' || activeCategory === 'genius-christian') resourceSearchInput.placeholder = '✨ Search Genius Christian & Worship Lyrics (e.g. Holy Forever, Way Maker)...';
      else resourceSearchInput.placeholder = '🔍 Search Titles, Lyrics, or CCLI #...';
    } else if (lp.getCurrentTab() === 'scriptures') {
      const activeObj = lp.getInstalledBibles().find(b => b.id === lp.getActiveBibleVersion()) || { abbreviation: 'All' };
      if (mode === 'reference') {
        resourceSearchInput.placeholder = `📖 Scripture in ${activeObj.abbreviation} (e.g. Jn 3 16, 1 Cor 13 4-8, Ps 23)...`;
      } else {
        resourceSearchInput.placeholder = `🔍 Keyword search in ${activeObj.abbreviation} (e.g. "grace and truth")...`;
      }
    } else if (lp.getCurrentTab() === 'media') {
      resourceSearchInput.placeholder = '🔍 Search Media by name, tag, type...';
    } else if (lp.getCurrentTab() === 'presentations') {
      resourceSearchInput.placeholder = '🔍 Search Presentations by title, slide content...';
    } else if (lp.getCurrentTab() === 'themes') {
      resourceSearchInput.placeholder = '🔍 Search Themes by name, font, style...';
    }
  }

  if (btnToggleDualBible) {
    btnToggleDualBible.style.display = lp.getCurrentTab() === 'scriptures' ? 'inline-block' : 'none';
  }
  renderSearchModeMenu();
}

export function renderSearchModeMenu() {
  if (!searchModeMenu) return;
  if (!ctx) return;
  const lp = ctx;
  searchModeMenu.innerHTML = '';
  const currentMode = currentSearchModes[lp.getCurrentTab()];
  let options: { id: string; label: string }[] = [];

  if (lp.getCurrentTab() === 'songs') {
    options = [
      { id: 'all', label: '🔍 All Fields (Default)' },
      { id: 'title', label: '📄 Title Only' },
      { id: 'lyrics', label: '📝 Lyrics Only' },
      { id: 'ccli', label: '🔢 CCLI # / Catalog ID' },
      { id: 'genius', label: '✨ Genius Christian Lyrics' }
    ];
  } else if (lp.getCurrentTab() === 'scriptures') {
    options = [
      { id: 'reference', label: '📖 Smart Reference Mode (e.g. Jn 3 16)' },
      { id: 'keyword', label: '🔍 Keyword / Phrase Search' }
    ];
  } else if (lp.getCurrentTab() === 'media') {
    options = [
      { id: 'all', label: '🎬 All Media Types' },
      { id: 'videos', label: '🎥 Videos Only' },
      { id: 'images', label: '🖼️ Images Only' },
      { id: 'audio', label: '🎵 Audio Only' }
    ];
  } else if (lp.getCurrentTab() === 'presentations') {
    options = [
      { id: 'all', label: '📊 All Fields (Title & Slides)' },
      { id: 'title', label: '📄 Title Only' }
    ];
  } else if (lp.getCurrentTab() === 'themes') {
    options = [
      { id: 'all', label: '🎨 All Fields' },
      { id: 'title', label: '🏷️ Theme Name Only' }
    ];
  }

  options.forEach(opt => {
    const item = document.createElement('div');
    item.className = `search-mode-item ${currentMode === opt.id ? 'active' : ''}`;
    item.textContent = opt.label;
    item.addEventListener('click', (e) => {
      e.stopPropagation();
      currentSearchModes[lp.getCurrentTab()] = opt.id;
      if (searchModeDropdownWrap) searchModeDropdownWrap.classList.remove('open');
      updateSearchModeUI();
      filterAndRenderCatalog();
      if (resourceSearchInput) resourceSearchInput.focus();
    });
    searchModeMenu.appendChild(item);
  });
}

btnSearchMode?.addEventListener('click', (e) => {
  e.stopPropagation();
  if (searchModeDropdownWrap) searchModeDropdownWrap.classList.toggle('open');
});

btnSearchClear?.addEventListener('click', () => {
  if (resourceSearchInput) {
    resourceSearchInput.value = '';
    if (btnSearchClear) btnSearchClear.style.display = 'none';
    filterAndRenderCatalog();
    resourceSearchInput.focus();
  }
});

let currentLibraryTabRequestId = 0;

export async function loadLibraryTab(tabName?: string) {
  if (!ctx) return;
  const lp = ctx;
  const normTab = (tabName === 'song' || tabName === 'songs') ? 'songs'
    : ((tabName === 'scripture' || tabName === 'scriptures') ? 'scriptures'
    : ((tabName === 'presentation' || tabName === 'presentations') ? 'presentations'
    : ((tabName === 'theme' || tabName === 'themes') ? 'themes'
    : ((tabName === 'media') ? 'media' : (tabName || 'songs')))));

  lp.setCurrentTab(normTab);
  activeCategory = 'all';
  if (resourceSearchInput && resourceSearchInput.value) {
    resourceSearchInput.value = '';
    if (btnSearchClear) btnSearchClear.style.display = 'none';
  }
  updateSearchModeUI();
  const reqId = ++currentLibraryTabRequestId;

  document.querySelectorAll<HTMLElement>('.tab-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.tab === normTab);
  });

  // Songs/Scriptures default to the table (list) view; Media/Presentations/
  // Themes — all thumbnail-heavy content — default to the grid view.
  activeResourceViewMode = (normTab === 'media' || normTab === 'presentations' || normTab === 'themes') ? 'grid' : 'table';
  document.getElementById('btn-resource-view-grid')?.classList.toggle('active', activeResourceViewMode === 'grid');
  document.getElementById('btn-resource-view-table')?.classList.toggle('active', activeResourceViewMode === 'table');

  if (normTab === 'scriptures') lp.applyDefaultBibleVersionIfUnset();
  renderCategoryTree(normTab);

  try {
    const res = await fetch(`/api/${normTab}`);
    if (reqId !== currentLibraryTabRequestId) return;
    if (res.ok) {
      const data = await res.json();
      activeLibraryItems = Array.isArray(data) ? data : [];
      filterAndRenderCatalog();
    } else {
      activeLibraryItems = [];
      filterAndRenderCatalog();
    }
  } catch (err) {
    if (reqId !== currentLibraryTabRequestId) return;
    console.error(`Error loading ${normTab}:`, err);
    activeLibraryItems = [];
    filterAndRenderCatalog();
  }
}

export function renderCategoryTree(tabName: string) {
  if (!categoryTreeContainer) return;
  categoryTreeContainer.innerHTML = '';
  const normTab = (tabName === 'song' || tabName === 'songs') ? 'songs'
    : ((tabName === 'scripture' || tabName === 'scriptures') ? 'scriptures'
    : ((tabName === 'presentation' || tabName === 'presentations') ? 'presentations'
    : ((tabName === 'theme' || tabName === 'themes') ? 'themes'
    : ((tabName === 'media') ? 'media' : (tabName || 'songs')))));

  if (normTab === 'scriptures' && ctx) {
    const lp = ctx;
    // 1. All Translations Node
    const allNode = document.createElement('div');
    allNode.className = `bible-tree-node ${lp.getActiveBibleVersion() === 'all' ? 'active' : ''}`;
    allNode.innerHTML = `
      <div style="display: flex; align-items: center; gap: 6px;">
        <span>📖</span>
        <span style="font-weight: 600; color: #fff;">All Translations</span>
      </div>
      <span class="bible-lang-badge" style="font-size: 8.5px;">ALL</span>
    `;
    allNode.addEventListener('click', () => {
      lp.setActiveBibleVersion('all');
      lp.setBibleVersionUserSelected(true);
      document.querySelectorAll('.bible-tree-node').forEach(b => b.classList.remove('active'));
      allNode.classList.add('active');
      updateSearchModeUI();
      filterAndRenderCatalog();
    });
    categoryTreeContainer.appendChild(allNode);

    // 2. Installed Versions Section Header
    const headerNode = document.createElement('div');
    headerNode.style.padding = '8px 10px 4px';
    headerNode.style.fontSize = '10px';
    headerNode.style.fontWeight = '700';
    headerNode.style.color = 'var(--text-dim)';
    headerNode.style.textTransform = 'uppercase';
    headerNode.textContent = 'Installed Versions';
    categoryTreeContainer.appendChild(headerNode);

    const availableBibles = lp.getInstalledBibles().filter(b => (b.verse_count === undefined || b.verse_count > 0));
    if (availableBibles.length === 0) {
      const emptyNote = document.createElement('div');
      emptyNote.style.padding = '8px 12px';
      emptyNote.style.fontSize = '11px';
      emptyNote.style.color = 'var(--text-dim)';
      emptyNote.textContent = 'No Bible versions installed.';
      categoryTreeContainer.appendChild(emptyNote);
    } else {
      availableBibles.forEach(bible => {
        const isDefault = areTranslationsEquivalent(bible.id, lp.getAppOptions().defaultBibleVersion) ||
          Boolean(bible.isDefault) ||
          Boolean(bible.is_default) ||
          (!lp.getAppOptions().defaultBibleVersion && bible === availableBibles[0]);
        const isActive = bible.id === lp.getActiveBibleVersion();

        const nodeEl = document.createElement('div');
        nodeEl.className = `bible-tree-node ${isActive ? 'active' : ''}`;
        nodeEl.dataset.bibleId = bible.id;

        nodeEl.innerHTML = `
          <div style="display: flex; align-items: center; gap: 6px; overflow: hidden;">
            <span style="font-weight: 600; color: #fff;">${lp.escapeHtml(bible.abbreviation || bible.id)}</span>
            <span style="font-size: 10px; color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${lp.escapeHtml(bible.name || bible.id)}</span>
          </div>
          <div style="display: flex; align-items: center; gap: 4px;">
            ${bible.language ? `<span class="bible-lang-badge" style="font-size: 8.5px;">${lp.escapeHtml(bible.language)}</span>` : ''}
            ${isDefault ? '<span class="bible-default-badge" title="Default Translation">⭐</span>' : ''}
          </div>
        `;

        nodeEl.addEventListener('click', () => {
          lp.setActiveBibleVersion(bible.id);
          lp.setBibleVersionUserSelected(true);
          document.querySelectorAll('.bible-tree-node').forEach(b => b.classList.remove('active'));
          nodeEl.classList.add('active');
          updateSearchModeUI();
          filterAndRenderCatalog();
        });

        nodeEl.addEventListener('contextmenu', (e) => {
          e.preventDefault();
          e.stopPropagation();
          lp.setContextMenuTargetBible(bible);
          lp.showContextMenu(document.getElementById('bible-version-context-menu'), e.clientX, e.clientY);
        });

        categoryTreeContainer.appendChild(nodeEl);
      });
    }

    // 3. Install Button at Bottom
    const actionWrap = document.createElement('div');
    actionWrap.style.padding = '8px 10px';
    actionWrap.style.display = 'flex';
    actionWrap.style.flexDirection = 'column';
    actionWrap.style.gap = '4px';
    actionWrap.style.borderTop = '1px solid var(--border-color)';
    actionWrap.style.marginTop = '6px';

    const btnInstallBibles = document.createElement('button');
    btnInstallBibles.className = 'btn btn-tag';
    btnInstallBibles.style.width = '100%';
    btnInstallBibles.style.textAlign = 'left';
    btnInstallBibles.innerHTML = '➕ Install Bibles (ChurchApps)...';
    btnInstallBibles.addEventListener('click', () => openImportModal());
    actionWrap.appendChild(btnInstallBibles);

    categoryTreeContainer.appendChild(actionWrap);
    return;
  }

  // Generic Category Tree for other tabs
  let nodes: { id: string; label: string }[] = [];
  if (tabName === 'songs') {
    nodes = [
      { id: 'all', label: '📁 All Songs' },
      { id: 'collections', label: '📁 Collections' },
      { id: 'my-collections', label: '📁 My Collections' },
      { id: 'favorites', label: '⭐ Favorites' },
      { id: 'genius-christian', label: '✨ Genius Christian Lyrics' }
    ];
  } else if (tabName === 'media') {
    nodes = [
      { id: 'all', label: '🎬 All Media' },
      { id: 'videos', label: '🎥 Videos & Motion Loops' },
      { id: 'images', label: '🖼️ Images & Stills' },
      { id: 'audio', label: '🎵 Audio Tracks' },
      { id: 'feeds', label: '📡 Live Camera Feeds' },
      { id: 'online-images', label: '🌐 Online Images (Pexels/Pixabay)' },
      { id: 'ytdlp-import', label: '📥 Download Video (yt-dlp)' }
    ];
  } else if (tabName === 'presentations') {
    nodes = [
      { id: 'all', label: '📊 All Presentations' },
      { id: 'announcements', label: '📢 Announcements' },
      { id: 'sermons', label: '📖 Sermon Slides' }
    ];
  } else if (tabName === 'themes') {
    nodes = [
      { id: 'all', label: '🎨 All Themes' },
      { id: 'song-themes', label: '🎵 Song Themes' },
      { id: 'scripture-themes', label: '📖 Scripture Themes' },
      { id: 'presentation-themes', label: '📊 Presentation Themes' }
    ];
  }

  nodes.forEach(n => {
    const nodeEl = document.createElement('div');
    nodeEl.className = `category-node ${activeCategory === n.id ? 'active' : ''}`;
    nodeEl.textContent = n.label;
    nodeEl.addEventListener('click', () => {
      if (n.id === 'ytdlp-import') {
        openImportModal('ytdlp');
        return;
      }
      activeCategory = n.id;
      document.querySelectorAll('.category-node').forEach(c => c.classList.remove('active'));
      nodeEl.classList.add('active');
      if (n.id === 'online-images' && activeResourceViewMode !== 'grid') {
        activeResourceViewMode = 'grid';
        document.getElementById('btn-resource-view-grid')?.classList.add('active');
        document.getElementById('btn-resource-view-table')?.classList.remove('active');
      }
      filterAndRenderCatalog();
    });
    categoryTreeContainer.appendChild(nodeEl);
  });
}

export function areTranslationsEquivalent(t1: unknown, t2: unknown) {
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

export function getBibleAbbreviation(str: unknown) {
  if (!str) return 'KJV';
  if (!ctx) return 'KJV';
  const b = ctx.getInstalledBibles().find(x => areTranslationsEquivalent(x.id, str) || areTranslationsEquivalent(x.abbreviation, str) || areTranslationsEquivalent(x.name, str));
  if (b && b.abbreviation) return b.abbreviation;
  const s = String(str).trim();
  if (s.toLowerCase().includes('world english') || s.toLowerCase() === 'web' || s.toLowerCase() === 'world_english_bible') return 'WEB';
  if (s.toLowerCase().includes('red letter') || s.toLowerCase().includes('rl')) return 'KJV (RL)';
  if (s.toLowerCase().includes('king james') || s.toLowerCase() === 'kjv') return 'KJV';
  if (s.toLowerCase().includes('american standard') || s.toLowerCase() === 'asv') return 'ASV';
  if (s.toLowerCase().includes('basic english') || s.toLowerCase() === 'bbe') return 'BBE';
  if (s.toLowerCase().includes('holman') || s.toLowerCase() === 'hcsb') return 'HCSB';
  return s.length > 8 ? s.substring(0, 7) + '…' : s.toUpperCase();
}

export function normalizeBibleApiCode(str: unknown) {
  if (!str) return 'kjv';
  const s = String(str).toLowerCase().replace(/[\s\-_()]/g, '');
  if (s.includes('worldenglish') || s === 'web') return 'web';
  if (s.includes('kingjames') || s.includes('kjv')) return 'kjv';
  if (s.includes('americanstandard') || s.includes('asv')) return 'asv';
  if (s.includes('basicenglish') || s === 'bbe') return 'bbe';
  if (s.includes('holman') || s.includes('hcsb')) return 'hcsb';
  return s.substring(0, 8);
}

export function scriptureMatchesVersion(sc: any, versionId: unknown) {
  if (!versionId || versionId === 'all') return true;
  if (areTranslationsEquivalent(sc.version, versionId)) return true;
  if (!ctx) return false;
  const b = ctx.getInstalledBibles().find(x => areTranslationsEquivalent(x.id, versionId) || areTranslationsEquivalent(x.abbreviation, versionId) || areTranslationsEquivalent(x.name, versionId));
  if (b) {
    return areTranslationsEquivalent(sc.version, b.abbreviation) || areTranslationsEquivalent(sc.version, b.id) || areTranslationsEquivalent(sc.version, b.name);
  }
  return false;
}

if (btnToggleDualBible) {
  btnToggleDualBible.addEventListener('click', () => {
    if (!ctx) return;
    const lp = ctx;
    lp.setIsDualBibleMode(!lp.getIsDualBibleMode());
    const pVersion = (lp.getSelectedLibraryItem() && lp.getSelectedLibraryItem().version ? lp.getSelectedLibraryItem().version : (lp.getActiveBibleVersion() !== 'all' ? lp.getActiveBibleVersion() : (lp.getInstalledBibles()[0] ? lp.getInstalledBibles()[0].abbreviation : 'Primary'))).trim();

    if (lp.getIsDualBibleMode()) {
      const otherInstalled = lp.getInstalledBibles().filter(b =>
        !areTranslationsEquivalent(b.id, pVersion) &&
        !areTranslationsEquivalent(b.abbreviation, pVersion) &&
        !areTranslationsEquivalent(b.name, pVersion)
      );
      if (!lp.getSecondaryBibleVersion() || areTranslationsEquivalent(lp.getSecondaryBibleVersion(), pVersion)) {
        if (otherInstalled.length > 0) {
          lp.setSecondaryBibleVersion(otherInstalled[0].id);
        } else {
          const standardOnline = ['kjv', 'asv', 'web', 'bbe', 'hcsb'];
          const fallback = standardOnline.find(code => !areTranslationsEquivalent(code, pVersion));
          lp.setSecondaryBibleVersion(fallback || 'asv');
        }
      }
    }
    btnToggleDualBible.classList.toggle('active', lp.getIsDualBibleMode());
    const pAbbr = getBibleAbbreviation(pVersion);
    const sAbbr = getBibleAbbreviation(lp.getSecondaryBibleVersion() || 'Parallel');
    btnToggleDualBible.textContent = lp.getIsDualBibleMode()
      ? `👥 Dual: ${pAbbr} | ${sAbbr}`
      : '👥 Compare';
    if (lp.getSelectedLibraryItem()) renderAssetPreview(lp.getSelectedLibraryItem(), 'scriptures');
  });
}

// As-You-Type Query Engine & Filtering Logic

let geniusSearchTimer: ReturnType<typeof setTimeout> | null = null;

export async function searchGeniusChristianSongs(query: string) {
  if (!ctx) return;
  const lp = ctx;
  if (geniusSearchTimer) clearTimeout(geniusSearchTimer);

  const container = (activeResourceViewMode === 'table') ? catalogTableBody : catalogGrid;
  if (container) {
    container.innerHTML = `
      <div style="padding: 30px; text-align: center; color: #ffd700; font-weight: 500;">
        <span style="font-size: 18px;">✨</span> Searching Genius for Christian lyrics matching "${lp.escapeHtml(query)}"...
      </div>`;
  }

  geniusSearchTimer = setTimeout(async () => {
    try {
      const res = await fetch(`/api/songs/genius/search?q=${encodeURIComponent(query)}`);
      if (res.ok) {
        const hits = await res.json();
        if (hits.length === 0) {
          if (container) {
            container.innerHTML = `
              <div style="padding: 30px; text-align: center; color: var(--text-dim);">
                <p>No Christian lyrics found on Genius for "${lp.escapeHtml(query)}".</p>
                <p style="font-size: 11px; margin-top: 4px;">Try searching by another title or artist.</p>
              </div>`;
          }
        } else {
          renderGeniusHits(hits);
        }
      } else {
        if (container) {
          container.innerHTML = `<div style="padding: 20px; text-align: center; color: #ff5252;">Error searching Genius: ${lp.escapeHtml(res.statusText)}</div>`;
        }
      }
    } catch (e) {
      if (container) {
        container.innerHTML = `<div style="padding: 20px; text-align: center; color: #ff5252;">Network error: ${lp.escapeHtml(e instanceof Error ? e.message : String(e))}</div>`;
      }
    }
  }, 300);
}

export function renderGeniusHits(hits: any[]) {
  if (!ctx) return;
  const lp = ctx;
  if (catalogItemCountEl) {
    catalogItemCountEl.textContent = `${hits.length} Genius songs`;
  }

  if (activeResourceViewMode === 'table') {
    if (catalogTable) catalogTable.style.display = 'table';
    if (catalogGrid) catalogGrid.style.display = 'none';
    const headers = document.getElementById('catalog-headers');
    if (headers) headers.innerHTML = `<th>Title</th><th>Artist</th><th>Source</th>`;
    if (!catalogTableBody) return;
    catalogTableBody.innerHTML = '';

    hits.forEach((hit: any, idx: number) => {
      const tr = document.createElement('tr');
      tr.dataset.id = hit.id;
      tr.dataset.index = String(idx);
      tr.tabIndex = 0;
      if (idx === 0) tr.classList.add('selected');

      const ccliBadge = hit.ccli_number
        ? `<span style="background: rgba(0, 229, 255, 0.15); color: #00e5ff; border: 1px solid rgba(0, 229, 255, 0.4); font-size: 9.5px; padding: 2px 6px; border-radius: 3px; font-weight: 600; margin-left: 6px;">CCLI #${lp.escapeHtml(hit.ccli_number)}</span>`
        : '';

      tr.innerHTML = `
        <td style="font-weight: 600; color: #ffd700;">✨ ${lp.escapeHtml(hit.title)}</td>
        <td>${lp.escapeHtml(hit.artist)}</td>
        <td>
          <span style="background: rgba(255,215,0,0.15); color: #ffd700; border: 1px solid rgba(255,215,0,0.4); font-size: 9.5px; padding: 2px 6px; border-radius: 3px; font-weight: 600;">GENIUS</span>
          ${ccliBadge}
        </td>
      `;

      tr.addEventListener('click', () => {
        highlightCatalogRow(idx);
        renderGeniusPreview(hit);
      });

      tr.addEventListener('dblclick', () => {
        importAndStageGeniusSong(hit);
      });

      tr.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        e.stopPropagation();
        highlightCatalogRow(idx);
        lp.setContextMenuTarget({ type: 'genius-hit', hit: hit });
        lp.showContextMenu(document.getElementById('library-item-context-menu'), e.clientX, e.clientY);
      });

      catalogTableBody.appendChild(tr);
    });

    if (hits.length > 0) renderGeniusPreview(hits[0]);
  } else {
    if (catalogTable) catalogTable.style.display = 'none';
    if (catalogGrid) catalogGrid.style.display = 'grid';
    if (!catalogGrid) return;
    catalogGrid.innerHTML = '';

    hits.forEach((hit: any, idx: number) => {
      const card = document.createElement('div');
      card.className = `catalog-grid-card ${idx === 0 ? 'selected' : ''}`;
      card.dataset.id = hit.id;
      card.tabIndex = 0;

      const thumbStyle = hit.thumbnail
        ? `background-image: url('${escapeCssUrl(hit.thumbnail)}'); background-size: cover; background-position: center;`
        : `background: linear-gradient(135deg, #1f1c18, #473e34);`;

      const ccliTag = hit.ccli_number ? `#${hit.ccli_number}` : 'GENIUS';

      card.innerHTML = `
        <div class="grid-card-thumb" style="${thumbStyle}">
          <span style="position: absolute; top: 4px; right: 4px; background: rgba(0,0,0,0.7); color: #ffd700; font-size: 8.5px; padding: 1px 4px; border-radius: 2px;">${lp.escapeHtml(ccliTag)}</span>
          ${hit.thumbnail ? '' : '✨'}
        </div>
        <div class="grid-card-title">${lp.escapeHtml(hit.title)}</div>
      `;

      card.addEventListener('click', () => {
        document.querySelectorAll('.catalog-grid-card').forEach(c => c.classList.remove('selected'));
        card.classList.add('selected');
        renderGeniusPreview(hit);
      });

      card.addEventListener('dblclick', () => {
        importAndStageGeniusSong(hit);
      });

      catalogGrid.appendChild(card);
    });

    if (hits.length > 0) renderGeniusPreview(hits[0]);
  }
}

export function renderGeniusPreview(hit: any) {
  if (!resourcePreviewMonitor) return;
  if (!ctx) return;
  const ccliHtml = hit.ccli_number
    ? `<div style="font-size: 11px; color: #00e5ff; font-weight: 600; margin-bottom: 8px;">🔢 CCLI Song #: ${ctx.escapeHtml(hit.ccli_number)}</div>`
    : '';

  resourcePreviewMonitor.innerHTML = `
    <div class="asset-preview-content" style="background: linear-gradient(135deg, #181512, #2e261f, #141b24); font-family: Segoe UI, sans-serif; display: flex; flex-direction: column; justify-content: center; align-items: center; text-align: center; padding: 18px; color: #fff;">
      <div class="preview-badge" style="background: rgba(255,215,0,0.2); color: #ffd700; border: 1px solid #ffd700; margin-bottom: 8px; font-size: 10px; padding: 2px 8px; border-radius: 10px; font-weight: 700;">✨ GENIUS CHRISTIAN LYRICS</div>
      <div style="font-size: 18px; font-weight: 700; margin-bottom: 4px;">${ctx.escapeHtml(hit.title)}</div>
      <div style="font-size: 13px; color: #ffd700; margin-bottom: 6px;">${ctx.escapeHtml(hit.artist)}</div>
      ${ccliHtml}
      <p style="font-size: 11.5px; color: #bbb; line-height: 1.5; max-width: 320px; margin-bottom: 14px;">
        Double-click or click below to automatically import all formatted lyrics slides into your Library & Schedule.
      </p>
      <button class="btn btn-primary" id="btn-preview-import-genius" style="background: linear-gradient(135deg, #ffd700, #ffaa00); color: #000; font-weight: 700; border: none; padding: 6px 16px; border-radius: 4px; cursor: pointer;">
        📥 Import & Stage Lyrics
      </button>
    </div>
  `;

  const btn = document.getElementById('btn-preview-import-genius');
  if (btn) {
    btn.addEventListener('click', () => importAndStageGeniusSong(hit));
  }
}

export async function importAndStageGeniusSong(hit: any, goLive: boolean = false) {
  if (!ctx) return;
  const lp = ctx;
  try {
    const res = await fetch('/api/songs/genius/import', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ url: hit.url, title: hit.title, artist: hit.artist })
    });
    if (res.ok) {
      const song = await res.json();
      currentSearchModes.songs = 'all';
      activeCategory = 'all';
      if (resourceSearchInput) resourceSearchInput.value = '';
      updateSearchModeUI();
      await loadLibraryTab('songs');
      addItemToSchedule('songs', song.id);
      if (goLive) {
        setTimeout(() => lp.sendCommand({ GoLive: { item_index: null, slide_index: 0 } }), 100);
      }
      const ccliMsg = song.ccli_number ? ` (CCLI #${song.ccli_number})` : '';
      lp.showToast(`✓ Imported "${song.title}"${ccliMsg} with ${song.slides.length} slides from Genius!`, 'success');
    } else {
      const err = await res.text();
      lp.showToast(`Could not import lyrics from Genius: ${err}`, 'error');
    }
  } catch (e) {
    lp.showToast(`Import error: ${e instanceof Error ? e.message : String(e)}`, 'error');
  }
}

// ============================================================================
// ONLINE IMAGE SEARCH (Pexels / Pixabay) — Media tab "Online Images" category
// ============================================================================
/**
 * Called from app_ui.ts's search-box Enter handler. Must be a function (not a
 * bare variable read) — app_core.ts and app_ui.ts are separate ES modules;
 * cross-file *variable* reads like `ctx!.getCurrentTab()`/`activeCategory` silently
 * resolve to a stale globalThis snapshot taken once at initial load (see the
 * `try { (globalThis as any).x = x } catch {}` exposals below), never the
 * live value. Calling into a function defined here works correctly because
 * the function body closes over the real, live module-scoped variables.
 * Returns true if this was an online-images search (caller should stop).
 */
export function tryHandleOnlineImagesEnter(): boolean {
  if (!ctx) return false;
  if (ctx.getCurrentTab() === 'media' && activeCategory === 'online-images') {
    const q = resourceSearchInput ? resourceSearchInput.value.trim() : '';
    if (q) searchOnlineImages(q);
    return true;
  }
  return false;
}

export async function searchOnlineImages(query: string) {
  if (!ctx) return;
  const lp = ctx;
  // Triggered only by pressing Enter (see app_ui.ts's keydown handler) — these
  // hit paid/rate-limited external APIs, so there's no per-keystroke debounce
  // needed here, unlike the Genius search this was originally modeled on.
  if (catalogTable) catalogTable.style.display = 'none';
  if (catalogGrid) {
    catalogGrid.style.display = 'grid';
    catalogGrid.innerHTML = `
      <div style="grid-column: 1 / -1; padding: 30px; text-align: center; color: #64b5f6; font-weight: 500;">
        <span style="font-size: 18px;">🌐</span> Searching Pexels &amp; Pixabay for "${lp.escapeHtml(query)}"...
      </div>`;
  }

  try {
    const res = await fetch(`/api/media/online/search?q=${encodeURIComponent(query)}`);
    if (res.ok) {
      const data = await res.json();
      const results = (data && data.results) || [];
      if (results.length === 0) {
        if (catalogGrid) {
          catalogGrid.innerHTML = `
            <div style="grid-column: 1 / -1; padding: 30px; text-align: center; color: var(--text-dim);">
              <p>No online images found for "${lp.escapeHtml(query)}".</p>
              <p style="font-size: 11px; margin-top: 4px;">Check that a Pexels and/or Pixabay API key is configured in Options, and try another keyword.</p>
            </div>`;
        }
      } else {
        renderOnlineImageHits(results);
      }
    } else {
      if (catalogGrid) catalogGrid.innerHTML = `<div style="grid-column: 1 / -1; padding: 20px; text-align: center; color: #ff5252;">Error searching online images: ${lp.escapeHtml(res.statusText)}</div>`;
    }
  } catch (e) {
    if (catalogGrid) catalogGrid.innerHTML = `<div style="grid-column: 1 / -1; padding: 20px; text-align: center; color: #ff5252;">Network error: ${lp.escapeHtml(e instanceof Error ? e.message : String(e))}</div>`;
  }
}

export function renderOnlineImageHits(results: any[]) {
  if (!ctx) return;
  const lp = ctx;
  if (catalogItemCountEl) catalogItemCountEl.textContent = `${results.length} online images`;
  if (!catalogGrid) return;
  catalogGrid.innerHTML = '';

  results.forEach((hit, idx) => {
    const card = document.createElement('div');
    card.className = `catalog-grid-card ${idx === 0 ? 'selected' : ''}`;
    card.dataset.id = hit.id;
    card.tabIndex = 0;

    const providerLabel = (hit.provider || '').toUpperCase();
    const cardLabel = hit.tags || hit.photographer || providerLabel;
    card.innerHTML = `
      <div class="grid-card-thumb" style="background-image: url('${escapeCssUrl(hit.thumbnail_url)}'); background-size: cover; background-position: center;">
        <span style="position: absolute; top: 4px; right: 4px; background: rgba(0,0,0,0.7); color: #64b5f6; font-size: 8.5px; padding: 1px 4px; border-radius: 2px;">${lp.escapeHtml(providerLabel)}</span>
      </div>
      <div class="grid-card-title" title="${lp.escapeHtml(cardLabel)}">${lp.escapeHtml(cardLabel)}</div>
    `;

    card.addEventListener('click', () => {
      document.querySelectorAll('.catalog-grid-card').forEach(c => c.classList.remove('selected'));
      card.classList.add('selected');
      renderOnlineImagePreview(hit);
    });

    card.addEventListener('dblclick', () => {
      importOnlineImage(hit);
    });

    catalogGrid.appendChild(card);
  });

  if (results.length > 0) renderOnlineImagePreview(results[0]);
}

export function renderOnlineImagePreview(hit: any) {
  if (!resourcePreviewMonitor) return;
  if (!ctx) return;
  const attribution = hit.photographer
    ? `Photo by ${ctx.escapeHtml(hit.photographer)} on ${ctx.escapeHtml((hit.provider || '').charAt(0).toUpperCase() + (hit.provider || '').slice(1))}`
    : ctx.escapeHtml(hit.provider || '');
  const tagsHtml = hit.tags
    ? `<div style="font-size: 9.5px; color: #64b5f6; margin-bottom: 4px;">🏷️ ${ctx.escapeHtml(hit.tags)}</div>`
    : '';

  resourcePreviewMonitor.innerHTML = `
    <div class="asset-preview-content" style="background-image: url('${escapeCssUrl(hit.thumbnail_url)}'); background-size: cover; background-position: center; display: flex; flex-direction: column; justify-content: flex-end; align-items: center; text-align: center; padding: 12px; color: #fff;">
      <div style="background: rgba(0,0,0,0.65); border-radius: 4px; padding: 8px 12px; width: 100%;">
        ${tagsHtml}
        <div style="font-size: 10.5px; color: #ddd; margin-bottom: 8px;">${attribution}</div>
        <button class="btn btn-primary" id="btn-preview-import-online-image" style="background: linear-gradient(135deg, #64b5f6, #2979ff); color: #fff; font-weight: 700; border: none; padding: 6px 16px; border-radius: 4px; cursor: pointer;">
          📥 Add to Media Library
        </button>
      </div>
    </div>
  `;

  const btn = document.getElementById('btn-preview-import-online-image');
  if (btn) btn.addEventListener('click', () => importOnlineImage(hit));
}

export async function importOnlineImage(hit: any) {
  if (!ctx) return;
  const lp = ctx;
  try {
    lp.showToast(`Downloading image from ${hit.provider}...`, 'info');
    const res = await fetch('/api/media/online/import', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ result: hit })
    });
    if (res.ok) {
      const mediaItem = await res.json();
      lp.showToast(`✓ Added "${mediaItem.name}" to your Media library!`, 'success');
      if (lp.getCurrentTab() === 'media') {
        try {
          const listRes = await fetch('/api/media');
          if (listRes.ok) activeLibraryItems = await listRes.json();
        } catch (_) {}
      }
    } else {
      const err = await res.text();
      lp.showToast(`Could not import image: ${err}`, 'error');
    }
  } catch (e) {
    lp.showToast(`Import error: ${e instanceof Error ? e.message : String(e)}`, 'error');
  }
}

export function filterAndRenderCatalog() {
  if (!ctx) return;
  const lp = ctx;
  const rawQuery = resourceSearchInput ? resourceSearchInput.value.trim() : '';
  const query = rawQuery.toLowerCase();
  if (btnSearchClear) btnSearchClear.style.display = rawQuery ? 'block' : 'none';

  const mode = currentSearchModes[lp.getCurrentTab()] || 'all';
  let matchedOnlineReference = null;

  if (lp.getCurrentTab() === 'media' && activeCategory === 'online-images') {
    if (!query) {
      if (catalogItemCountEl) catalogItemCountEl.textContent = 'Online Image Search';
      if (catalogGrid) { catalogGrid.style.display = 'grid'; catalogGrid.innerHTML = `
        <div style="grid-column: 1 / -1; padding: 40px 20px; text-align: center; color: var(--text-dim);">
          <div style="font-size: 28px; margin-bottom: 8px;">🌐</div>
          <p style="font-size: 14px; font-weight: 600; color: #64b5f6;">Online Images (Pexels &amp; Pixabay)</p>
          <p style="font-size: 11px; margin-top: 4px; max-width: 360px; margin-left: auto; margin-right: auto;">
            Type a keyword above (e.g. "sunrise", "worship", "cross") and press <strong>Enter</strong> to search free stock photos. Click a thumbnail to preview, double-click to add it to your Media library.
          </p>
        </div>`; }
      if (catalogTable) catalogTable.style.display = 'none';
      if (resourcePreviewMonitor) {
        resourcePreviewMonitor.innerHTML = '<div class="asset-preview-empty">Search Pexels/Pixabay for images</div>';
      }
      return;
    }
    // Typing alone doesn't search this category (it hits paid/rate-limited
    // external APIs) — only Enter (wired in app_ui.ts's keydown handler) does.
    // Leave whatever's already on screen (a prior search, or the hint above)
    // untouched until the user submits.
    return;
    return;
  }

  if (lp.getCurrentTab() === 'songs' && (mode === 'genius' || activeCategory === 'genius-christian')) {
    if (!query) {
      if (catalogItemCountEl) catalogItemCountEl.textContent = 'Genius Search';
      const container = (activeResourceViewMode === 'table') ? catalogTableBody : catalogGrid;
      if (container) {
        container.innerHTML = `
          <div style="padding: 40px 20px; text-align: center; color: var(--text-dim);">
            <div style="font-size: 28px; margin-bottom: 8px;">✨</div>
            <p style="font-size: 14px; font-weight: 600; color: #ffd700;">Genius Christian & Worship Lyrics</p>
            <p style="font-size: 11px; margin-top: 4px; max-width: 360px; margin-left: auto; margin-right: auto;">
              Type a song title or artist in the search bar above to search Genius for Christian lyrics (e.g. "Holy Forever", "Way Maker", "Goodness of God", "10,000 Reasons").
            </p>
          </div>`;
      }
      if (resourcePreviewMonitor) {
        resourcePreviewMonitor.innerHTML = '<div class="asset-preview-empty">Search Genius for Christian lyrics</div>';
      }
      return;
    }
    searchGeniusChristianSongs(rawQuery);
    return;
  }

  if (!query) {
    if (lp.getCurrentTab() === 'scriptures') {
      if (lp.getActiveBibleVersion() === 'all') {
        filteredLibraryItems = [...activeLibraryItems];
      } else {
        filteredLibraryItems = activeLibraryItems.filter(sc => scriptureMatchesVersion(sc, lp.getActiveBibleVersion()));
        if (filteredLibraryItems.length === 0) filteredLibraryItems = [...activeLibraryItems];
      }
    } else if (lp.getCurrentTab() === 'media') {
      const effectiveCategory = (activeCategory && activeCategory !== 'all' && activeCategory !== 'ytdlp-import')
        ? activeCategory
        : mode;
      filteredLibraryItems = activeLibraryItems.filter(m => {
        const mTypeLower = (m.media_type || '').toLowerCase();
        if (effectiveCategory === 'videos') return mTypeLower.includes('video');
        if (effectiveCategory === 'images') return mTypeLower.includes('image') || mTypeLower.includes('still');
        if (effectiveCategory === 'audio') return mTypeLower.includes('audio');
        if (effectiveCategory === 'feeds') return mTypeLower.includes('feed') || mTypeLower.includes('camera');
        return true;
      });
    } else {
      filteredLibraryItems = [...activeLibraryItems];
    }
  } else if (lp.getCurrentTab() === 'songs') {
    if (rawQuery.length >= 2) {
      matchedOnlineReference = { type: 'genius', query: rawQuery };
    }
    filteredLibraryItems = activeLibraryItems.filter(s => {
      const matchTitle = (s.title || '').toLowerCase().includes(query) || (s.alternate_title || '').toLowerCase().includes(query);
      const matchAuthor = (s.author || '').toLowerCase().includes(query);
      const matchCcli = (s.ccli_number || '').toLowerCase().includes(query) || (s.id || '').toLowerCase().includes(query);
      const matchLyrics = s.slides && s.slides.some((sl: any) => (sl.text || '').toLowerCase().includes(query));

      if (mode === 'title') return matchTitle;
      if (mode === 'lyrics') return matchLyrics;
      if (mode === 'ccli') return matchCcli;
      return matchTitle || matchAuthor || matchCcli || matchLyrics;
    });
  } else if (lp.getCurrentTab() === 'scriptures') {
    if (mode === 'reference') {
      const parsedRef = parseScriptureReference(rawQuery);
      if (parsedRef.isReference) {
        matchedOnlineReference = parsedRef;
        const matchingItems: any[] = [];
        const refBook = parsedRef.book;
        activeLibraryItems.forEach((sc: any) => {
          if (!refBook) return;
          if (!scriptureMatchesVersion(sc, lp.getActiveBibleVersion())) return;
          const matchBook = (sc.book || '').toLowerCase() === refBook.toLowerCase();
          if (!matchBook) return;
          if (parsedRef.chapter && sc.chapter !== parsedRef.chapter) return;

          if (parsedRef.hasSpecificVerse) {
            const startV = parsedRef.verseStart;
            if (startV === undefined || startV === null) return;
            const endV = parsedRef.verseEnd || startV;
            if (sc.verse_start <= startV && sc.verse_end >= startV) {
              const specificVerses = (sc.verses || []).filter((v: any) => v.verse_number >= startV && v.verse_number <= endV);
              if (specificVerses.length > 0) {
                const cleanBook = sc.book;
                const refStr = (startV === endV)
                  ? `${cleanBook} ${sc.chapter}:${startV} (${sc.version})`
                  : `${cleanBook} ${sc.chapter}:${startV}-${endV} (${sc.version})`;
                const specificItem = {
                  ...sc,
                  id: `scrip_${cleanBook.replace(/\s+/g, '_')}_${sc.chapter}_${startV}_${endV}_${(sc.version || 'KJV').replace(/\s+/g, '_')}`,
                  verse_start: startV,
                  verse_end: endV,
                  reference: refStr,
                  verses: specificVerses
                };
                matchingItems.push(specificItem);
                return;
              }
            }
          }
          if (!parsedRef.hasSpecificVerse) {
            matchingItems.push(sc);
          }
        });
        filteredLibraryItems = matchingItems;
      } else {
        filteredLibraryItems = activeLibraryItems.filter(sc => {
          const matchVersion = scriptureMatchesVersion(sc, lp.getActiveBibleVersion());
          return matchVersion && ((sc.reference || '').toLowerCase().includes(query)
            || (sc.book || '').toLowerCase().includes(query));
        });
      }
    } else {
      const qTokens = query.split(/\s+/).filter(Boolean);
      const scored: any[] = [];
      const seenIds = new Set();

      activeLibraryItems.forEach((sc: any) => {
        if (!scriptureMatchesVersion(sc, lp.getActiveBibleVersion())) return;
        if (!sc.verses || sc.verses.length === 0) return;

        sc.verses.forEach((v: any) => {
          const textLower = (v.text || '').toLowerCase();
          let score = 0;
          if (textLower.includes(query)) {
            score += 1000;
          }
          if (qTokens.length > 1 && qTokens.every(t => textLower.includes(t))) {
            score += 400;
          }
          qTokens.forEach(t => {
            if (textLower.includes(t)) score += 50;
          });

          if (score > 0) {
            const cleanBook = sc.book;
            const refStr = `${cleanBook} ${sc.chapter}:${v.verse_number} (${sc.version})`;
            const itemId = `scrip_${cleanBook.replace(/\s+/g, '_')}_${sc.chapter}_${v.verse_number}_${v.verse_number}_${(sc.version || 'KJV').replace(/\s+/g, '_')}`;
            if (!seenIds.has(itemId)) {
              seenIds.add(itemId);
              const specificItem = {
                ...sc,
                id: itemId,
                verse_start: v.verse_number,
                verse_end: v.verse_number,
                reference: refStr,
                verses: [v]
              };
              scored.push({ item: specificItem, score });
            }
          }
        });
      });

      scored.sort((a, b) => b.score - a.score);
      filteredLibraryItems = scored.map(s => s.item);
    }
  } else if (lp.getCurrentTab() === 'media') {
    filteredLibraryItems = activeLibraryItems.filter((m: any) => {
      const matchName = (m.name || '').toLowerCase().includes(query);
      const matchPath = (m.file_path || '').toLowerCase().includes(query);
      const matchType = (m.media_type || '').toLowerCase().includes(query);
      const matchesSearch = matchName || matchPath || matchType;

      const mTypeLower = (m.media_type || '').toLowerCase();
      const effectiveCategory = (activeCategory && activeCategory !== 'all' && activeCategory !== 'ytdlp-import')
        ? activeCategory
        : mode;

      if (effectiveCategory === 'videos') return matchesSearch && mTypeLower.includes('video');
      if (effectiveCategory === 'images') return matchesSearch && (mTypeLower.includes('image') || mTypeLower.includes('still'));
      if (effectiveCategory === 'audio') return matchesSearch && mTypeLower.includes('audio');
      if (effectiveCategory === 'feeds') return matchesSearch && (mTypeLower.includes('feed') || mTypeLower.includes('camera'));
      return matchesSearch;
    });
  } else if (lp.getCurrentTab() === 'presentations') {
    filteredLibraryItems = activeLibraryItems.filter(p => {
      const matchTitle = (p.title || '').toLowerCase().includes(query);
      const matchAuthor = (p.author || '').toLowerCase().includes(query);
      const matchSlides = p.slides && p.slides.some((s: any) => (s.content || s.title || '').toLowerCase().includes(query));
      if (mode === 'title') return matchTitle;
      return matchTitle || matchAuthor || matchSlides;
    });
  } else if (lp.getCurrentTab() === 'themes') {
    const categoryMap: Record<string, string> = { 'song-themes': 'song', 'scripture-themes': 'scripture', 'presentation-themes': 'presentation' };
    const wantedCategory = categoryMap[activeCategory];
    filteredLibraryItems = activeLibraryItems.filter((t: any) => {
      if (wantedCategory && (t.category || 'song') !== wantedCategory) return false;
      const matchName = (t.name || '').toLowerCase().includes(query);
      const matchFont = (t.font_family || '').toLowerCase().includes(query);
      const matchBg = (t.background || '').toLowerCase().includes(query);
      if (mode === 'title') return matchName;
      return matchName || matchFont || matchBg;
    });
  }

  renderCatalog(filteredLibraryItems, lp.getCurrentTab(), matchedOnlineReference);
}

let currentCatalogItems: any[] = [];
let catalogVisibleCount = 100;
const CATALOG_BATCH_SIZE = 100;

export function updateCatalogCountDisplay() {
  if (!catalogItemCountEl) return;
  if (!ctx) return;
  const total = currentCatalogItems.length;
  const tabLabel = ctx.getCurrentTab() || 'items';
  if (total > catalogVisibleCount) {
    catalogItemCountEl.textContent = `Showing ${catalogVisibleCount} of ${total} ${tabLabel}`;
  } else {
    catalogItemCountEl.textContent = `${total} ${tabLabel}`;
  }
}

export function loadMoreCatalogItems() {
  if (!ctx) return;
  const lp = ctx;
  if (catalogVisibleCount >= currentCatalogItems.length) return;
  const start = catalogVisibleCount;
  const end = Math.min(start + CATALOG_BATCH_SIZE, currentCatalogItems.length);
  if (start >= end) return;

  const nextBatch = currentCatalogItems.slice(start, end);
  catalogVisibleCount = end;
  updateCatalogCountDisplay();

  if (activeResourceViewMode === 'table') {
    appendCatalogTableRows(nextBatch, lp.getCurrentTab(), start);
  } else {
    appendCatalogGridCards(nextBatch, lp.getCurrentTab(), start);
  }
}

// Attach autoload scroll listener
const catalogScrollBodyEl = document.querySelector('.catalog-scroll-body');
if (catalogScrollBodyEl) {
  catalogScrollBodyEl.addEventListener('scroll', () => {
    if (catalogVisibleCount >= currentCatalogItems.length) return;
    const { scrollTop, scrollHeight, clientHeight } = catalogScrollBodyEl;
    if (scrollTop + clientHeight >= scrollHeight - 120) {
      loadMoreCatalogItems();
    }
  });
}

export function renderCatalog(items: any[], tabName: string, matchedOnlineRef: any = null) {
  if (!ctx) return;
  const lp = ctx;
  currentCatalogItems = items;
  catalogVisibleCount = Math.min(CATALOG_BATCH_SIZE, items.length);

  updateCatalogCountDisplay();

  if (catalogScrollBodyEl) {
    catalogScrollBodyEl.scrollTop = 0;
  }

  if (activeResourceViewMode === 'table') {
    if (catalogTable) catalogTable.style.display = 'table';
    if (catalogGrid) catalogGrid.style.display = 'none';
    renderCatalogTable(items.slice(0, catalogVisibleCount), tabName, matchedOnlineRef);
  } else {
    if (catalogTable) catalogTable.style.display = 'none';
    if (catalogGrid) catalogGrid.style.display = 'grid';
    renderCatalogGrid(items.slice(0, catalogVisibleCount), tabName);
  }

  if (items.length > 0) {
    // Preserve selection if previously selected item is still in filtered results
    let targetIdx = 0;
    if (lp.getSelectedLibraryItem()) {
      const foundIdx = items.findIndex(it => it.id === lp.getSelectedLibraryItem().id);
      if (foundIdx !== -1) {
        targetIdx = foundIdx;
      }
    }
    selectedCatalogIndex = targetIdx;
    selectAndPreviewItem(items[targetIdx], tabName);
  } else if (!matchedOnlineRef) {
    if (resourcePreviewMonitor) {
      resourcePreviewMonitor.innerHTML = '<div class="asset-preview-empty">No items match query</div>';
    }
  }
}

export function createCatalogTableRow(item: any, idx: number, tabName: string) {
  if (!ctx) return document.createElement('tr');
  const lp = ctx;
  const tr = document.createElement('tr');
  tr.dataset.id = item.id;
  tr.dataset.index = String(idx);
  tr.tabIndex = 0;
  if (idx === selectedCatalogIndex) tr.classList.add('selected');

  if (tabName === 'songs') {
    tr.innerHTML = `
      <td style="font-weight: 600; color: #fff;">🎵 ${lp.escapeHtml(item.title)}</td>
      <td>${lp.escapeHtml(item.author || '')}</td>
      <td style="color: var(--text-dim); font-size: 10px;">${lp.escapeHtml(item.ccli_number || item.copyright || '')}</td>
    `;
  } else if (tabName === 'scriptures') {
    const snippet = item.verses && item.verses.length > 0 ? item.verses[0].text : '';
    tr.innerHTML = `
      <td style="font-weight: 600; color: #fff;">📖 ${lp.escapeHtml(item.reference)}</td>
      <td><span class="bible-lang-badge">${lp.escapeHtml(item.version)}</span></td>
      <td style="color: var(--text-dim); font-size: 10.5px; max-width: 200px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${lp.escapeHtml(snippet)}</td>
    `;
  } else if (tabName === 'media') {
    tr.innerHTML = `
      <td style="font-weight: 600; color: #fff;">🎬 ${lp.escapeHtml(item.name)}</td>
      <td>${lp.escapeHtml(item.media_type || 'Image')}</td>
      <td>${item.duration_seconds ? item.duration_seconds + 's' : 'Still'}</td>
    `;
  } else if (tabName === 'presentations') {
    tr.innerHTML = `
      <td style="font-weight: 600; color: #fff;">📊 ${lp.escapeHtml(item.title)}</td>
      <td>${lp.escapeHtml(item.author || 'Media')}</td>
      <td>${item.slides ? item.slides.length : 1} slides</td>
    `;
  } else if (tabName === 'themes') {
    tr.innerHTML = `
      <td style="font-weight: 600; color: #fff;">🎨 ${lp.escapeHtml(item.name)}</td>
      <td>${lp.escapeHtml(item.font_family || 'Segoe UI')} (${item.font_size || 38}px)</td>
      <td>${lp.escapeHtml(item.background_type || 'Gradient')}</td>
    `;
  }

  tr.addEventListener('click', () => {
    selectedCatalogIndex = idx;
    highlightCatalogRow(idx);
    selectAndPreviewItem(item, tabName);
  });

  // Register Pragmatic DnD Draggable on Table Row
  catalogDndCleanups.push(
    draggable({
      element: tr,
      getInitialData: () => ({
        type: tabName === 'media' ? 'media-item' : (tabName === 'themes' ? 'theme-item' : 'catalog-item'),
        tabName: tabName,
        mediaType: item.media_type || 'Image',
        filePath: item.file_path || '',
        themeName: item.name || '',
        bg: item.background || item.file_path || '',
        name: item.name || item.title || item.reference || '',
        title: item.title || item.name || item.reference || '',
        id: item.id,
        itemId: item.id
      }),
      onDragStart: () => { tr.style.opacity = '0.5'; },
      onDrop: () => { tr.style.opacity = '1'; }
    })
  );

  tr.draggable = true;
  tr.addEventListener('dragstart', (e) => {
    if (!e.dataTransfer) return;
    e.dataTransfer.setData('application/json', JSON.stringify({
      type: tabName === 'media' ? 'media-item' : (tabName === 'themes' ? 'theme-item' : 'catalog-item'),
      tabName: tabName,
      mediaType: item.media_type || 'Image',
      filePath: item.file_path || '',
      themeName: item.name || '',
      bg: item.background || item.file_path || '',
      name: item.name || item.title || item.reference || '',
      title: item.title || item.name || item.reference || '',
      id: item.id
    }));
    e.dataTransfer.setData('text/plain', item.file_path || item.id);
    e.dataTransfer.effectAllowed = 'copyMove';
    tr.style.opacity = '0.5';
  });
  tr.addEventListener('dragend', () => {
    tr.style.opacity = '1';
  });

  tr.addEventListener('dblclick', () => {
    addItemToSchedule(tabName, item.id);
  });

  // Right-Click Context Menu on Library Table Row
  tr.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    e.stopPropagation();
    selectedCatalogIndex = idx;
    highlightCatalogRow(idx);
    selectAndPreviewItem(item, tabName);
    lp.setContextMenuTarget({ type: 'library-item', tab: tabName, item: item });
    lp.showContextMenu(document.getElementById('library-item-context-menu'), e.clientX, e.clientY);
  });

  tr.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      if (idx === catalogVisibleCount - 1 && catalogVisibleCount < currentCatalogItems.length) {
        loadMoreCatalogItems();
      }
      const next = Math.min(catalogVisibleCount - 1, idx + 1);
      highlightCatalogRow(next);
      if (currentCatalogItems[next]) selectAndPreviewItem(currentCatalogItems[next], tabName);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      const prev = Math.max(0, idx - 1);
      highlightCatalogRow(prev);
      if (currentCatalogItems[prev]) selectAndPreviewItem(currentCatalogItems[prev], tabName);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      if (e.ctrlKey) {
        addItemToSchedule(tabName, item.id);
        setTimeout(() => lp.sendCommand({ GoLive: { item_index: null, slide_index: 0 } }), 100);
      } else {
        addItemToSchedule(tabName, item.id);
      }
    }
  });

  return tr;
}

export function appendCatalogTableRows(items: any[], tabName: string, startIndex: number) {
  if (!catalogTableBody) return;
  items.forEach((item, i) => {
    const tr = createCatalogTableRow(item, startIndex + i, tabName);
    catalogTableBody.appendChild(tr);
  });
}

export function renderCatalogTable(items: any[], tabName: string, matchedOnlineRef: any = null) {
  if (!ctx) return;
  const headers = document.getElementById('catalog-headers');
  if (!catalogTableBody) return;

  // Clean up previous Pragmatic DND listeners on catalog items
  catalogDndCleanups.forEach(fn => { try { fn(); } catch (_) {} });
  catalogDndCleanups = [];

  catalogTableBody.innerHTML = '';

  if (headers) {
    if (tabName === 'songs') headers.innerHTML = `<th>Title</th><th>Author / Details</th><th>CCLI # / Copyright</th>`;
    else if (tabName === 'scriptures') headers.innerHTML = `<th>Reference</th><th>Version</th><th>Verses / Snippet</th>`;
    else if (tabName === 'media') headers.innerHTML = `<th>Name</th><th>Type</th><th>Duration</th>`;
    else if (tabName === 'presentations') headers.innerHTML = `<th>Title</th><th>Author</th><th>Slides</th>`;
    else if (tabName === 'themes') headers.innerHTML = `<th>Name</th><th>Font / Layout</th><th>Type</th>`;
  }

  if (matchedOnlineRef && tabName === 'songs' && matchedOnlineRef.type === 'genius') {
    const promptTr = document.createElement('tr');
    promptTr.className = 'catalog-fetch-prompt-row';
    promptTr.tabIndex = 0;
    promptTr.innerHTML = `
      <td colspan="3" style="padding: 10px; color: #ffd700; font-weight: 600; cursor: pointer; background: rgba(255,215,0,0.06);">
        ✨ Search Genius for Christian Lyrics: "${ctx.escapeHtml(matchedOnlineRef.query)}" ➔
      </td>
    `;
    promptTr.addEventListener('click', () => {
      currentSearchModes.songs = 'genius';
      updateSearchModeUI();
      filterAndRenderCatalog();
    });
    promptTr.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        currentSearchModes.songs = 'genius';
        updateSearchModeUI();
        filterAndRenderCatalog();
      }
    });
    if (items.length === 0) {
      catalogTableBody.appendChild(promptTr);
      return;
    } else {
      catalogTableBody.appendChild(promptTr);
    }
  }

  if (matchedOnlineRef && items.length === 0 && tabName === 'scriptures') {
    const fetchTrans = ctx.getActiveBibleVersion() === 'all' ? (ctx.getAppOptions().defaultBibleVersion || 'kjv') : ctx.getActiveBibleVersion();
    const activeObj = ctx.getInstalledBibles().find(b => b.id === fetchTrans) || ctx.getInstalledBibles()[0];
    const promptTr = document.createElement('tr');
    promptTr.className = 'catalog-fetch-prompt-row';
    promptTr.tabIndex = 0;
    promptTr.innerHTML = `
      <td colspan="3" style="padding: 10px; color: #00e5ff; font-weight: 600;">
        🌐 Fetch & Stage "${ctx.escapeHtml(matchedOnlineRef.formatted)}" [${ctx.escapeHtml(activeObj.abbreviation)}] from Online Scripture API ➔
      </td>
    `;
    promptTr.addEventListener('click', () => fetchAndStageOnlineScripture(matchedOnlineRef.formatted, fetchTrans));
    promptTr.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') fetchAndStageOnlineScripture(matchedOnlineRef.formatted, fetchTrans);
    });
    catalogTableBody.appendChild(promptTr);
    return;
  }

  appendCatalogTableRows(items, tabName, 0);
}

export function highlightCatalogRow(idx: number) {
  if (!catalogTableBody) return;
  selectedCatalogIndex = idx;
  // Exclude the decorative ".catalog-fetch-prompt-row" (Genius/online-fetch
  // prompt), which `renderCatalogTable` prepends ahead of the real item
  // rows -- otherwise `idx` (an item index, same semantics `createCatalogTableRow`
  // uses) would be off by one against raw DOM row position whenever that
  // prompt row is present.
  const rows = catalogTableBody.querySelectorAll<HTMLElement>('tr:not(.catalog-fetch-prompt-row)');
  rows.forEach((r, i) => {
    r.classList.toggle('selected', i === idx);
    if (i === idx) r.focus();
  });
}

export function createCatalogGridCard(item: any, idx: number, tabName: string) {
  if (!ctx) return document.createElement('div');
  const lp = ctx;
  const card = document.createElement('div');
  card.className = 'catalog-grid-card';
  card.dataset.id = item.id;
  card.dataset.index = String(idx);
  card.tabIndex = 0;
  if (idx === selectedCatalogIndex) card.classList.add('selected');

  let icon = '🎵';
  let bgStyle = 'linear-gradient(135deg, #0f2027, #203a43)';
  let title = item.title || item.name || item.reference || 'Item';

  if (tabName === 'themes') {
    icon = '🎨';
    bgStyle = item.background || bgStyle;
  } else if (tabName === 'scriptures') {
    icon = '📖';
    bgStyle = 'linear-gradient(135deg, #1f1c18, #473e34)';
  } else if (tabName === 'presentations') {
    icon = '📊';
    bgStyle = 'linear-gradient(135deg, #2c3e50, #3498db)';
  } else if (tabName === 'media') {
    icon = '🎬';
    bgStyle = 'linear-gradient(135deg, #141e30, #243b55)';
  }

  const defaultBadge = (tabName === 'themes' && item.is_default) ? '<span class="grid-card-default-badge" title="Default theme for this category">⭐</span>' : '';
  card.innerHTML = `
    <div class="grid-card-thumb" style="background: ${escapeHtml(bgStyle)};">${icon}${defaultBadge}</div>
    <div class="grid-card-title">${lp.escapeHtml(title)}</div>
  `;

  if (tabName === 'themes') {
    card.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      contextMenuTargetTheme = item;
      lp.showContextMenu(document.getElementById('theme-context-menu'), e.clientX, e.clientY);
    });
  }

  card.addEventListener('click', () => {
    selectedCatalogIndex = idx;
    document.querySelectorAll('.catalog-grid-card').forEach(c => c.classList.remove('selected'));
    card.classList.add('selected');
    selectAndPreviewItem(item, tabName);
  });

  if (tabName === 'themes') {
    card.addEventListener('dblclick', () => openThemeEditor(item));
  }

  // Register Pragmatic DnD Draggable on Grid Card
  catalogDndCleanups.push(
    draggable({
      element: card,
      getInitialData: () => ({
        type: tabName === 'media' ? 'media-item' : (tabName === 'themes' ? 'theme-item' : 'catalog-item'),
        tabName: tabName,
        mediaType: item.media_type || 'Image',
        filePath: item.file_path || '',
        themeName: item.name || '',
        bg: item.background || item.file_path || '',
        name: item.name || item.title || item.reference || '',
        title: item.title || item.name || item.reference || '',
        id: item.id,
        itemId: item.id
      }),
      onDragStart: () => { card.style.opacity = '0.5'; },
      onDrop: () => { card.style.opacity = '1'; }
    })
  );

  card.draggable = true;
  card.addEventListener('dragstart', (e) => {
    if (!e.dataTransfer) return;
    e.dataTransfer.setData('application/json', JSON.stringify({
      type: tabName === 'media' ? 'media-item' : (tabName === 'themes' ? 'theme-item' : 'catalog-item'),
      tabName: tabName,
      mediaType: item.media_type || 'Image',
      filePath: item.file_path || '',
      themeName: item.name || '',
      bg: item.background || item.file_path || '',
      name: item.name || item.title || item.reference || '',
      title: item.title || item.name || item.reference || '',
      id: item.id
    }));
    e.dataTransfer.setData('text/plain', item.file_path || item.id);
    e.dataTransfer.effectAllowed = 'copyMove';
    card.style.opacity = '0.5';
  });
  card.addEventListener('dragend', () => {
    card.style.opacity = '1';
  });

  card.addEventListener('dblclick', () => {
    // Themes get their own dblclick (open the Theme Editor) registered above —
    // "adding a theme to the schedule" isn't a real action, unlike every other tab.
    if (tabName === 'themes') return;
    addItemToSchedule(tabName, item.id);
  });

  // Right-Click Context Menu on Library Grid Card
  card.addEventListener('contextmenu', (e) => {
    // Themes get their own dedicated context menu (theme-context-menu) registered
    // above — this generic one's "Set Theme/Background", "Copy Content", etc. don't
    // apply to a Theme record itself.
    if (tabName === 'themes') return;
    e.preventDefault();
    e.stopPropagation();
    selectedCatalogIndex = idx;
    document.querySelectorAll('.catalog-grid-card').forEach(c => c.classList.remove('selected'));
    card.classList.add('selected');
    selectAndPreviewItem(item, tabName);
    lp.setContextMenuTarget({ type: 'library-item', tab: tabName, item: item });
    lp.showContextMenu(document.getElementById('library-item-context-menu'), e.clientX, e.clientY);
  });

  card.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      addItemToSchedule(tabName, item.id);
    }
  });

  return card;
}

export function appendCatalogGridCards(items: any[], tabName: string, startIndex: number) {
  if (!catalogGrid) return;
  items.forEach((item, i) => {
    const card = createCatalogGridCard(item, startIndex + i, tabName);
    catalogGrid.appendChild(card);
  });
}

export function renderCatalogGrid(items: any[], tabName: string) {
  if (!catalogGrid) return;

  // Clean up previous Pragmatic DND listeners on catalog items
  catalogDndCleanups.forEach(fn => { try { fn(); } catch (_) {} });
  catalogDndCleanups = [];

  catalogGrid.innerHTML = '';
  appendCatalogGridCards(items, tabName, 0);
}

const STAGEABLE_TAB_TYPES: Record<string, string> = {
  songs: 'song',
  scriptures: 'scripture',
  presentations: 'presentation',
  media: 'media'
};

/** Previewing a song/scripture/presentation/media library item stages it
 * through the real backend (same {item_type, item_id} shorthand as
 * AddToSchedule, resolved server-side via ToScheduleItem so it picks up the
 * same auto-split as actually scheduling it) rather than the local-only
 * "ad-hoc" projection — so the Preview panel shows the same slides,
 * double-click-to-Live and the Go Live button work, exactly as they do for
 * a real staged schedule item. Themes (not a schedulable item type) still
 * use the lightweight ad-hoc preview. */
export function selectAndPreviewItem(item: any, tabName: string) {
  if (!ctx) return;
  ctx.setSelectedLibraryItem(item);
  renderAssetPreview(item, tabName);
  const itemType = STAGEABLE_TAB_TYPES[tabName];
  if (itemType && item && item.id) {
    ctx.setAdhocPreview(null, '');
    ctx.sendCommand({ StageItem: { item_type: itemType, item_id: item.id } });
  } else {
    ctx.setAdhocPreview(item, tabName);
  }
}

export function renderAssetPreview(item: any, tabName: string) {
  if (!item || !resourcePreviewMonitor) return;
  if (!ctx) return;
  const lp = ctx;

  let title = item.title || item.name || item.reference || '';
  let author = item.author || item.version || '';
  let sampleText = '';
  let bgStyle = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)';
  let theme: any = null;

  if (item.slides && item.slides[0] && item.slides[0].background) {
    bgStyle = item.slides[0].background;
  } else if (item.theme_name) {
    const t = lp.getAvailableThemes().find(x => x.name === item.theme_name);
    if (t) { bgStyle = t.bg; theme = t; }
  }

  if (tabName === 'scriptures' && item.verses && item.verses.length > 0) {
    const qLower = (resourceSearchInput ? resourceSearchInput.value : '').trim().toLowerCase();
    let targetVerseIdx = 0;
    if (qLower && item.verses.length > 1) {
      const matchIdx = item.verses.findIndex((v: any) => (v.text || '').toLowerCase().includes(qLower));
      if (matchIdx !== -1) targetVerseIdx = matchIdx;
    }
    const v1 = item.verses[targetVerseIdx] || item.verses[0];
    const w1 = (v1.text || '').trim().split(/\s+/).filter(Boolean).length;
    if (w1 < 20 && item.verses.length > targetVerseIdx + 1) {
      const v2 = item.verses[targetVerseIdx + 1];
      const w2 = (v2.text || '').trim().split(/\s+/).filter(Boolean).length;
      if (w1 + w2 <= 40) {
        sampleText = `${v1.verse_number}. ${v1.text}\n\n${v2.verse_number}. ${v2.text}`;
      } else {
        sampleText = `${v1.verse_number}. ${v1.text}`;
      }
    } else {
      sampleText = `${v1.verse_number}. ${v1.text}`;
    }
  } else if (tabName === 'themes') {
    bgStyle = item.background || bgStyle;
    sampleText = `Sample Theme Typography\n${item.name}`;
    theme = lp.getAvailableThemes().find(x => x.name === item.name) || null;
  }

  // Songs/presentations preview their first real `Slide` record directly
  // (same shape Preview/Live render) so positioned elements, background_v2,
  // and reference_label all come along for free; scriptures/themes synthesize
  // an equivalent flat-text slide from the sample text computed above.
  const previewSlide = ((tabName === 'songs' || tabName === 'presentations') && item.slides && item.slides[0])
    ? item.slides[0]
    : { text: sampleText, elements: [], background: bgStyle };

  // Dual Translation Comparison Render in Scriptures Tab
  if (tabName === 'scriptures' && lp.getIsDualBibleMode()) {
    const pVersion = (item.version || (lp.getActiveBibleVersion() !== 'all' ? lp.getActiveBibleVersion() : (lp.getInstalledBibles()[0] ? lp.getInstalledBibles()[0].abbreviation : 'Primary'))).trim();
    const pAbbr = getBibleAbbreviation(pVersion);
    
    const otherInstalled = lp.getInstalledBibles().filter(b => 
      !areTranslationsEquivalent(b.id, pVersion) &&
      !areTranslationsEquivalent(b.abbreviation, pVersion) &&
      !areTranslationsEquivalent(b.name, pVersion)
    );
    const standardOnline = ['kjv', 'asv', 'web', 'bbe', 'hcsb'];

    if (!lp.getSecondaryBibleVersion() || 
        areTranslationsEquivalent(lp.getSecondaryBibleVersion(), pVersion) ||
        areTranslationsEquivalent(lp.getSecondaryBibleVersion(), item.version)) {
      if (otherInstalled.length > 0) {
        lp.setSecondaryBibleVersion(otherInstalled[0].id);
      } else {
        const fallback = standardOnline.find(code => !areTranslationsEquivalent(code, pVersion));
        lp.setSecondaryBibleVersion(fallback || 'asv');
      }
    }
    const sAbbr = getBibleAbbreviation(lp.getSecondaryBibleVersion() || 'Parallel');

    if (btnToggleDualBible) {
      btnToggleDualBible.textContent = `👥 Dual: ${pAbbr} | ${sAbbr}`;
    }

    // Clean reference built directly from book, chapter, and verse bounds
    const cleanRef = (item.book && item.chapter)
      ? `${item.book} ${item.chapter}:${item.verse_start}${item.verse_end && item.verse_end !== item.verse_start ? '-' + item.verse_end : ''}`
      : (item.reference || '').replace(/\s*\([^)]*\)\s*/g, '').trim();

    let pickerOptions = '';
    lp.getInstalledBibles().forEach(b => {
      const isPrimary = areTranslationsEquivalent(b.id, pVersion) || areTranslationsEquivalent(b.abbreviation, pVersion) || areTranslationsEquivalent(b.name, pVersion);
      if (!isPrimary) {
        const isSel = areTranslationsEquivalent(b.id, lp.getSecondaryBibleVersion()) || areTranslationsEquivalent(b.abbreviation, lp.getSecondaryBibleVersion());
        pickerOptions += `<option value="${lp.escapeHtml(b.id)}" ${isSel ? 'selected' : ''}>${lp.escapeHtml(b.abbreviation)} (Installed)</option>`;
      }
    });

    resourcePreviewMonitor.innerHTML = `
      <div class="canvas-16-9" style="background: linear-gradient(135deg, #181512, #362f27); position: relative; display: flex; flex-direction: column; overflow: hidden;">
        <div class="canvas-dual-split">
          <div class="canvas-dual-col">
            <div class="canvas-dual-header">
              <div class="canvas-dual-ref">${lp.escapeHtml(cleanRef)}</div>
              <div class="canvas-dual-badge-row">
                <span class="canvas-dual-badge primary">${lp.escapeHtml(pAbbr)}</span>
              </div>
            </div>
            <div class="canvas-dual-text">${lp.escapeHtml(sampleText).replace(/\n/g, '<br>')}</div>
          </div>
          <div class="canvas-dual-col secondary-col">
            <div class="canvas-dual-header">
              <div class="canvas-dual-ref">${lp.escapeHtml(cleanRef)}</div>
              <div class="canvas-dual-badge-row">
                <select id="dual-secondary-picker" class="canvas-dual-picker">
                  ${pickerOptions}
                </select>
              </div>
            </div>
            <div class="canvas-dual-text" id="dual-secondary-text" style="color: #e0f2fe;">
              Loading ${lp.escapeHtml(sAbbr)} translation...
            </div>
          </div>
        </div>
        <div class="canvas-footer" style="display: flex; align-items: center; justify-content: space-between; height: 30px; padding: 0 8px; box-sizing: border-box;">
          <span style="font-size: 10px;">Dual: <strong style="color:#ffa726">${lp.escapeHtml(pAbbr)}</strong> + <strong style="color:#00e5ff">${lp.escapeHtml(sAbbr)}</strong></span>
          <button id="btn-add-dual-to-schedule" class="btn" style="background: linear-gradient(135deg, #00e5ff, #00b0ff); color: #000; font-weight: 800; padding: 2px 8px; font-size: 10.5px; border: none; border-radius: 3px; cursor: pointer;">
            ➕ Add to Schedule
          </button>
        </div>
      </div>
    `;

    const picker = document.getElementById('dual-secondary-picker');
    if (picker) {
      picker.addEventListener('change', (e) => {
        e.stopPropagation();
        lp.setSecondaryBibleVersion((e.target as HTMLSelectElement).value);
        const newSAbbr = getBibleAbbreviation(lp.getSecondaryBibleVersion());
        if (btnToggleDualBible) {
          btnToggleDualBible.textContent = `👥 Dual: ${pAbbr} | ${newSAbbr}`;
        }
        renderAssetPreview(item, 'scriptures');
      });
    }

    const addBtn = document.getElementById('btn-add-dual-to-schedule');
    if (addBtn) {
      addBtn.addEventListener('click', (e) => {
        e.stopPropagation();
        addDualScriptureToSchedule(item, pVersion, lp.getSecondaryBibleVersion(), item.verses, currentDualSecondaryVerses);
      });
    }

    // 100% Local SQLite Database passage lookup
    const passageUrl = `/api/bibles/passage?book=${encodeURIComponent(item.book || '')}&chapter=${item.chapter || 1}&version=${encodeURIComponent(lp.getSecondaryBibleVersion())}`;
    fetch(passageUrl)
      .then(r => r.json())
      .then(localData => {
        const el = document.getElementById('dual-secondary-text');
        if (localData && localData.verses && localData.verses.length > 0) {
          currentDualSecondaryVerses = localData.verses;
          const matchingVerses = localData.verses.filter((v: any) =>
            v.verse_number >= item.verse_start && v.verse_number <= item.verse_end
          );
          const vList = matchingVerses.length > 0 ? matchingVerses : localData.verses;
          
          const qLower = (resourceSearchInput ? resourceSearchInput.value : '').trim().toLowerCase();
          let targetIdx = 0;
          if (qLower && vList.length > 1) {
            const foundIdx = vList.findIndex((v: any) => (v.text || '').toLowerCase().includes(qLower));
            if (foundIdx !== -1) targetIdx = foundIdx;
          }
          
          const sv1 = vList[targetIdx] || vList[0];
          const sw1 = (sv1.text || '').trim().split(/\s+/).filter(Boolean).length;
          let sText = `${sv1.verse_number}. ${sv1.text}`;
          if (sw1 < 20 && vList.length > targetIdx + 1) {
            const sv2 = vList[targetIdx + 1];
            const sw2 = (sv2.text || '').trim().split(/\s+/).filter(Boolean).length;
            if (sw1 + sw2 <= 40) {
              sText = `${sv1.verse_number}. ${sv1.text}\n\n${sv2.verse_number}. ${sv2.text}`;
            }
          }
          
          if (el) el.innerHTML = lp.escapeHtml(sText).replace(/\n/g, '<br>');
        } else {
          if (el) el.textContent = `[${sAbbr} translation not available locally]`;
        }
      })
      .catch(err => {
        const el = document.getElementById('dual-secondary-text');
        if (el) el.textContent = `[${sAbbr} error: ${err.message}]`;
      });

    return;
  }

  if (tabName === 'media') {
    const mType = (item.media_type || '').toLowerCase();
    if (mType.includes('video')) {
      resourcePreviewMonitor.innerHTML = `
        <div class="canvas-16-9" style="background: #000; position: relative; display: flex; flex-direction: column; align-items: center; justify-content: center; overflow: hidden;">
          <video id="preview-video-element" src="${lp.escapeHtml(item.file_path)}" muted playsinline preload="metadata" style="width: 100%; height: 100%; object-fit: contain;"></video>
          <div class="preview-video-control-bar" style="position: absolute; bottom: 22px; left: 0; right: 0; background: rgba(10, 15, 20, 0.88); display: flex; align-items: center; gap: 6px; padding: 4px 8px; z-index: 5; border-top: 1px solid rgba(255,255,255,0.1);">
            <button class="btn" id="btn-preview-video-play" style="padding: 2px 8px; font-size: 10px; background: #00e5ff; color: #000; font-weight: 700; border: none; border-radius: 3px; cursor: pointer;">▶ Play</button>
            <button class="btn" id="btn-preview-video-stop" style="padding: 2px 8px; font-size: 10px; background: #37474f; color: #fff; font-weight: 700; border: none; border-radius: 3px; cursor: pointer;">⏹ Stop</button>
            <input type="range" id="preview-video-seek" min="0" max="100" value="0" style="flex: 1; height: 4px; cursor: pointer; accent-color: #00e5ff;">
            <span id="preview-video-time" style="font-size: 10px; color: #cfd8dc; font-variant-numeric: tabular-nums;">0:00 / 0:00</span>
            <button class="btn" id="btn-preview-video-mute" title="Mute/Unmute Preview Audio" style="background: transparent; border: none; font-size: 12px; cursor: pointer; color: #fff; padding: 0 4px;">🔊</button>
          </div>
          <div class="canvas-footer" style="background: rgba(0,0,0,0.85); position: absolute; bottom: 0; left: 0; right: 0; z-index: 4;">
            <span>🎬 ${lp.escapeHtml(item.name)}</span>
            <span>${lp.escapeHtml(item.media_type || 'Media')} ${item.duration_seconds ? '(' + item.duration_seconds + 's)' : ''}</span>
          </div>
        </div>
      `;

      const pVid = document.getElementById('preview-video-element') as HTMLVideoElement | null;
      const pPlayBtn = document.getElementById('btn-preview-video-play');
      const pStopBtn = document.getElementById('btn-preview-video-stop');
      const pSeek = document.getElementById('preview-video-seek') as HTMLInputElement | null;
      const pTime = document.getElementById('preview-video-time');
      const pMuteBtn = document.getElementById('btn-preview-video-mute');

      if (pVid && pPlayBtn && pStopBtn) {
        pVid.muted = false; // Allow audio in console preview
        pVid.autoplay = false;

        if (pMuteBtn) {
          pMuteBtn.addEventListener('click', () => {
            pVid.muted = !pVid.muted;
            pMuteBtn.textContent = pVid.muted ? '🔇' : '🔊';
          });
        }

        pPlayBtn.addEventListener('click', () => {
          if (pVid.paused) {
            pVid.play().then(() => {
              pPlayBtn.textContent = '⏸ Pause';
            }).catch(() => {});
          } else {
            pVid.pause();
            pPlayBtn.textContent = '▶ Play';
          }
        });

        pStopBtn.addEventListener('click', () => {
          pVid.pause();
          pVid.currentTime = 0;
          pPlayBtn.textContent = '▶ Play';
        });

        pVid.addEventListener('timeupdate', () => {
          if (pVid.duration && !isNaN(pVid.duration)) {
            const pct = (pVid.currentTime / pVid.duration) * 100;
            if (pSeek) pSeek.value = String(pct);
            if (pTime) pTime.textContent = `${lp.formatMediaTime(pVid.currentTime)} / ${lp.formatMediaTime(pVid.duration)}`;
          }
        });

        if (pSeek) {
          pSeek.addEventListener('input', () => {
            if (pVid.duration && !isNaN(pVid.duration)) {
              pVid.currentTime = (Number(pSeek.value) / 100) * pVid.duration;
            }
          });
        }

        pVid.addEventListener('ended', () => {
          pPlayBtn.textContent = '▶ Play';
          pVid.currentTime = 0;
        });
      }
      return;
    } else if (mType.includes('audio')) {
      resourcePreviewMonitor.innerHTML = `
        <div style="display: flex; flex-direction: column; align-items: center; justify-content: center; height: 100%; gap: 10px; padding: 16px; background: #141f26;">
          <div style="font-size: 36px;">🎵</div>
          <div style="font-weight: 700; color: #00e5ff; font-size: 13px; text-align: center; max-width: 90%; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;">${lp.escapeHtml(item.name)}</div>
          <audio controls src="${lp.escapeHtml(item.file_path)}" style="width: 85%; height: 32px;"></audio>
        </div>`;
      return;
    } else {
      resourcePreviewMonitor.innerHTML = `
        <div class="canvas-16-9" style="background: #000; position: relative; display: flex; align-items: center; justify-content: center;">
          <img src="${lp.escapeHtml(item.file_path)}" style="width: 100%; height: 100%; object-fit: contain;">
          <div class="canvas-footer" style="background: rgba(0,0,0,0.75); position: absolute; bottom: 0; left: 0; right: 0;">
            <span>🖼️ ${lp.escapeHtml(item.name)}</span>
            <span>Image</span>
          </div>
        </div>
      `;
      return;
    }
  }

  // Same scaffold (canvas box / elements overlay / lyrics container) and the
  // same canonical renderer (core/slide_render.ts) Preview/Live use, instead
  // of a hand-rolled escaper + a font-size lookup table keyed on character
  // count — which can never generalize across box sizes, and never rendered
  // positioned elements or background_v2 at all.
  resourcePreviewMonitor.innerHTML = `
    <div class="canvas-16-9" id="resource-preview-canvas">
      <div id="resource-preview-canvas-elements" style="display: none; position: absolute; inset: 0; z-index: 2; pointer-events: none;"></div>
      <div class="canvas-lyrics-container">
        <div class="canvas-lyrics" id="resource-preview-canvas-lyrics"></div>
      </div>
      <div class="canvas-footer">
        <span>${lp.escapeHtml(title)}</span>
        <span>${lp.escapeHtml(author)}</span>
      </div>
    </div>
  `;

  const previewCanvasEl = document.getElementById('resource-preview-canvas') as HTMLElement | null;
  const previewElementsEl = document.getElementById('resource-preview-canvas-elements') as HTMLElement | null;
  const previewLyricsEl = document.getElementById('resource-preview-canvas-lyrics') as HTMLElement | null;
  if (previewCanvasEl && previewLyricsEl) {
    resolveSlideBackgroundElement(previewCanvasEl, previewSlide);
    renderSlideVisual(previewCanvasEl, previewElementsEl, previewLyricsEl, previewSlide, theme);
  }
}

export async function fetchAndStageOnlineScripture(query: string, translation: string = 'kjv') {
  if (!ctx) return;
  try {
    const res = await fetch('/api/bibles/online/fetch', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ query, translation })
    });
    if (res.ok) {
      const data = await res.json();
      await loadLibraryTab('scriptures');
      const targetId = data.id || (data.items && data.items.length > 0 ? data.items[0].id : null);
      if (targetId) {
        addItemToSchedule('scriptures', targetId);
      }
    } else {
      ctx.showToast('Could not fetch online scripture for: ' + query, 'error');
    }
  } catch (e) {
    ctx.showToast('Fetch error: ' + (e instanceof Error ? e.message : String(e)), 'error');
  }
}

let lastAddedItemGuard = { id: '', time: 0 };

export async function addItemToSchedule(tabName: string, itemId: string) {
  if (!ctx) return;
  const now = Date.now();
  if (lastAddedItemGuard.id === itemId && (now - lastAddedItemGuard.time) < 400) {
    console.warn(`[addItemToSchedule] Ignored duplicate add for ${itemId} within ${now - lastAddedItemGuard.time}ms`);
    return;
  }
  lastAddedItemGuard = { id: itemId, time: now };

  const typeMap: Record<string, string> = {
    songs: 'song',
    scriptures: 'scripture',
    presentations: 'presentation',
    media: 'media',
    themes: 'theme'
  };
  const itemType = typeMap[tabName] || 'song';

  if (tabName === 'scriptures' || itemType === 'scripture') {
    const found = (filteredLibraryItems || []).find(it => it.id === itemId) || (activeLibraryItems || []).find(it => it.id === itemId);
    if (found && found.verses && found.verses.length > 0) {
      try {
        await fetch('/api/scriptures', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(found)
        });
      } catch (e) {
        console.warn('Auto-persisting specific scripture verse:', e);
      }
    }
  }

  ctx.sendCommand({ AddToSchedule: { item_type: itemType, item_id: itemId } });
}

export async function addDualScriptureToSchedule(primaryItem: any, pVersionParam: any, sVersionParam: any, primaryVerses: any[], secondaryVerses: any[]) {
  if (!ctx) return;
  if (!primaryItem) {
    ctx.showToast('No scripture selected to add', 'warning');
    return;
  }

  ctx.showToast('Creating Dual Translation schedule item...', 'info');

  const pVersion = (pVersionParam && pVersionParam !== 'all' ? pVersionParam : (primaryItem.version || (ctx.getInstalledBibles()[0] ? ctx.getInstalledBibles()[0].id : 'KJV'))).trim();
  const sVersion = (sVersionParam && sVersionParam !== 'all' ? sVersionParam : (ctx.getSecondaryBibleVersion() || (ctx.getInstalledBibles()[1] ? ctx.getInstalledBibles()[1].id : (ctx.getInstalledBibles()[0] ? ctx.getInstalledBibles()[0].id : 'ASV')))).trim();

  const pAbbr = getBibleAbbreviation(pVersion);
  const sAbbr = getBibleAbbreviation(sVersion);

  const cleanRef = (primaryItem.book && primaryItem.chapter)
    ? `${primaryItem.book} ${primaryItem.chapter}${primaryItem.verse_start ? ':' + primaryItem.verse_start : ''}${primaryItem.verse_end && primaryItem.verse_end !== primaryItem.verse_start ? '-' + primaryItem.verse_end : ''}`
    : (primaryItem.reference || '').replace(/\s*\([^)]*\)\s*/g, '').trim();

  let pList = (primaryVerses && primaryVerses.length > 0) ? [...primaryVerses] : (primaryItem.verses ? [...primaryItem.verses] : []);
  let sList = (secondaryVerses && secondaryVerses.length > 0) ? [...secondaryVerses] : (currentDualSecondaryVerses ? [...currentDualSecondaryVerses] : []);

  // 1. If primary verses list is empty, fetch locally from SQLite via /api/bibles/passage
  if (pList.length === 0 && primaryItem.book && primaryItem.chapter) {
    try {
      const pRes = await fetch(`/api/bibles/passage?book=${encodeURIComponent(primaryItem.book)}&chapter=${primaryItem.chapter}&version=${encodeURIComponent(pVersion)}`);
      if (pRes.ok) {
        const pData = await pRes.json();
        if (pData && pData.verses && pData.verses.length > 0) {
          pList = pData.verses;
        }
      }
    } catch (e) {
      console.warn('Could not pre-fetch primary verses locally:', e);
    }
  }

  // 2. If secondary verses list is empty, fetch locally from SQLite via /api/bibles/passage
  if (sList.length === 0 && primaryItem.book && primaryItem.chapter) {
    try {
      const sRes = await fetch(`/api/bibles/passage?book=${encodeURIComponent(primaryItem.book)}&chapter=${primaryItem.chapter}&version=${encodeURIComponent(sVersion)}`);
      if (sRes.ok) {
        const sData = await sRes.json();
        if (sData && sData.verses && sData.verses.length > 0) {
          sList = sData.verses;
          currentDualSecondaryVerses = sData.verses;
        }
      }
    } catch (e) {
      console.warn('Could not pre-fetch secondary verses locally:', e);
    }
  }

  // Filter primary verses to bounds if specified
  if (primaryItem.verse_start && pList.length > 0) {
    const vStart = primaryItem.verse_start;
    const vEnd = primaryItem.verse_end || vStart;
    const filtered = pList.filter(v => v.verse_number >= vStart && v.verse_number <= vEnd);
    if (filtered.length > 0) pList = filtered;
  }

  // Fallback if still empty
  if (pList.length === 0) {
    pList = [{
      verse_number: primaryItem.verse_start || 1,
      text: primaryItem.text || primaryItem.name || cleanRef
    }];
  }

  const pLabel = pAbbr;
  const sLabel = sAbbr;
  const presentationTitle = `${cleanRef} [${pLabel} | ${sLabel}]`;

  const slides = [];
  let i = 0;
  while (i < pList.length) {
    const pv1 = pList[i];
    const sv1 = sList.find(v => v.verse_number === pv1.verse_number) || sList[i] || { verse_number: pv1.verse_number, text: pv1.text };

    const pw1 = (pv1.text || '').trim().split(/\s+/).filter(Boolean).length;
    const sw1 = (sv1.text || '').trim().split(/\s+/).filter(Boolean).length;

    if (pw1 < 20 && sw1 < 20 && i + 1 < pList.length) {
      const pv2 = pList[i + 1];
      const sv2 = sList.find(v => v.verse_number === pv2.verse_number) || sList[i + 1] || { verse_number: pv2.verse_number, text: pv2.text };
      const pw2 = (pv2.text || '').trim().split(/\s+/).filter(Boolean).length;
      const sw2 = (sv2.text || '').trim().split(/\s+/).filter(Boolean).length;

      if (pw1 + pw2 <= 40 && sw1 + sw2 <= 40) {
        const slideTitle = `${cleanRef}:${pv1.verse_number}-${pv2.verse_number}`;
        const pText = `${pv1.verse_number}. ${pv1.text}\n\n${pv2.verse_number}. ${pv2.text}`;
        const sText = `${sv1.verse_number || pv1.verse_number}. ${sv1.text}\n\n${sv2.verse_number || pv2.verse_number}. ${sv2.text}`;

        slides.push({
          id: 'slide_' + Math.random().toString(36).substr(2, 9),
          title: slideTitle,
          content: formatParallelSlide(pLabel, pText, sLabel, sText),
          notes: null,
          custom_background: null
        });
        i += 2;
        continue;
      }
    }

    // Single verse slide
    const slideTitle = `${cleanRef}:${pv1.verse_number}`;
    const pText = `${pv1.verse_number}. ${pv1.text}`;
    const sText = `${sv1.verse_number || pv1.verse_number}. ${sv1.text}`;

    slides.push({
      id: 'slide_' + Math.random().toString(36).substr(2, 9),
      title: slideTitle,
      content: formatParallelSlide(pLabel, pText, sLabel, sText),
      notes: null,
      custom_background: null
    });
    i += 1;
  }

  if (slides.length === 0) {
    ctx.showToast('No verses available to create dual comparison', 'warning');
    return;
  }

  const payload = {
    id: 'pres_' + Date.now() + '_' + Math.random().toString(36).substr(2, 6),
    title: presentationTitle,
    author: `Dual: ${pLabel} + ${sLabel}`,
    slides: slides,
    modified_date: new Date().toISOString()
  };

  try {
    const res = await fetch('/api/presentations', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    if (res.ok) {
      const pres = await res.json();
      addItemToSchedule('presentations', pres.id);
      ctx.showToast(`✓ Added Dual Comparison (${pLabel} | ${sLabel}) to schedule!`, 'success');
      filterAndRenderCatalog();
    } else {
      const errText = await res.text();
      ctx.showToast('Failed to add dual comparison: ' + (errText || res.statusText), 'error');
    }
  } catch (err) {
    ctx.showToast('Error adding dual comparison: ' + (err instanceof Error ? err.message : String(err)), 'error');
  }
}
