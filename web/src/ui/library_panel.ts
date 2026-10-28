/**
 * OpenSanctuary / OS-Next Library / Resource Panel Controller
 * Extracted from app_core.ts (ongoing modularization pass) — browsing, searching,
 * and adding songs/scriptures/media/presentations/themes to the schedule: the
 * Bible translation selector and the table/grid catalog renderers.
 *
 * Cross-cutting state (currentTab, selectedLibraryItem, activeBibleVersion,
 * secondaryBibleVersion, isDualBibleMode, installedBibles) is still owned by
 * app_core.ts — it's also read by code outside this panel (the schedule/library
 * item context-menu dispatcher) — and reached here through the context object,
 * following the same pattern settings_dialog.ts/theme_picker.ts/arrangement_modal.ts
 * already use, so this stays a one-way dependency (app_core.ts imports from here,
 * never the reverse).
 */
import { errorMessage } from '../core/ui_utils.ts';
import type { ThemeDefinition } from '../core/presentation_helpers.ts';
import type { LibrarySlide, ScriptureVerse, LibraryItem, ShowCommand, InstalledBible, StateSnapshot } from '../core/models.ts';
import { draggable } from '@atlaskit/pragmatic-drag-and-drop/element/adapter';
import { parseScriptureReference } from '../core/bible_parser';
import { escapeHtml, escapeUserHtml, formatParallelSlide, resolveSlideBackgroundElement } from '../core/presentation_helpers.ts';
import { renderSlideVisual } from '../core/slide_render.ts';
import { openThemeEditor, setThemeAsDefault, duplicateTheme, deleteThemeItem } from './theme_editor.ts';

export interface LibraryPanelContext {
  getCurrentTab(): string;
  setCurrentTab(tab: string): void;
  getSelectedLibraryItem(): LibraryItem | null;
  setSelectedLibraryItem(item: LibraryItem): void;
  getActiveBibleVersion(): string;
  setActiveBibleVersion(v: string): void;
  getSecondaryBibleVersion(): string;
  setSecondaryBibleVersion(v: string): void;
  getIsDualBibleMode(): boolean;
  setIsDualBibleMode(v: boolean): void;
  getInstalledBibles(): InstalledBible[];
  getAppOptions(): Record<string, any>;
  getAvailableThemes(): ThemeDefinition[];
  getCurrentSnapshot(): StateSnapshot | null;
  showToast(message: string, type?: string): void;
  sendCommand(cmd: ShowCommand): void;
  escapeHtml(str: unknown): string;
  formatCssBackground(bg: string | null | undefined, defaultGradient?: string): string;
  showContextMenu(el: HTMLElement | null, x: number, y: number): void;
  setContextMenuTarget(target: any): void;
  setContextMenuTargetBible(bible: any): void;
  setBibleVersionUserSelected(v: boolean): void;
  applyDefaultBibleVersionIfUnset(): void;
  formatMediaTime(sec: number): string;
  setAdhocPreview(item: LibraryItem | null, tabName: string): void;
}

let ctx: LibraryPanelContext | null = null;
let currentDualSecondaryVerses: ScriptureVerse[] = [];

export function getCurrentDualSecondaryVerses(): ScriptureVerse[] {
  return currentDualSecondaryVerses;
}

export function setCurrentDualSecondaryVerses(verses: ScriptureVerse[]): void {
  currentDualSecondaryVerses = verses || [];
}

/** The translation a plugin offers (YouVersion's NIV...) that the Bible context menu was opened on, if any. */
let remoteMenuTarget: any = null;

/**
 * Reference-only results for the translations a plugin offers (YouVersion's NIV...): a search like "John 3:16"
 * lists "John 3:16 (NIV)" for each of them in scope, with no text. The text is fetched when the result is added to
 * the schedule or dragged there (the server makes a placeholder item and the plugin fills it in).
 */
export function remoteReferenceItems(parsedRef: any, activeVersion: unknown): LibraryItem[] {
  if (!ctx || !parsedRef?.isReference || !parsedRef.book || !parsedRef.chapter) return [];
  const all = ctx.getInstalledBibles().filter((b) => b.remote);
  const scope = (!activeVersion || activeVersion === 'all') ? all : all.filter((b) => b.id === activeVersion);
  const from = parsedRef.hasSpecificVerse ? (parsedRef.verseStart || 0) : 0;
  const to = parsedRef.hasSpecificVerse ? (parsedRef.verseEnd || from) : 0;
  const verses = from ? (to > from ? `:${from}-${to}` : `:${from}`) : '';
  return scope.flatMap((b): LibraryItem[] => b.remote ? [{
    id: `yvref:${b.remote.source}:${b.remote.bible_id}:${parsedRef.book}:${parsedRef.chapter}:${from}:${to}`,
    book: parsedRef.book,
    chapter: parsedRef.chapter,
    verse_start: from || 1,
    verse_end: to || from || 1,
    version: b.abbreviation,
    reference: `${parsedRef.book} ${parsedRef.chapter}${verses} (${b.abbreviation})`,
    verses: [],
    remote: b.remote,
  }] : []);
}

export function initLibraryPanel(context: LibraryPanelContext) {
  ctx = context;

  document.getElementById('ctx-remove-remote-bible')?.addEventListener('click', async () => {
    const b = remoteMenuTarget;
    document.getElementById('bible-version-context-menu')!.style.display = 'none';
    if (!b || !ctx) return;
    try {
      const res = await fetch(`/api/bibles/${encodeURIComponent(b.id)}`, { method: 'DELETE' });
      const body = await res.json().catch(() => ({}));
      if (!res.ok || body.success === false) throw new Error(body.error || `status ${res.status}`);
      ctx.showToast(`Removed ${b.abbreviation} from the list.`, 'success');
      window.dispatchEvent(new CustomEvent('os-library-changed', { detail: { category: 'scriptures', bibles: true } }));
    } catch (e) {
      ctx.showToast(`Could not remove ${b.abbreviation}: ${errorMessage(e)}`, 'error');
    }
  });

  // A plugin added something (e.g. an imported song): reload the tab if it's the one on screen.
  window.addEventListener('os-library-changed', (ev) => {
    const category = (ev as CustomEvent).detail?.category;
    // Refresh the list, but don't pull the operator out of the plugin page that just added something.
    if (category && ctx && ctx.getCurrentTab() === category) void loadLibraryTab(category, { keepPluginView: true });
  });

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

let activeLibraryItems: LibraryItem[] = [];
export function getActiveLibraryItems() { return activeLibraryItems; }

let filteredLibraryItems: LibraryItem[] = [];
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
      { id: 'ccli', label: '🔢 CCLI # / Catalog ID' }
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

/**
 * "Save as OpenSanctuary File" applies to songs, presentations and Bibles only; on a Bible it says so, since
 * it saves the whole translation the clicked chapter belongs to.
 */
function prepareBundleMenuItem(tabName: string, item: LibraryItem): void {
  const el = document.getElementById('ctx-lib-export-bundle');
  if (!el) return;
  const bundleable = tabName === 'songs' || tabName === 'presentations' || tabName === 'scriptures';
  el.style.display = bundleable ? '' : 'none';
  el.textContent = tabName === 'scriptures' && item?.version ? `📦 Save the ${item.version} Bible as OpenSanctuary File...` : '📦 Save as OpenSanctuary File...';
}

export async function loadLibraryTab(tabName?: string, opts: { keepPluginView?: boolean } = {}) {
  if (!ctx) return;
  const lp = ctx;
  const normTab = (tabName === 'song' || tabName === 'songs') ? 'songs'
    : ((tabName === 'scripture' || tabName === 'scriptures') ? 'scriptures'
    : ((tabName === 'presentation' || tabName === 'presentations') ? 'presentations'
    : ((tabName === 'theme' || tabName === 'themes') ? 'themes'
    : ((tabName === 'media') ? 'media' : (tabName || 'songs')))));

  lp.setCurrentTab(normTab);
  const keepPlugin = !!opts.keepPluginView && activeCategory.startsWith('plugin:');
  if (!keepPlugin) {
    activeCategory = 'all';
    showLibrarySource(null);
  }
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

// ---- plugin sources: a sandboxed plugin's page shown in place of a Resource Library tab's list ----
interface LibrarySource { id: string; tab: string; label: string }
const librarySources: LibrarySource[] = [];

/** Installed plugins not turned on yet that would put a page in a tab (see core/plugin_hints.ts). */
interface LibrarySourceHint extends LibrarySource { name: string }
let librarySourceHints: LibrarySourceHint[] = [];

export function setLibrarySourceHints(hints: LibrarySourceHint[]) {
  librarySourceHints = hints;
  if (ctx) renderCategoryTree(ctx.getCurrentTab());
}

export function registerLibrarySource(src: LibrarySource) {
  const i = librarySources.findIndex((s) => s.id === src.id);
  if (i >= 0) librarySources[i] = src; else librarySources.push(src);
  if (ctx) renderCategoryTree(ctx.getCurrentTab());
}

export function unregisterLibrarySource(id: string) {
  const i = librarySources.findIndex((s) => s.id === id);
  if (i < 0) return;
  librarySources.splice(i, 1);
  if (activeCategory === `plugin:${id}`) {
    activeCategory = 'all';
    showLibrarySource(null);
    if (ctx) filterAndRenderCatalog();
  }
  if (ctx) renderCategoryTree(ctx.getCurrentTab());
}

/** Shows one plugin's page (or, with null, the normal list and preview). */
function showLibrarySource(id: string | null) {
  const pane = document.querySelector('.plugin-source-pane') as HTMLElement | null;
  const catalog = document.querySelector('.resource-catalog-pane') as HTMLElement | null;
  const preview = document.querySelector('.resource-preview-pane') as HTMLElement | null;
  const on = !!id && !!pane;
  pane?.querySelectorAll<HTMLElement>('.plugin-source').forEach((w) => {
    w.style.display = id && w.dataset.pluginId === id ? 'flex' : 'none';
  });
  if (pane) pane.style.display = on ? 'flex' : 'none';
  if (catalog) catalog.style.display = on ? 'none' : '';
  if (preview) preview.style.display = on ? 'none' : '';
}

/** The sidebar entries for the plugins that put a page in this tab. */
function appendLibrarySourceNodes(tabName: string, container: HTMLElement) {
  for (const src of librarySources.filter((s) => s.tab === tabName)) {
    const nodeEl = document.createElement('div');
    nodeEl.className = `category-node plugin-source-node ${activeCategory === `plugin:${src.id}` ? 'active' : ''}`;
    nodeEl.textContent = src.label;
    nodeEl.addEventListener('click', () => {
      activeCategory = `plugin:${src.id}`;
      document.querySelectorAll('.category-node').forEach((c) => c.classList.remove('active'));
      nodeEl.classList.add('active');
      showLibrarySource(src.id);
    });
    container.appendChild(nodeEl);
  }
  // Plugins that would add an entry here once turned on: a muted entry that opens their review card.
  for (const hint of librarySourceHints.filter((h) => h.tab === tabName && !librarySources.some((s) => s.id === h.id))) {
    const nodeEl = document.createElement('div');
    nodeEl.className = 'category-node plugin-source-node plugin-source-node--off';
    nodeEl.dataset.pluginId = hint.id;
    nodeEl.textContent = hint.label;
    nodeEl.title = `${hint.name} is a plugin that isn't turned on yet. Click to review it in Settings > Plugins.`;
    nodeEl.addEventListener('click', () => {
      window.dispatchEvent(new CustomEvent('os-open-plugins-settings', { detail: { pluginId: hint.id } }));
    });
    container.appendChild(nodeEl);
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
      <div class="bible-tree-node-main">
        <span>📖</span>
        <span class="bible-tree-node-abbr">All Translations</span>
      </div>
      <span class="bible-lang-badge">ALL</span>
    `;
    allNode.addEventListener('click', () => {
      activeCategory = 'all';
      showLibrarySource(null);
      document.querySelectorAll('.category-node').forEach(c => c.classList.remove('active'));
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
    headerNode.className = 'os-eyebrow category-tree-heading';
    headerNode.textContent = 'Installed Versions';
    categoryTreeContainer.appendChild(headerNode);

    const availableBibles = lp.getInstalledBibles().filter(b => (b.verse_count === undefined || b.verse_count > 0));
    if (availableBibles.length === 0) {
      const emptyNote = document.createElement('div');
      emptyNote.className = 'category-tree-note';
      emptyNote.textContent = 'No Bible versions installed.';
      // With nothing to read from, point at whichever plugin provides Bibles (running, or installed but not
      // turned on yet): its entry is in the list below, and this opens it the same way. Generic on purpose --
      // the console doesn't name plugins (core/plugin_hints.ts).
      const bibleEntry = librarySources.find((s) => s.tab === 'scriptures')
        ?? librarySourceHints.find((h) => h.tab === 'scriptures' && !librarySources.some((s) => s.id === h.id));
      if (bibleEntry) {
        const getBtn = document.createElement('button');
        getBtn.type = 'button';
        getBtn.className = 'btn btn-sm btn-primary';
        getBtn.id = 'btn-get-bibles';
        getBtn.style.marginTop = 'var(--os-space-3)';
        getBtn.textContent = `Get Bibles: ${bibleEntry.label}`;
        getBtn.addEventListener('click', () => {
          const node = (categoryTreeContainer.querySelector('.category-tree-actions .plugin-source-node:not(.plugin-source-node--off)')
            ?? categoryTreeContainer.querySelector('.category-tree-actions .plugin-source-node--off')) as HTMLElement | null;
          node?.click();
        });
        emptyNote.appendChild(document.createElement('br'));
        emptyNote.appendChild(getBtn);
      }
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
          <div class="bible-tree-node-main">
            <span class="bible-tree-node-abbr">${lp.escapeHtml(bible.abbreviation || bible.id)}</span>
            <span class="bible-tree-node-name">${lp.escapeHtml(bible.name || bible.id)}</span>
          </div>
          <div class="bible-tree-node-meta">
            ${bible.remote ? `<span class="bible-source-tag" title="Text is fetched from ${lp.escapeHtml(bible.remote.tag)} when a passage is added to the schedule">${lp.escapeHtml(bible.remote.tag)}</span>` : ''}
            ${bible.language ? `<span class="bible-lang-badge">${lp.escapeHtml(bible.language)}</span>` : ''}
            ${isDefault && !bible.remote ? '<span class="bible-default-badge" title="Default Translation">⭐</span>' : ''}
          </div>
        `;

        nodeEl.addEventListener('click', () => {
          lp.setActiveBibleVersion(bible.id);
          lp.setBibleVersionUserSelected(true);
          activeCategory = 'all';
          showLibrarySource(null);
          document.querySelectorAll('.category-node').forEach(c => c.classList.remove('active'));
          document.querySelectorAll('.bible-tree-node').forEach(b => b.classList.remove('active'));
          nodeEl.classList.add('active');
          updateSearchModeUI();
          filterAndRenderCatalog();
        });

        nodeEl.addEventListener('contextmenu', (e) => {
          e.preventDefault();
          e.stopPropagation();
          lp.setContextMenuTargetBible(bible);
          remoteMenuTarget = bible.remote ? bible : null;
          // A translation a plugin offers can't be the default or compared (it has no text of its own); it can be removed.
          for (const id of ['ctx-set-default-bible', 'ctx-compare-bible']) {
            const el = document.getElementById(id);
            if (el) el.style.display = bible.remote ? 'none' : '';
          }
          const rm = document.getElementById('ctx-remove-remote-bible');
          if (rm) rm.style.display = bible.remote ? '' : 'none';
          lp.showContextMenu(document.getElementById('bible-version-context-menu'), e.clientX, e.clientY);
        });

        categoryTreeContainer.appendChild(nodeEl);
      });
    }

    // 3. Plugins that add Bibles (or would, once turned on), at the bottom
    const actionWrap = document.createElement('div');
    actionWrap.className = 'category-tree-actions';
    appendLibrarySourceNodes('scriptures', actionWrap);

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
      { id: 'favorites', label: '⭐ Favorites' }
    ];
  } else if (tabName === 'media') {
    nodes = [
      { id: 'all', label: '🎬 All Media' },
      { id: 'videos', label: '🎥 Videos & Motion Loops' },
      { id: 'images', label: '🖼️ Images & Stills' },
      { id: 'audio', label: '🎵 Audio Tracks' },
      { id: 'feeds', label: '📡 Live Camera Feeds' }
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
      activeCategory = n.id;
      showLibrarySource(null);
      document.querySelectorAll('.category-node').forEach(c => c.classList.remove('active'));
      nodeEl.classList.add('active');
      filterAndRenderCatalog();
    });
    categoryTreeContainer.appendChild(nodeEl);
  });
  appendLibrarySourceNodes(normTab, categoryTreeContainer);
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

export function scriptureMatchesVersion(sc: LibraryItem, versionId: unknown) {
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
    const selectedNow = lp.getSelectedLibraryItem();
    const pVersion = (selectedNow && selectedNow.version ? selectedNow.version : (lp.getActiveBibleVersion() !== 'all' ? lp.getActiveBibleVersion() : (lp.getInstalledBibles()[0]?.abbreviation ?? 'Primary'))).trim();

    if (lp.getIsDualBibleMode()) {
      const otherInstalled = lp.getInstalledBibles().filter(b =>
        !areTranslationsEquivalent(b.id, pVersion) &&
        !areTranslationsEquivalent(b.abbreviation, pVersion) &&
        !areTranslationsEquivalent(b.name, pVersion)
      );
      if (!lp.getSecondaryBibleVersion() || areTranslationsEquivalent(lp.getSecondaryBibleVersion(), pVersion)) {
        if (otherInstalled.length > 0) {
          if (otherInstalled[0]) lp.setSecondaryBibleVersion(otherInstalled[0].id);
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
    const selectedPreview = lp.getSelectedLibraryItem();
    if (selectedPreview) renderAssetPreview(selectedPreview, 'scriptures');
  });
}

// As-You-Type Query Engine & Filtering Logic

// As-You-Type Query Engine & Filtering Logic

export function filterAndRenderCatalog() {
  if (!ctx) return;
  const lp = ctx;
  const rawQuery = resourceSearchInput ? resourceSearchInput.value.trim() : '';
  const query = rawQuery.toLowerCase();
  if (btnSearchClear) btnSearchClear.style.display = rawQuery ? 'block' : 'none';

  const mode = currentSearchModes[lp.getCurrentTab()] || 'all';
  let matchedOnlineReference = null;

  if (!query) {
    if (lp.getCurrentTab() === 'scriptures') {
      if (lp.getActiveBibleVersion() === 'all') {
        filteredLibraryItems = [...activeLibraryItems];
      } else {
        filteredLibraryItems = activeLibraryItems.filter(sc => scriptureMatchesVersion(sc, lp.getActiveBibleVersion()));
        if (filteredLibraryItems.length === 0) filteredLibraryItems = [...activeLibraryItems];
      }
    } else if (lp.getCurrentTab() === 'media') {
      const effectiveCategory = (activeCategory && activeCategory !== 'all')
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
    filteredLibraryItems = activeLibraryItems.filter(s => {
      const matchTitle = (s.title || '').toLowerCase().includes(query) || (s.alternate_title || '').toLowerCase().includes(query);
      const matchAuthor = (s.author || '').toLowerCase().includes(query);
      const matchCcli = (s.ccli_number || '').toLowerCase().includes(query) || (s.id || '').toLowerCase().includes(query);
      const matchLyrics = s.slides && s.slides.some((sl: LibrarySlide) => (sl.text || '').toLowerCase().includes(query));

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
        const matchingItems: LibraryItem[] = [];
        const refBook = parsedRef.book;
        activeLibraryItems.forEach((sc) => {
          if (!refBook) return;
          if (!scriptureMatchesVersion(sc, lp.getActiveBibleVersion())) return;
          const matchBook = (sc.book || '').toLowerCase() === refBook.toLowerCase();
          if (!matchBook) return;
          if (parsedRef.chapter && sc.chapter !== parsedRef.chapter) return;

          if (parsedRef.hasSpecificVerse) {
            const startV = parsedRef.verseStart;
            if (startV === undefined || startV === null) return;
            const endV = parsedRef.verseEnd || startV;
            if ((sc.verse_start ?? 0) <= startV && (sc.verse_end ?? 0) >= startV) {
              const specificVerses = (sc.verses || []).filter((v: ScriptureVerse) => v.verse_number >= startV && v.verse_number <= endV);
              if (specificVerses.length > 0) {
                const cleanBook = sc.book ?? '';
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
        matchingItems.push(...remoteReferenceItems(parsedRef, lp.getActiveBibleVersion()));
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

      activeLibraryItems.forEach((sc) => {
        if (!scriptureMatchesVersion(sc, lp.getActiveBibleVersion())) return;
        if (!sc.verses || sc.verses.length === 0) return;

        sc.verses.forEach((v: ScriptureVerse) => {
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
            const cleanBook = sc.book ?? '';
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
    filteredLibraryItems = activeLibraryItems.filter((m) => {
      const matchName = (m.name || '').toLowerCase().includes(query);
      const matchPath = (m.file_path || '').toLowerCase().includes(query);
      const matchType = (m.media_type || '').toLowerCase().includes(query);
      const matchesSearch = matchName || matchPath || matchType;

      const mTypeLower = (m.media_type || '').toLowerCase();
      const effectiveCategory = (activeCategory && activeCategory !== 'all')
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
      const matchSlides = p.slides && p.slides.some((s: LibrarySlide) => (s.content || s.title || '').toLowerCase().includes(query));
      if (mode === 'title') return matchTitle;
      return matchTitle || matchAuthor || matchSlides;
    });
  } else if (lp.getCurrentTab() === 'themes') {
    const categoryMap: Record<string, string> = { 'song-themes': 'song', 'scripture-themes': 'scripture', 'presentation-themes': 'presentation' };
    const wantedCategory = categoryMap[activeCategory];
    filteredLibraryItems = activeLibraryItems.filter((t) => {
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

let currentCatalogItems: LibraryItem[] = [];
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

export function renderCatalog(items: LibraryItem[], tabName: string, matchedOnlineRef: any = null) {
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
      const foundIdx = items.findIndex(it => it.id === lp.getSelectedLibraryItem()?.id);
      if (foundIdx !== -1) {
        targetIdx = foundIdx;
      }
    }
    selectedCatalogIndex = targetIdx;
    const target = items[targetIdx];
    if (target) selectAndPreviewItem(target, tabName);
  } else if (!matchedOnlineRef) {
    if (resourcePreviewMonitor) {
      resourcePreviewMonitor.innerHTML = '<div class="asset-preview-empty">No items match query</div>';
    }
  }
}

export function createCatalogTableRow(item: LibraryItem, idx: number, tabName: string) {
  if (!ctx) return document.createElement('tr');
  const lp = ctx;
  const tr = document.createElement('tr');
  tr.dataset.id = item.id;
  tr.dataset.index = String(idx);
  tr.tabIndex = 0;
  if (idx === selectedCatalogIndex) tr.classList.add('selected');

  if (tabName === 'songs') {
    tr.innerHTML = `
      <td class="cell-title">🎵 ${escapeUserHtml(item.title)}</td>
      <td>${escapeUserHtml(item.author || '')}</td>
      <td class="cell-meta">${escapeUserHtml(item.ccli_number || item.copyright || '')}</td>
    `;
  } else if (tabName === 'scriptures') {
    const snippet = item.remote ? 'Text loads when added to the schedule' : (item.verses && item.verses.length > 0 ? (item.verses[0]?.text ?? '') : '');
    tr.innerHTML = `
      <td class="cell-title">📖 ${escapeUserHtml(item.reference)}</td>
      <td><span class="bible-lang-badge">${lp.escapeHtml(item.version)}</span>${item.remote ? ` <span class="bible-source-tag">${lp.escapeHtml(item.remote.tag)}</span>` : ''}</td>
      <td class="cell-snippet">${lp.escapeHtml(snippet)}</td>
    `;
  } else if (tabName === 'media') {
    tr.innerHTML = `
      <td class="cell-title">🎬 ${escapeUserHtml(item.name)}</td>
      <td>${lp.escapeHtml(item.media_type || 'Image')}</td>
      <td>${item.duration_seconds ? item.duration_seconds + 's' : 'Still'}</td>
    `;
  } else if (tabName === 'presentations') {
    tr.innerHTML = `
      <td class="cell-title">📊 ${escapeUserHtml(item.title)}</td>
      <td>${escapeUserHtml(item.author || 'Media')}</td>
      <td>${item.slides ? item.slides.length : 1} slides</td>
    `;
  } else if (tabName === 'themes') {
    tr.innerHTML = `
      <td class="cell-title">🎨 ${escapeUserHtml(item.name)}</td>
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
    prepareBundleMenuItem(tabName, item);
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

export function appendCatalogTableRows(items: LibraryItem[], tabName: string, startIndex: number) {
  if (!catalogTableBody) return;
  items.forEach((item, i) => {
    const tr = createCatalogTableRow(item, startIndex + i, tabName);
    catalogTableBody.appendChild(tr);
  });
}

export function renderCatalogTable(items: LibraryItem[], tabName: string, _matchedOnlineRef: any = null) {
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

  appendCatalogTableRows(items, tabName, 0);
}

export function highlightCatalogRow(idx: number) {
  if (!catalogTableBody) return;
  selectedCatalogIndex = idx;
  // Exclude the decorative ".catalog-fetch-prompt-row" (online-fetch
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

export function createCatalogGridCard(item: LibraryItem, idx: number, tabName: string) {
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
    <div class="grid-card-title">${escapeUserHtml(title)}</div>
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
    prepareBundleMenuItem(tabName, item);
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

export function appendCatalogGridCards(items: LibraryItem[], tabName: string, startIndex: number) {
  if (!catalogGrid) return;
  items.forEach((item, i) => {
    const card = createCatalogGridCard(item, startIndex + i, tabName);
    catalogGrid.appendChild(card);
  });
}

export function renderCatalogGrid(items: LibraryItem[], tabName: string) {
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
export function selectAndPreviewItem(item: LibraryItem, tabName: string) {
  if (!ctx) return;
  if (item?.remote) {
    // Reference-only result: the text is fetched when it is added or dragged to the schedule, not for a preview.
    ctx.setSelectedLibraryItem(item);
    ctx.showToast(`${item.reference}: add it to the schedule (double-click, or drag it) to load its text.`, 'info');
    return;
  }
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

export function renderAssetPreview(item: LibraryItem, tabName: string) {
  if (!item || !resourcePreviewMonitor) return;
  if (!ctx) return;
  const lp = ctx;

  let title = item.title || item.name || item.reference || '';
  let author = item.author || item.version || '';
  let sampleText = '';
  let bgStyle = 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)';
  let theme: ThemeDefinition | null = null;

  if (item.slides && item.slides[0] && item.slides[0].background) {
    bgStyle = item.slides[0].background;
  } else if (item.theme_name) {
    const t = lp.getAvailableThemes().find(x => x.name === item.theme_name);
    if (t) { bgStyle = t.bg; theme = t; }
  }

  if (tabName === 'scriptures' && item.remote) {
    // A translation a plugin offers: the reference only, until the passage is added to the schedule.
    sampleText = `${item.reference}\n\n${item.remote.tag} \u00b7 text loads when added`;
  } else if (tabName === 'scriptures' && item.verses && item.verses.length > 0) {
    const qLower = (resourceSearchInput ? resourceSearchInput.value : '').trim().toLowerCase();
    let targetVerseIdx = 0;
    if (qLower && item.verses.length > 1) {
      const matchIdx = item.verses.findIndex((v: ScriptureVerse) => (v.text || '').toLowerCase().includes(qLower));
      if (matchIdx !== -1) targetVerseIdx = matchIdx;
    }
    const v1 = item.verses[targetVerseIdx] || item.verses[0];
    if (!v1) return;
    const w1 = (v1.text || '').trim().split(/\s+/).filter(Boolean).length;
    const v2 = item.verses[targetVerseIdx + 1];
    if (w1 < 20 && v2) {
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
    const pVersion = (item.version || (lp.getActiveBibleVersion() !== 'all' ? lp.getActiveBibleVersion() : (lp.getInstalledBibles()[0]?.abbreviation ?? 'Primary'))).trim();
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
        if (otherInstalled[0]) lp.setSecondaryBibleVersion(otherInstalled[0].id);
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
          <span style="font-size: 10px;">Dual: <strong style="color: var(--os-accent-text)">${lp.escapeHtml(pAbbr)}</strong> + <strong style="color: var(--os-accent-text)">${lp.escapeHtml(sAbbr)}</strong></span>
          <button id="btn-add-dual-to-schedule" class="btn btn-primary btn-sm">
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
        addDualScriptureToSchedule(item, pVersion, lp.getSecondaryBibleVersion(), item.verses ?? [], currentDualSecondaryVerses);
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
          const matchingVerses = localData.verses.filter((v: ScriptureVerse) =>
            v.verse_number >= (item.verse_start ?? 0) && v.verse_number <= (item.verse_end ?? Infinity)
          );
          const vList = matchingVerses.length > 0 ? matchingVerses : localData.verses;
          
          const qLower = (resourceSearchInput ? resourceSearchInput.value : '').trim().toLowerCase();
          let targetIdx = 0;
          if (qLower && vList.length > 1) {
            const foundIdx = vList.findIndex((v: ScriptureVerse) => (v.text || '').toLowerCase().includes(qLower));
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
            <button class="btn btn-primary btn-sm" id="btn-preview-video-play">▶ Play</button>
            <button class="btn btn-sm" id="btn-preview-video-stop">⏹ Stop</button>
            <input type="range" id="preview-video-seek" class="live-media-seek" min="0" max="100" value="0">
            <span id="preview-video-time" class="live-media-time">0:00 / 0:00</span>
            <button class="btn btn-ghost btn-sm btn-icon" id="btn-preview-video-mute" title="Mute/Unmute Preview Audio">🔊</button>
          </div>
          <div class="canvas-footer" style="background: rgba(0,0,0,0.85); position: absolute; bottom: 0; left: 0; right: 0; z-index: 4;">
            <span>🎬 ${escapeUserHtml(item.name)}</span>
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
        <div class="asset-preview-audio">
          <div class="asset-preview-audio-icon">🎵</div>
          <div class="asset-preview-card-title">${escapeUserHtml(item.name)}</div>
          <audio controls src="${lp.escapeHtml(item.file_path)}"></audio>
        </div>`;
      return;
    } else {
      resourcePreviewMonitor.innerHTML = `
        <div class="canvas-16-9" style="background: #000; position: relative; display: flex; align-items: center; justify-content: center;">
          <img src="${lp.escapeHtml(item.file_path)}" style="width: 100%; height: 100%; object-fit: contain;">
          <div class="canvas-footer" style="background: rgba(0,0,0,0.75); position: absolute; bottom: 0; left: 0; right: 0;">
            <span>🖼️ ${escapeUserHtml(item.name)}</span>
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
        <span>${escapeUserHtml(title)}</span>
        <span>${escapeUserHtml(author)}</span>
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
    if (found && !found.remote && found.verses && found.verses.length > 0) {
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

  const pVersion = (pVersionParam && pVersionParam !== 'all' ? pVersionParam : (primaryItem.version || (ctx.getInstalledBibles()[0]?.id ?? 'KJV'))).trim();
  const sVersion = (sVersionParam && sVersionParam !== 'all' ? sVersionParam : (ctx.getSecondaryBibleVersion() || (ctx.getInstalledBibles()[1]?.id ?? ctx.getInstalledBibles()[0]?.id ?? 'ASV'))).trim();

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
