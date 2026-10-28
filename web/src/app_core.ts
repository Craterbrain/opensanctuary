// OpenSanctuary Rust Client Show Controller & Bible Manager (app.js)
import type { ShowCommand } from './core/models.ts';
import { errorMessage } from './core/ui_utils.ts';
import type { StateSnapshot, Schedule, ShowState, LibrarySlide, ScriptureVerse, LibraryItem, InstalledBible } from './core/models.ts';
import { showLicensedLink } from './ui/licensed_link.ts';
import { stripLicensedText } from './core/licensed';
import { exportBundle } from './ui/bundle_ui.ts';
import { draggable, dropTargetForElements } from '@atlaskit/pragmatic-drag-and-drop/element/adapter';
import { resolveHostSessionToken, hostTokenHeader, submitManualHostToken, getConsoleSessionId } from './core/host_session.ts';
import { api } from './core/api_client.ts';
import { ensurePatternClip, configurePatternPrerender, setMonitorHeightsProvider, onPatternRenderProgress, parsePatternModes, prerenderAlwaysPatterns, type PatternResolutionSetting } from './core/pattern_render.ts';
import { attachClosestEdge, extractClosestEdge, type Edge } from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge';
import { getReorderDestinationIndex } from '@atlaskit/pragmatic-drag-and-drop-hitbox/util/get-reorder-destination-index';
import {
  resolveTargetItemToDelete,
  effectiveArrangement,
  resolveSlideAt,
  resolveBackgroundAt,
  applyResolvedBackground,
  parseAnimatedPatternMarker,
  isPrimaryDropTarget,
  DEFAULT_THEMES,
  extractImageUrl,
  isImageBackground,
  isVideoBackground,
  isAudioMedia,
  formatCssBackground,
  escapeHtml,
  escapeUserHtml,
  resolveThemeAt,
  type ThemeDefinition
} from './core/presentation_helpers.ts';
import { applyBackgroundThumbnail } from './core/pattern_thumbs.ts';
import { deckModeFor, type DeckName } from './ui/deck_view_mode.ts';
import { showToast, formatMediaTime, copyToClipboard } from './core/ui_utils.ts';
export { showToast, formatMediaTime, copyToClipboard };
import { formatSlideLyricsHtml, applyAutoFitLyrics, renderSlideVisual } from './core/slide_render.ts';
import { parseDisplayOutputs, serializeDisplayOutputs } from './core/display_config.ts';
import { MediaSyncManager, type MediaSyncPayload } from './core/media_sync.ts';
import {
  initSettingsDialog,
  resetOptionsModal
} from './ui/settings_dialog.ts';
import { initThemePicker, openThemePicker } from './ui/theme_picker.ts';
import { initThemeEditor } from './ui/theme_editor.ts';
import { buildSlideCardElement, buildSlideThumbElement, renderDeckEmptyState, slideDeckStyle } from './ui/slide_deck_view.ts';
import { initArrangementModal, openArrangementModal } from './ui/arrangement_modal.ts';
import { initCcliReportModal } from './ui/ccli_report_modal.ts';
import {
  initLibraryPanel,
  loadLibraryTab,
  renderCategoryTree,
  filterAndRenderCatalog,
  areTranslationsEquivalent,
  addItemToSchedule,
  renderAssetPreview,
  addDualScriptureToSchedule,
  updateSearchModeUI,
} from './ui/library_panel.ts';
import {
  initPresentationPlayback,
  isPresentationPlaying,
  stopPresentationPlayback,
  updatePresentationControlsUI,
  getPresentationPlaybackItemId,
} from './ui/presentation_playback.ts';
import { initMediaImagePicker, openMediaImagePicker } from './ui/media_image_picker.ts';
import { initVideoPicker } from './ui/video_picker.ts';
import { dialogManager } from './ui/dialog_manager.ts';
import { contextMenuManager } from './ui/context_menu_manager.ts';
import { DEFAULT_ICON_SET, applyTheme } from './core/icon_sets.ts';
import { createEngineWebSocket, EngineWebSocketClient } from './core/ws_client.ts';

// Functions app_ui.ts hands back after it loads (see main.ts's sequential
// `await import(...)` calls -- app_core.ts loads first, so it cannot
// statically import app_ui.ts without forcing app_ui.ts to evaluate first,
// reversing that order). app_ui.ts calls registerUiCallbacks() once, near
// the end of its own module body, after all of these are declared.
interface UiCallbacks {
  editExistingItem: (item: any, scheduleContext?: { itemIndex: number; slideIndex?: number } | null) => void;
  openImportModal: (initialMode?: string) => void;
  openSlideEditor: (type?: string) => void;
  promptNewSectionHeader: (insertIndex: any) => void;
  setImportMode: (mode: string) => void;
  switchToPairingTab: () => void;
  switchToAdbProvisionTab: () => void;
  showFirstTimeSetup: () => void;
}
const uiCallbacks: Partial<UiCallbacks> = {};
export function registerUiCallbacks(cb: UiCallbacks): void {
  Object.assign(uiCallbacks, cb);
}
// Exposed on window for Playwright E2E tests that drive the running app
// directly (tests/e2e_compare_windows_linux.test.ts, e2e_slide_templates.test.ts,
// tests/theme_picker.test.ts), outside this module graph entirely -- not
// needed by any in-repo module.
try { (globalThis as any).sendCommand = sendCommand; } catch (_) {}
try { (globalThis as any).openThemePicker = openThemePicker; } catch (_) {}


let ws: any = null;
let engineWsClient: EngineWebSocketClient | null = null;
export let currentSnapshot: StateSnapshot | null = null;
let protocolMismatchWarned = false;
let consoleLockedWarned = false;
export let currentTab = 'songs';
export let selectedLibraryItem: LibraryItem | null = null;
export let currentEditorType = 'song';
export let editingItemId: string | null = null;
export let activeImportMode = 'local';
export let activeLiveViewMode = 'matrix'; // 'matrix' or 'list'
export let expandedScheduleIndex: number | null = 0; // Only ONE schedule item expanded at a time, or null if all collapsed

// Active Deck Context & Undo Placeholder State
let lastActiveDeckContext: 'schedule' | 'preview' | 'live' = 'schedule';
export let activeUndoPlaceholder: {
  index: number;
  title: string;
  createdAt: number;
  timer: ReturnType<typeof setTimeout> | null;
} | null = null;

// Context Menu State
export let currentContextMenuTarget: any = null;

// Available Presentation Themes & Background Photos — canonical list lives in
// core/presentation_helpers.ts (already auto-populated from the slide
// editor's own gradient/pattern presets); this is the live, server-refreshed
// copy that extractImageUrl/isImageBackground/formatCssBackground below get
// passed explicitly, since they can't see this module's own mutable state.
export let availableThemes = [...DEFAULT_THEMES];
let lastThemesFetchTime = 0;

// [ARCH:toast-notifications] Fully JS-built UI, no static HTML — delegated to src/core/ui_utils.ts

export async function refreshAvailableThemes(force = false) {
  if (!force && Date.now() - lastThemesFetchTime < 15000) return;
  try {
    const res = await fetch('/api/themes');
    if (res.ok) {
      const dbThemes = await res.json();
      if (Array.isArray(dbThemes) && dbThemes.length > 0) {
        const mapped: ThemeDefinition[] = dbThemes.map(t => ({
          id: t.id,
          name: t.name,
          bg: t.background || 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)',
          font: t.font_family || 'Segoe UI',
          color: t.font_color,
          category: t.category || 'song',
          isDefault: !!t.is_default,
          fontSize: t.font_size,
          textShadow: t.text_shadow,
          alignment: t.alignment,
          lineHeight: t.line_height,
          letterSpacing: t.letter_spacing,
          opacity: t.opacity,
          marginTop: t.margin_top,
          marginBottom: t.margin_bottom,
          marginLeft: t.margin_left,
          marginRight: t.margin_right,
          verticalAlign: t.vertical_align,
          referencePosition: t.reference_position || 'inline',
          chromaKeyEnabled: !!t.chroma_key_enabled,
          chromaKeyColor: t.chroma_key_color,
          safeAreaPercent: t.safe_area_percent || 0,
        }));
        const seen = new Set(mapped.map(m => m.name.toLowerCase()));
        DEFAULT_THEMES.forEach(dt => {
          if (!seen.has(dt.name.toLowerCase())) mapped.push(dt);
        });
        availableThemes = mapped;
        lastThemesFetchTime = Date.now();
      }
    }
  } catch (err) {
    console.warn('Using default theme catalog:', err);
  }
}

// Bible Translations Management (Dynamic DB Lookups - Zero Hardcoded Defaults)
export let installedBibles: InstalledBible[] = [];

export async function refreshInstalledBibles() {
  try {
    const res = await fetch('/api/bibles');
    if (res.ok) {
      const dbBibles = await res.json();
      if (Array.isArray(dbBibles)) {
        installedBibles = dbBibles.filter(b => (b.verse_count === undefined || b.verse_count > 0));
        // The default is always a Bible that is stored here, never a translation a plugin offers (YouVersion's NIV...).
        const stored = installedBibles.filter(b => !b.remote);
        if (!appOptions.defaultBibleVersion && stored.length > 0) {
          const defaultObj = stored.find(b => b.is_default || b.isDefault);
          const pick = defaultObj ? defaultObj.id : stored[0]?.id;
          if (pick) appOptions.defaultBibleVersion = pick;
        }
        installedBibles.forEach(b => {
          b.isDefault = areTranslationsEquivalent(b.id, appOptions.defaultBibleVersion);
        });
        applyDefaultBibleVersionIfUnset();
        if (currentTab === 'scriptures') {
          renderCategoryTree('scriptures');
        }
      }
    }
  } catch (err) {
    console.warn('Could not fetch installed bibles:', err);
  }
}

export let activeBibleVersion = 'all'; // 'all' or specific translation ID
// True once the user has explicitly picked a translation (or "All") this session,
// so the configured default only governs the initial/auto-opened state.
let bibleVersionUserSelected = false;

function applyDefaultBibleVersionIfUnset() {
  if (bibleVersionUserSelected) return;
  if (appOptions.defaultBibleVersion && installedBibles.some(b => areTranslationsEquivalent(b.id, appOptions.defaultBibleVersion))) {
    activeBibleVersion = appOptions.defaultBibleVersion;
  }
}

export let secondaryBibleVersion = '';
export let isDualBibleMode = false;
export let currentDualSecondaryVerses: any[] = [];
let contextMenuTargetBible: any = null;


// Global Application Options (16:9 Default Standard)
// outputMonitor and transitionEffect used to live here but were dead settings — saved
// to the DB and never read by anything (transitionEffect had no rendering path to wire
// to at all; ShowState.transition/SetTransition exists server-side but nothing ever
// sends or renders it). outputMonitor is now the openLiveOutputWindow action button in
// the Settings dialog, which doesn't need a persisted preference. See
// web/src/core/settings_schema.ts.
export let appOptions = {
  aspectRatio: '16:9',
  alertPosition: 'bottom',
  alertFontSize: '36',
  churchName: 'Grace Sanctuary Media Ministry',
  defaultBibleVersion: '',
  displayOutputs: '[]',
  iconSet: DEFAULT_ICON_SET,
  colorTheme: DEFAULT_ICON_SET,
  themeLinked: 'true',
  // Pre-rendered animated backgrounds (core/pattern_render.ts): 'on' | 'off', and 'auto' | '1440' | '1080'.
  patternPrerender: 'on',
  patternResolution: 'auto',
  // JSON {patternKey: 'auto' | 'always' | 'never'}: per-pattern pre-render choice.
  patternModes: '{}'
};

// Auto resolution looks at the connected monitors (any below 1440p -> render 1080p).
setMonitorHeightsProvider(async () => {
  const res = await fetch('/api/displays');
  const data = res.ok ? await res.json() : null;
  return ((data && data.monitors) || []).map((m: { height: number }) => m.height);
});
let patternRenderToastShown = false;
let patternAlwaysQueued = false;
onPatternRenderProgress(p => {
  if (!p) { patternRenderToastShown = false; return; }
  if (!patternRenderToastShown) {
    patternRenderToastShown = true;
    showToast('Preparing smooth playback for an animated background. This runs in the background.', 'info', 5000);
  }
});

export async function loadAppOptions() {
  try {
    // The server withholds credential settings (API keys) from callers with
    // no host token, so make sure the token is resolved before reading them --
    // otherwise saving the settings dialog would write those keys back blank.
    await resolveHostSessionToken();
    const res = await fetch('/api/settings');
    if (res.ok) {
      const dbSettings = await res.json();
      if (dbSettings && typeof dbSettings === 'object') {
        Object.assign(appOptions, dbSettings);
        applyAppOptionsToUI();
        if (installedBibles && installedBibles.length > 0) {
          installedBibles.forEach(b => {
            b.isDefault = areTranslationsEquivalent(b.id, appOptions.defaultBibleVersion);
          });
          applyDefaultBibleVersionIfUnset();
          if (currentTab === 'scriptures') {
            renderCategoryTree('scriptures');
          }
        }
      }
    }
  } catch (err) {
    console.warn('Using default app options:', err);
  }
}

export async function saveAppOptions(newOptions: any) {
  Object.assign(appOptions, newOptions);
  applyAppOptionsToUI();
  try {
    await fetch('/api/settings', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(appOptions)
    });
  } catch (err) {
    console.warn('Failed to save settings to DB:', err);
  }
}

function applyAppOptionsToUI() {
  applyTheme(appOptions);
  configurePatternPrerender({
    enabled: appOptions.patternPrerender !== 'off',
    resolution: (['auto', '1440', '1080'].includes(appOptions.patternResolution) ? appOptions.patternResolution : 'auto') as PatternResolutionSetting,
    modes: parsePatternModes(appOptions.patternModes),
  });
  // Patterns set to "always" get rendered ahead of time, once things have settled.
  if (!patternAlwaysQueued) {
    patternAlwaysQueued = true;
    setTimeout(prerenderAlwaysPatterns, 8000);
  }
  const outAsp = document.getElementById('output-aspect-ratio-text');
  if (outAsp) outAsp.textContent = `${appOptions.aspectRatio || '16:9'} Hardware Mirror`;
  if (canvasFooterLeftEl && (!currentSnapshot || !currentSnapshot.state || !currentSnapshot.state.live_item)) {
    canvasFooterLeftEl.textContent = appOptions.churchName;
  }
  const ratio = (appOptions.aspectRatio === '4:3') ? '4 / 3' : '16 / 9';
  document.documentElement.style.setProperty('--app-aspect-ratio', ratio);
}

// Safe event listener helper
export function on(id: string | Element | null, event: string, handler: EventListenerOrEventListenerObject) {
  const el = typeof id === 'string' ? document.getElementById(id) : id;
  if (el) el.addEventListener(event, handler);
}

// DOM Elements
export const scheduleListEl = document.getElementById('schedule-items-list');
const scheduleTitleEl = document.getElementById('schedule-title-display');
const previewTitleEl = document.getElementById('preview-title-display');
export const previewSlideMatrixEl = document.getElementById('preview-slide-matrix');
const previewCanvasEl = document.getElementById('preview-canvas');
const previewCanvasLyricsEl = document.getElementById('preview-canvas-lyrics-text');
const previewCanvasFooterLeftEl = document.getElementById('preview-canvas-footer-left');
const previewCanvasFooterRightEl = document.getElementById('preview-canvas-footer-right');
const previewCanvasImageEl = document.getElementById('preview-canvas-image');
const previewCanvasElementsEl = document.getElementById('preview-canvas-elements');
const previewSlideCounterTextEl = document.getElementById('preview-slide-counter-text');
const liveTitleEl = document.getElementById('live-title-display');
export const liveSlideMatrixEl = document.getElementById('live-slide-matrix');
const canvasLyricsEl = document.getElementById('canvas-lyrics-text');
const liveCanvasElementsEl = document.getElementById('live-canvas-elements');
const canvasFooterLeftEl = document.getElementById('canvas-footer-left');
const canvasFooterRightEl = document.getElementById('canvas-footer-right');
const canvasBlackoutEl = document.getElementById('canvas-blackout');
const canvasLogoEl = document.getElementById('canvas-logo');
const canvasNurseryAlertEl = document.getElementById('canvas-nursery-alert');
const canvasNurseryTextEl = document.getElementById('canvas-nursery-text');
const slideCounterTextEl = document.getElementById('slide-counter-text');
const liveCanvasEl = document.getElementById('live-canvas');

// Search & Preview Actions
export const btnToggleDualBible = document.getElementById('btn-toggle-dual-bible');

// Ribbon Buttons
export const btnNew = document.getElementById('btn-new');
export const btnOpen = document.getElementById('btn-open');
export const btnSave = document.getElementById('btn-save');
export const btnImport = document.getElementById('btn-import');
export const btnStore = document.getElementById('btn-store');
export const btnWeb = document.getElementById('btn-web');
export const btnRemote = document.getElementById('btn-remote');
export const btnGoLive = document.getElementById('btn-golive');
export const btnAlert = document.getElementById('btn-alert');
export const btnLogo = document.getElementById('btn-logo');
export const btnBlack = document.getElementById('btn-black');
export const btnClear = document.getElementById('btn-clear');
export const btnLiveStatus = document.getElementById('btn-live-status');

// Modals
export const alertModal = document.getElementById('alert-modal');
export const alertTextInput = document.getElementById('alert-text-input') as HTMLInputElement | null;
export const openModal = document.getElementById('open-modal');
export const saveModal = document.getElementById('save-modal');
export const storeModal = document.getElementById('store-modal');
export const webModal = document.getElementById('web-modal');
export const remoteModal = document.getElementById('remote-modal');
export const importModal = document.getElementById('import-modal');
export const createModal = document.getElementById('create-modal');
export const optionsModal = document.getElementById('options-modal');
export const shortcutsModal = document.getElementById('shortcuts-modal');
export const aboutModal = document.getElementById('about-modal');
export const scheduleArticlesModal = document.getElementById('schedule-articles-modal');

// Editor Elements
export const createItemTitle = document.getElementById('create-item-title') as HTMLInputElement | null;
export const createItemAuthor = document.getElementById('create-item-author') as HTMLInputElement | null;
export const createItemCopyright = document.getElementById('create-item-copyright') as HTMLInputElement | null;
export const createItemPublicDomain = document.getElementById('create-item-public-domain') as HTMLInputElement | null;
export const createItemContent = document.getElementById('create-item-content') as HTMLInputElement | null;

// ============================================================================
// GLOBAL RIGHT-CLICK SUPPRESSION & CONTEXT MENU MANAGEMENT
// ============================================================================
// Suppress default browser/system context menu everywhere
document.addEventListener('contextmenu', (e) => {
  e.preventDefault();
});

export function showContextMenu(menuEl: any, x: number, y: number) {
  return contextMenuManager.showElement(menuEl, x, y);
}

export function hideAllContextMenus() {
  contextMenuManager.hideAll();
}

/**
 * Shows/hides the "Merge with Previous/Next into Presentation" items on the
 * schedule item context menu. Merging is only offered between two adjacent,
 * non-header items (section headers/groups have no slides to merge).
 */
function updateScheduleMergeMenuVisibility(idx: number, item: any, schedule: any) {
  const prevBtn = document.getElementById('ctx-sched-merge-prev');
  const nextBtn = document.getElementById('ctx-sched-merge-next');
  const items = (schedule && schedule.items) || [];
  const isMergeable = (it: any) => it && !it.is_section_header && (it.item_type || '').toLowerCase() !== 'header' && (it.item_type || '').toLowerCase() !== 'group';

  const canMergePrev = isMergeable(item) && idx > 0 && isMergeable(items[idx - 1]);
  const canMergeNext = isMergeable(item) && idx < items.length - 1 && isMergeable(items[idx + 1]);

  if (prevBtn) prevBtn.style.display = canMergePrev ? '' : 'none';
  if (nextBtn) nextBtn.style.display = canMergeNext ? '' : 'none';
}

/** Shows "Set Slide Duration..." on the slide context menu only for presentation items. */
function updateSlideDurationMenuVisibility(item: any) {
  const btn = document.getElementById('ctx-slide-duration');
  if (btn) btn.style.display = (item && item.item_type === 'presentation') ? '' : 'none';
}

// ============================================================================
// MODAL MANAGEMENT & DIALOG RESET SYSTEM
// ============================================================================
// ============================================================================
// PRESENTATION SLIDESHOW PLAYBACK (delegated to src/ui/presentation_playback.ts)
// ============================================================================
initPresentationPlayback({
  getCurrentSnapshot: () => currentSnapshot,
  sendCommand: sendCommand,
});

export function showModal(modalEl: any, options?: any) {
  if (!modalEl) return null;
  hideAllContextMenus();
  return dialogManager.openModal(modalEl, options);
}

export function closeModal(modalEl: any) {
  if (!modalEl) return false;
  return dialogManager.closeModal(modalEl);
}

function showConfirmDialog(title: string, message: string, detail: string, onAccept: () => Promise<void> | void, acceptBtnText = '🗑️ Delete Permanently') {
  return dialogManager.showConfirmDialog(title, message, detail, onAccept, acceptBtnText);
}

export function closeTopmostModal() {
  hideAllContextMenus();
  return dialogManager.closeTopmostModal();
}

function resetCreateModal() {
  editingItemId = null;
  if (createItemTitle) createItemTitle.value = '';
  if (createItemAuthor) createItemAuthor.value = '';
  if (createItemContent) createItemContent.value = '';
  if (createItemCopyright) createItemCopyright.value = '';
  if (createItemPublicDomain) createItemPublicDomain.checked = false;
  currentEditorType = 'song';
  document.querySelectorAll<HTMLElement>('.type-pill').forEach(pill => {
    pill.classList.toggle('active', pill.dataset.type === 'song');
  });
  // The canvas slide editor's own state (studioSlides, the mounted SlideEditor
  // instance) lives in app_ui.ts and is fully reinitialized by openSlideEditor/
  // editExistingItem on the next open, so nothing further to reset here.
}

export function resetImportModal() {
  const fileEl = document.getElementById('file-freeshow-fsb') as HTMLInputElement | null;
  if (fileEl) fileEl.value = '';
  uiCallbacks.setImportMode?.('local');
}

export function handleImportBack() {
  closeModal(importModal);
}

export function handleOptionsBack() {
  // The settings sidebar is a flat set of peer categories, not a drill-down wizard
  // like the old tab carousel — there's no "previous step" to return to, so Back and
  // Cancel both just close.
  closeModal(optionsModal);
}

export function resetAlertModal() {
  if (alertTextInput) {
    if (currentSnapshot && currentSnapshot.state && currentSnapshot.state.alert_message) {
      alertTextInput.value = currentSnapshot.state.alert_message;
    } else {
      alertTextInput.value = '';
    }
  }
}

// ============================================================================
// SETTINGS DIALOG (delegated to src/ui/settings_dialog.ts)
// ============================================================================
initSettingsDialog({
  getAppOptions: () => appOptions,
  getInstalledBibles: () => installedBibles,
  getAvailableThemes: () => typeof availableThemes !== 'undefined' ? availableThemes : [],
  areTranslationsEquivalent: (a, b) => areTranslationsEquivalent(a, b),
  getDisplayOutputs: () => parseDisplayOutputs(appOptions.displayOutputs),
  saveDisplayOutputs: (outputs) => saveAppOptions({ displayOutputs: serializeDisplayOutputs(outputs) }),
  setThemeOptions: (changes) => saveAppOptions(changes),
  switchToPairingTab: () => uiCallbacks.switchToPairingTab?.(),
  switchToAdbProvisionTab: () => uiCallbacks.switchToAdbProvisionTab?.(),
  showFirstTimeSetup: () => uiCallbacks.showFirstTimeSetup?.(),
});


export function resetSaveModal() {
  const titleEl = document.getElementById('save-schedule-title') as HTMLInputElement | null;
  if (titleEl) {
    titleEl.value = scheduleTitleEl ? (scheduleTitleEl.textContent || 'Sunday Morning Worship') : 'Sunday Morning Worship';
  }
  syncSaveFormatLabel();
}

/** The Save button names the file type the chosen format will download. */
export function syncSaveFormatLabel() {
  const select = document.getElementById('save-format-select') as HTMLSelectElement | null;
  const button = document.getElementById('btn-confirm-save');
  if (!select || !button) return;
  const ext = select.value === 'json' ? 'osj' : select.value;
  button.textContent = `💾 Download & Save (.${ext})`;
}

export function resetOpenModal() {
  const fInput = document.getElementById('file-schedule-input') as HTMLInputElement | null;
  if (fInput) fInput.value = '';
}

export function resetWebModal() {
  const urlEl = document.getElementById('web-stream-url') as HTMLInputElement | null;
  if (urlEl) urlEl.value = '';
}

// Register DialogManager reset hooks for all modal dialogs
dialogManager.setBeforeEscapeClose(hideAllContextMenus);
dialogManager.registerResetHook('create-modal', resetCreateModal);
dialogManager.registerResetHook('import-modal', resetImportModal);
dialogManager.registerResetHook('alert-modal', resetAlertModal);
dialogManager.registerResetHook('options-modal', resetOptionsModal);
dialogManager.registerResetHook('save-modal', resetSaveModal);
dialogManager.registerResetHook('open-modal', resetOpenModal);
dialogManager.registerResetHook('web-modal', resetWebModal);
dialogManager.registerResetHook('arrangement-modal', () => {
  document.body.classList.remove('editor-open');
});
dialogManager.setupConfirmModalBindings();

// Backdrop click closes and resets modal
document.querySelectorAll('.modal-backdrop').forEach(backdrop => {
  backdrop.addEventListener('click', (e) => {
    if (e.target === backdrop) {
      closeModal(backdrop);
    }
  });
});

// ============================================================================
// THEME & BACKGROUND PICKER MODAL (delegated to src/ui/theme_picker.ts)
// ============================================================================
initThemePicker({
  getAvailableThemes: () => typeof availableThemes !== 'undefined' ? availableThemes : [],
  refreshThemes: refreshAvailableThemes,
  showModal: showModal,
  closeModal: closeModal,
  escapeHtml: escapeHtml
});

initThemeEditor({
  refreshThemes: () => refreshAvailableThemes(true),
  showToast: showToast,
  escapeHtml: escapeHtml
});

// ============================================================================
// SONG ARRANGEMENT EDITOR MODAL (delegated to src/ui/arrangement_modal.ts)
// ============================================================================
initArrangementModal({
  showToast: showToast,
  showModal: showModal,
  closeModal: closeModal,
  sendCommand: sendCommand,
  getSlideBadge: getSlideBadge,
  getSlideBadgeColor: getSlideBadgeColor,
  escapeHtml: escapeHtml,
  formatCssBackground: formatCssBackground,
  getCurrentSnapshot: () => currentSnapshot,
  renderSchedule: renderSchedule,
  renderPreviewDeck: renderPreviewDeck,
  renderLiveDeck: renderLiveDeck,
});

// ============================================================================
// CCLI USAGE REPORT MODAL (delegated to src/ui/ccli_report_modal.ts)
// ============================================================================
initCcliReportModal({
  showToast: showToast,
  showModal: showModal,
  closeModal: closeModal,
});

// CCLI reporting due-date reminder (docs/CCLI_REPORTING.md) -- checked once
// per console load rather than only when Settings happens to be opened, so
// an operator who never opens Settings still sees it. A persistent
// (duration: 0) toast, the closest existing primitive to a "banner" here,
// since this shouldn't silently disappear off-screen before anyone notices.
(async () => {
  try {
    const settings = await api.settings.get();
    const dueDate = (settings?.ccliReportingDueDate || '').trim();
    if (!dueDate) return;
    const due = new Date(`${dueDate}T00:00:00`);
    if (isNaN(due.getTime())) return;
    const daysLeft = Math.ceil((due.getTime() - Date.now()) / 86400000);
    if (daysLeft > 30) return;
    const message = daysLeft < 0
      ? `CCLI usage report is overdue (was due ${dueDate}). Settings → Integrations → CCLI Usage Report to export it.`
      : `CCLI usage report is due ${dueDate} (${daysLeft} day${daysLeft === 1 ? '' : 's'} left). Settings → Integrations → CCLI Usage Report to export it.`;
    showToast(message, 'warning', 0);
  } catch (_) {
    // Not fatal -- the Settings panel itself still shows the due date.
  }
})();

// ============================================================================
// MEDIA IMAGE PICKER (delegated to src/ui/media_image_picker.ts)
// ============================================================================
initMediaImagePicker({
  showModal: showModal,
  closeModal: closeModal,
  escapeHtml: escapeHtml,
});

initVideoPicker({
  showModal: showModal,
  closeModal: closeModal,
  escapeHtml: escapeHtml,
  showToast: showToast,
});

// ============================================================================
// BIBLE REFERENCE PARSER (POWERED BY RUST ENGINE API)
// ============================================================================
// (Bible reference parser implementation delegated to src/core/bible_parser.ts)


// ============================================================================
// WEBSOCKET & ENGINE COMMUNICATIONS
// ============================================================================
export function initWebSocket() {
  if (engineWsClient) {
    return;
  }

  engineWsClient = createEngineWebSocket({
    validateProtocol: true,
    onOpen: () => {
      ws = engineWsClient?.getRawSocket() || null;
      try { globalThis.ws = ws; } catch (_) {}
    },
    onClose: () => {
      ws = engineWsClient?.getRawSocket() || null;
      try { globalThis.ws = ws; } catch (_) {}
    },
    onProtocolMismatch: (errorMessage) => {
      console.error(`[Protocol] ${errorMessage}`);
      // Warn once, not on every message — a version mismatch persists across the
      // whole session (a server update without a matching client rebuild), so
      // repeating the toast on every broadcast would just spam the operator.
      if (!protocolMismatchWarned) {
        protocolMismatchWarned = true;
        showToast('⚠️ Server/client version mismatch detected — reload this page.', 'warning', 8000);
      }
    },
    onSnapshot: (snapshot) => {
      currentSnapshot = snapshot;
      renderAll(snapshot);
    },
    onCommandRejected: (reason, message) => {
      // "One console at a time, first one connected wins" (docs/CLIENT_PAIRING.md).
      // Warn once, not on every subsequent command this console can't send
      // while locked out -- that would spam the operator with the same fact
      // repeatedly instead of just once.
      if (reason === 'console_locked' && !consoleLockedWarned) {
        consoleLockedWarned = true;
        showToast(`🔒 ${message}`, 'warning', 10000);
      }
    },
  });

  ws = engineWsClient.getRawSocket();
  try { globalThis.ws = ws; } catch (_) {}

  // Remote console access (docs/CLIENT_PAIRING.md): if this page can't
  // resolve a host token at all (not the native webview, not loopback, no
  // valid link/cached token), show a way to paste one in by hand rather
  // than silently failing every command forever with no explanation.
  resolveHostSessionToken().then((token) => {
    if (!token) showHostAuthBanner();
  });
}

/**
 * A small, self-contained "this console isn't authenticated yet" banner,
 * built the same lazy-DOM-creation way `showToast` builds its container --
 * no index.html changes needed. Lets an operator paste in a host token they
 * were handed out-of-band (the server's printed terminal link/QR, or the
 * bare token read aloud/copied) -- see `submitManualHostToken` in
 * host_session.ts. The console still renders and shows live state either
 * way (reading is already zero-auth); this only unblocks issuing commands.
 */
function showHostAuthBanner() {
  if (typeof document === 'undefined' || !document.body) return;
  if (document.getElementById('os-host-auth-banner')) return; // already shown

  const banner = document.createElement('div');
  banner.id = 'os-host-auth-banner';
  banner.className = 'os-banner';
  banner.innerHTML = `
    <span>🔒 This console isn't authenticated — paste the host token shown on the server to control the show.</span>
    <input type="password" id="os-host-auth-input" placeholder="Host token">
    <button id="os-host-auth-connect" class="btn btn-primary">Connect</button>
    <span id="os-host-auth-error" class="os-banner-error"></span>
    <button id="os-host-auth-dismiss" class="modal-close-btn os-banner-dismiss" title="Dismiss">{icon:close}</button>
  `;
  document.body.appendChild(banner);

  const input = document.getElementById('os-host-auth-input') as HTMLInputElement | null;
  const errorEl = document.getElementById('os-host-auth-error');
  const connect = async () => {
    if (!input) return;
    if (errorEl) errorEl.textContent = '';
    const ok = await submitManualHostToken(input.value);
    if (ok) {
      banner.remove();
      showToast('✓ Console authenticated', 'success');
    } else if (errorEl) {
      errorEl.textContent = 'Invalid token';
    }
  };
  document.getElementById('os-host-auth-connect')?.addEventListener('click', connect);
  input?.addEventListener('keydown', (e: KeyboardEvent) => { if (e.key === 'Enter') connect(); });
  document.getElementById('os-host-auth-dismiss')?.addEventListener('click', () => banner.remove());
}

export function sendCommand(cmd: ShowCommand) {
  if (engineWsClient) {
    engineWsClient.sendCommand(cmd);
  } else {
    // /api/command requires the host token for any caller with no paired-
    // device token (see docs/CLIENT_PAIRING.md) -- same as the WS path
    // engineWsClient.sendCommand takes below.
    resolveHostSessionToken().then(() => {
      const hostHeaders = hostTokenHeader();
      fetch('/api/command', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...hostHeaders,
          ...(hostHeaders['x-host-token'] ? { 'x-console-session-id': getConsoleSessionId() } : {}),
        },
        body: JSON.stringify(cmd)
      }).catch(() => {});
    });
  }
}

// ============================================================================
// UI RENDERING
// ============================================================================
export function renderAll(snapshot = currentSnapshot) {
  if (snapshot) currentSnapshot = snapshot;
  if (!snapshot) snapshot = currentSnapshot;
  if (!snapshot) return;
  if (snapshot.state) renderRibbon(snapshot.state);
  if (snapshot.schedule) renderSchedule(snapshot.schedule);
  if (snapshot.state) {
    renderPreviewDeck(snapshot.state);
    renderPreviewOutput(snapshot.state);
    renderLiveDeck(snapshot.state);
    renderLiveOutput(snapshot.state);
  }
}

function renderRibbon(state: ShowState) {
  if (btnBlack) btnBlack.classList.toggle('active', !!state.is_blackout);
  if (btnClear) btnClear.classList.toggle('active', !!state.is_clear_text);
  if (btnLogo) btnLogo.classList.toggle('active', !!state.is_logo_override);
  if (btnAlert) btnAlert.classList.toggle('active', !!state.alert_message);
}

// ============================================================================
// SCHEDULE PANEL: SINGLE ITEM EXPANSION & GROUP COLLAPSING (PRAGMATIC DND)
// ============================================================================
export let lastRenderedScheduleKey = '';
export const collapsedGroupIds = new Set<string>();

// Cleanup subscriptions for dynamic DOM elements
let scheduleDndCleanups: Array<() => void> = [];
let previewDndCleanups: Array<() => void> = [];

/**
 * Keeps expandedScheduleIndex synchronized when items in the schedule move.
 */
function adjustExpandedIndexOnReorder(fromIdx: number, toIdx: number) {
  if (expandedScheduleIndex === null || fromIdx === toIdx) return;
  if (expandedScheduleIndex === fromIdx) {
    expandedScheduleIndex = toIdx;
  } else if (fromIdx < toIdx && expandedScheduleIndex > fromIdx && expandedScheduleIndex <= toIdx) {
    expandedScheduleIndex -= 1;
  } else if (fromIdx > toIdx && expandedScheduleIndex >= toIdx && expandedScheduleIndex < fromIdx) {
    expandedScheduleIndex += 1;
  }
}

/**
 * Computes destination index using Pragmatic DnD's getReorderDestinationIndex
 * with edge normalization for multi-directional / grid matrix layouts.
 */
function computeSafeDestinationIndex(startIndex: number, indexOfTarget: number, closestEdge: Edge | null, axis: 'vertical' | 'horizontal' = 'vertical'): number {
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

/**
 * Records screen positions of schedule items before mutation for FLIP animation.
 */
function recordSchedulePositions(): Map<string, DOMRect> {
  const positions = new Map<string, DOMRect>();
  if (!scheduleListEl) return positions;
  const els = scheduleListEl.querySelectorAll<HTMLElement>('[data-schedule-key]');
  els.forEach(el => {
    const key = el.getAttribute('data-schedule-key');
    if (key) {
      positions.set(key, el.getBoundingClientRect());
    }
  });
  return positions;
}

/**
 * Applies FLIP (First, Last, Invert, Play) smooth CSS transform animations to reordered schedule elements.
 */
function applyScheduleFLIP(prevPositions: Map<string, DOMRect>) {
  if (!scheduleListEl || !prevPositions || prevPositions.size === 0) return;
  requestAnimationFrame(() => {
    const els = scheduleListEl.querySelectorAll<HTMLElement>('[data-schedule-key]');
    els.forEach((el) => {
      const key = el.getAttribute('data-schedule-key');
      if (key && prevPositions.has(key)) {
        const oldRect = prevPositions.get(key)!;
        const newRect = el.getBoundingClientRect();
        const deltaY = oldRect.top - newRect.top;
        if (Math.abs(deltaY) > 0.5) {
          el.style.transform = `translateY(${deltaY}px)`;
          el.style.transition = 'none';
          void el.offsetHeight; // Force reflow
          el.style.transition = 'transform 0.22s cubic-bezier(0.2, 0.0, 0.1, 1)';
          el.style.transform = '';
          const onEnd = () => {
            el.style.transition = '';
            el.style.transform = '';
            el.removeEventListener('transitionend', onEnd);
          };
          el.addEventListener('transitionend', onEnd, { once: true });
        }
      }
    });
  });
}

/**
 * Clears all drop indicators and active states from schedule DOM nodes.
 */
function clearAllScheduleDropIndicators() {
  if (!scheduleListEl) return;
  scheduleListEl.querySelectorAll('.drop-above, .drop-below, .dragover-active, .dragging').forEach(el => {
    el.classList.remove('drop-above', 'drop-below', 'dragover-active', 'dragging');
  });
}

/**
 * Reorders schedule items: optimistic array mutation immediately, backend command sent,
 * but full renderSchedule() deferred to next frame/microtask so nodes are not destroyed mid-gesture.
 */
function reorderScheduleItemsAnimated(schedule: Schedule, fromIdx: number, toIdx: number, focusTargetSelector?: string) {
  const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : schedule;
  if (!sched || !sched.items || fromIdx === toIdx || fromIdx < 0 || toIdx < 0
      || fromIdx >= sched.items.length || toIdx >= sched.items.length) {
    return;
  }

  // 1. Optimistic in-memory mutation
  const prevPositions = recordSchedulePositions();
  const [moved] = sched.items.splice(fromIdx, 1);
  if (moved) sched.items.splice(toIdx, 0, moved);
  if (schedule && schedule !== sched && schedule.items) {
    const [m2] = schedule.items.splice(fromIdx, 1);
    if (m2) schedule.items.splice(toIdx, 0, m2);
  }
  adjustExpandedIndexOnReorder(fromIdx, toIdx);

  // Keep selected_item_index (lives on state, not schedule — see
  // ShowState::selected_item_index in src/core/models.rs) consistent with the move
  const stateForSelection = currentSnapshot && currentSnapshot.state;
  if (stateForSelection && stateForSelection.selected_item_index !== undefined && stateForSelection.selected_item_index !== null) {
    const sel = stateForSelection.selected_item_index;
    if (sel === fromIdx) {
      stateForSelection.selected_item_index = toIdx;
    } else if (fromIdx < sel && toIdx >= sel) {
      stateForSelection.selected_item_index = sel - 1;
    } else if (fromIdx > sel && toIdx <= sel) {
      stateForSelection.selected_item_index = sel + 1;
    }
  }

  clearAllScheduleDropIndicators();
  sendCommand({ ReorderSchedule: { from: fromIdx, to: toIdx } });

  // 2. Defer full DOM re-render & FLIP animation to the next animation frame
  // to avoid destroying nodes synchronously inside the drop gesture callback!
  requestAnimationFrame(() => {
    lastRenderedScheduleKey = '';
    renderSchedule(sched);
    applyScheduleFLIP(prevPositions);

    if (focusTargetSelector) {
      setTimeout(() => {
        const el = scheduleListEl?.querySelector(focusTargetSelector) as HTMLElement;
        if (el) el.focus();
      }, 50);
    }
  });
}

/**
 * Reorders child slides inside a schedule item: optimistic mutation immediately,
 * renderSchedule deferred to next frame so nodes are not destroyed mid-gesture.
 */
function reorderChildSlidesAnimated(schedule: Schedule, itemIdx: number, fromIdx: number, toIdx: number, focusTargetSelector?: string) {
  const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : schedule;
  const it = sched && sched.items ? sched.items[itemIdx] : null;
  const arr = effectiveArrangement(it);
  if (!it || fromIdx === toIdx || fromIdx < 0 || toIdx < 0
      || fromIdx >= arr.length || toIdx >= arr.length) {
    return;
  }

  const prevPositions = recordSchedulePositions();
  if (!it.arrangement || !Array.isArray(it.arrangement) || it.arrangement.length === 0) {
    it.arrangement = effectiveArrangement(it);
  }
  const [movedEntry] = it.arrangement.splice(fromIdx, 1);
  if (movedEntry) it.arrangement.splice(toIdx, 0, movedEntry);

  if (schedule && schedule !== sched && schedule.items && schedule.items[itemIdx]) {
    const sItem = schedule.items[itemIdx];
    if (!sItem.arrangement || !Array.isArray(sItem.arrangement) || sItem.arrangement.length === 0) {
      sItem.arrangement = effectiveArrangement(sItem);
    }
    const [m2] = sItem.arrangement.splice(fromIdx, 1);
    if (m2) sItem.arrangement.splice(toIdx, 0, m2);
  }

  // Keep live item in sync if this is the currently live item
  if (currentSnapshot && currentSnapshot.state && currentSnapshot.state.live_item
      && currentSnapshot.state.live_item.id === it.id) {
    const lItem = currentSnapshot.state.live_item;
    if (!lItem.arrangement || !Array.isArray(lItem.arrangement) || lItem.arrangement.length === 0) {
      lItem.arrangement = effectiveArrangement(lItem);
    }
    const [mLive] = lItem.arrangement.splice(fromIdx, 1);
    if (mLive) lItem.arrangement.splice(toIdx, 0, mLive);
    if (currentSnapshot.state.live_slide_index === fromIdx) {
      currentSnapshot.state.live_slide_index = toIdx;
    } else if (fromIdx < toIdx && currentSnapshot.state.live_slide_index > fromIdx && currentSnapshot.state.live_slide_index <= toIdx) {
      currentSnapshot.state.live_slide_index -= 1;
    } else if (fromIdx > toIdx && currentSnapshot.state.live_slide_index >= toIdx && currentSnapshot.state.live_slide_index < fromIdx) {
      currentSnapshot.state.live_slide_index += 1;
    }
  }

  // Keep staged item in sync if this is the currently staged item
  if (currentSnapshot && currentSnapshot.state && currentSnapshot.state.staged_item
      && currentSnapshot.state.staged_item.id === it.id) {
    const sItem = currentSnapshot.state.staged_item;
    if (!sItem.arrangement || !Array.isArray(sItem.arrangement) || sItem.arrangement.length === 0) {
      sItem.arrangement = effectiveArrangement(sItem);
    }
    const [mStaged] = sItem.arrangement.splice(fromIdx, 1);
    if (mStaged) sItem.arrangement.splice(toIdx, 0, mStaged);
    if (currentSnapshot.state.staged_slide_index === fromIdx) {
      currentSnapshot.state.staged_slide_index = toIdx;
    } else if (fromIdx < toIdx && currentSnapshot.state.staged_slide_index > fromIdx && currentSnapshot.state.staged_slide_index <= toIdx) {
      currentSnapshot.state.staged_slide_index -= 1;
    } else if (fromIdx > toIdx && currentSnapshot.state.staged_slide_index >= toIdx && currentSnapshot.state.staged_slide_index < fromIdx) {
      currentSnapshot.state.staged_slide_index += 1;
    }
    lastRenderedPreviewKey = '';
    renderPreviewDeck(currentSnapshot.state);
  }

  clearAllScheduleDropIndicators();
  sendCommand({ ReorderItemSlides: { item_index: itemIdx, from: fromIdx, to: toIdx } });

  requestAnimationFrame(() => {
    lastRenderedScheduleKey = '';
    renderSchedule(sched);
    applyScheduleFLIP(prevPositions);

    if (focusTargetSelector) {
      setTimeout(() => {
        const el = scheduleListEl?.querySelector(focusTargetSelector) as HTMLElement;
        if (el) el.focus();
      }, 50);
    }
  });
}

// Active Panel / Deck Interaction Tracker
document.addEventListener('click', (e) => {
  if ((e.target as Element | null)?.closest('#preview-panel') || (e.target as Element | null)?.closest('#preview-slide-matrix')) {
    lastActiveDeckContext = 'preview';
  } else if ((e.target as Element | null)?.closest('#live-output-panel') || (e.target as Element | null)?.closest('#live-slide-matrix') || (e.target as Element | null)?.closest('#live-panel')) {
    lastActiveDeckContext = 'live';
  } else if ((e.target as Element | null)?.closest('#schedule-panel') || (e.target as Element | null)?.closest('#schedule-items-list')) {
    lastActiveDeckContext = 'schedule';
  }
}, true);

/**
 * Creates an interactive DOM placeholder row in the schedule where an item was just deleted.
 * Hangs around for a second (1500ms), pauses on hover, and undos on click.
 */
function createUndoPlaceholderElement(ph: { index: number; title: string; createdAt: number; timer: any }): HTMLElement {
  const el = document.createElement('div');
  el.className = 'schedule-undo-placeholder';
  el.setAttribute('role', 'alert');
  el.innerHTML = `
    <div class="undo-content">
      <span style="font-size: 12px;">🗑️</span>
      <span>Removed "<strong>${escapeUserHtml(ph.title)}</strong>"</span>
    </div>
    <button class="btn btn-undo" type="button" title="Undo delete (Ctrl+Z)">↩ Undo</button>
    <div class="undo-progress"></div>
  `;

  const btnUndo = el.querySelector('.btn-undo');
  if (btnUndo) {
    btnUndo.addEventListener('click', (e) => {
      e.stopPropagation();
      triggerUndoFromPlaceholder();
    });
  }

  el.addEventListener('click', () => {
    triggerUndoFromPlaceholder();
  });

  // Pause timer on hover so user can comfortably click
  el.addEventListener('mouseenter', () => {
    if (activeUndoPlaceholder && activeUndoPlaceholder.timer) {
      clearTimeout(activeUndoPlaceholder.timer);
      activeUndoPlaceholder.timer = null;
    }
    const prog = el.querySelector('.undo-progress') as HTMLElement;
    if (prog) prog.style.animationPlayState = 'paused';
  });

  el.addEventListener('mouseleave', () => {
    if (activeUndoPlaceholder && !activeUndoPlaceholder.timer) {
      activeUndoPlaceholder.timer = setTimeout(() => {
        clearActiveUndoPlaceholder();
      }, 1000);
      const prog = el.querySelector('.undo-progress') as HTMLElement;
      if (prog) prog.style.animationPlayState = 'running';
    }
  });

  return el;
}

/**
 * Dismisses and removes the active undo placeholder with a smooth fade-out.
 */
function clearActiveUndoPlaceholder() {
  if (!activeUndoPlaceholder) return;
  if (activeUndoPlaceholder.timer) {
    clearTimeout(activeUndoPlaceholder.timer);
    activeUndoPlaceholder.timer = null;
  }
  const phEl = document.querySelector('.schedule-undo-placeholder') as HTMLElement;
  if (phEl) {
    phEl.style.opacity = '0';
    phEl.style.transform = 'translateY(-4px)';
    setTimeout(() => {
      activeUndoPlaceholder = null;
      phEl.remove();
      const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : null;
      if (sched) {
        lastRenderedScheduleKey = '';
        renderSchedule(sched);
      }
    }, 250);
  } else {
    activeUndoPlaceholder = null;
  }
}

/**
 * Triggered when clicking Undo on any placeholder or pressing Ctrl+Z during placeholder lifespan.
 */
export function triggerUndoFromPlaceholder() {
  if (activeUndoPlaceholder && activeUndoPlaceholder.timer) {
    clearTimeout(activeUndoPlaceholder.timer);
    activeUndoPlaceholder.timer = null;
  }
  const restoredTitle = activeUndoPlaceholder ? activeUndoPlaceholder.title : 'item';
  activeUndoPlaceholder = null;
  sendCommand('Undo');
  showToast(`✓ Restored "${restoredTitle}"`, 'success');
  const activeSched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : null;
  if (activeSched) {
    lastRenderedScheduleKey = '';
    renderSchedule(activeSched);
  }
}

/**
 * Deletes a specific schedule item by index with a temporary 1-second undo placeholder.
 */
function deleteScheduleItemByIndex(index: number, title?: string) {
  const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : null;
  if (!sched || !sched.items || index < 0 || index >= sched.items.length) return;
  const itemTitle = title || sched.items[index]?.title || 'Item';

  if (activeUndoPlaceholder && activeUndoPlaceholder.timer) {
    clearTimeout(activeUndoPlaceholder.timer);
    activeUndoPlaceholder.timer = null;
  }

  // Send backend command to remove item from schedule
  sendCommand({ RemoveFromSchedule: index });

  // Create new placeholder that hangs around for ~1.5s
  activeUndoPlaceholder = {
    index: index,
    title: itemTitle,
    createdAt: Date.now(),
    timer: setTimeout(() => {
      clearActiveUndoPlaceholder();
    }, 1500)
  };

  lastRenderedScheduleKey = '';
  renderSchedule(sched);
}

// ============================================================================
// SLIDE BADGE & VERSE NUMBER PRESERVATION HELPERS
// ============================================================================
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

export interface ActiveSelectedSlide {
  itemIndex: number;
  slideIndex: number;
  context: 'schedule' | 'preview' | 'live';
}

let activeSelectedSlide: ActiveSelectedSlide | null = null;

export function getActiveSelectedSlide(): ActiveSelectedSlide | null {
  return activeSelectedSlide;
}

export function setActiveSelectedSlide(sel: ActiveSelectedSlide | null) {
  activeSelectedSlide = sel;
  try { globalThis.activeSelectedSlide = sel; } catch (_) {}
  updateSlideSelectionVisuals();
}

export function clearActiveSelectedSlide() {
  activeSelectedSlide = null;
  try { globalThis.activeSelectedSlide = null; } catch (_) {}
  updateSlideSelectionVisuals();
}

export function updateSlideSelectionVisuals() {
  if (scheduleListEl) {
    const childItems = scheduleListEl.querySelectorAll('.schedule-child-item');
    childItems.forEach((el: Element) => {
      const itIdx = Number((el as HTMLElement).dataset.itemIndex);
      const slIdx = Number((el as HTMLElement).dataset.slideIndex);
      const isSelected = activeSelectedSlide &&
        activeSelectedSlide.itemIndex === itIdx &&
        activeSelectedSlide.slideIndex === slIdx;
      el.classList.toggle('selected-child-slide', !!isSelected);
    });
  }

  if (previewSlideMatrixEl) {
    const cards = previewSlideMatrixEl.querySelectorAll('.slide-card');
    cards.forEach((el: Element, idx: number) => {
      const isSelected = activeSelectedSlide &&
        activeSelectedSlide.context === 'preview' &&
        activeSelectedSlide.slideIndex === idx;
      el.classList.toggle('active-staged', !!isSelected || currentSnapshot?.state.staged_slide_index === idx);
    });
  }
}

/**
 * Deletes the currently selected slide across Schedule, Preview, or Live.
 * If the parent item has only 1 slide, removes the entire item with undo.
 */
export function deleteSelectedSlideOrItem(): boolean {
  // If activeSelectedSlide is null, check if preview has a staged slide and preview is active
  if (!activeSelectedSlide && lastActiveDeckContext === 'preview' && currentSnapshot && currentSnapshot.state.staged_item) {
    const stagedItem = currentSnapshot.state.staged_item;
    const sched = currentSnapshot.schedule;
    const itemIdx = (sched && sched.items) ? sched.items.findIndex((it) => it.id === stagedItem.id) : -1;
    if (itemIdx >= 0) {
      activeSelectedSlide = {
        itemIndex: itemIdx,
        slideIndex: currentSnapshot.state.staged_slide_index || 0,
        context: 'preview'
      };
    }
  }

  if (!activeSelectedSlide) return false;
  const snap = currentSnapshot;
  if (!snap) return false;

  const sched = (snap && snap.schedule) ? snap.schedule : null;
  if (!sched || !sched.items || sched.items.length === 0) return false;

  let itemIdx = activeSelectedSlide.itemIndex;
  if (itemIdx < 0 || itemIdx >= sched.items.length) {
    const stagedNow = snap.state.staged_item;
    const liveNow = snap.state.live_item;
    if (activeSelectedSlide.context === 'preview' && stagedNow) {
      itemIdx = sched.items.findIndex((it) => it.id === stagedNow.id);
    } else if (activeSelectedSlide.context === 'live' && liveNow) {
      itemIdx = sched.items.findIndex((it) => it.id === liveNow.id);
    }
  }

  if (itemIdx < 0 || itemIdx >= sched.items.length) return false;

  const item = sched.items[itemIdx];
  if (!item || !item.slides || item.slides.length === 0) return false;

  const slideIdx = activeSelectedSlide.slideIndex;
  if (slideIdx < 0 || slideIdx >= item.slides.length) return false;

  const slideToDelete = item.slides[slideIdx];
  const slideTag = getSlideBadge(slideToDelete, slideIdx);

  if (item.slides.length > 1) {
    sendCommand({ RemoveItemSlide: { item_index: itemIdx, slide_index: slideIdx } });

    // Optimistically update local schedule and state
    item.slides.splice(slideIdx, 1);
    if (snap.state.staged_item && snap.state.staged_item.id === item.id) {
      snap.state.staged_item.slides = item.slides.slice();
      if (snap.state.staged_slide_index >= item.slides.length) {
        snap.state.staged_slide_index = Math.max(0, item.slides.length - 1);
      }
    }
    if (snap.state.live_item && snap.state.live_item.id === item.id) {
      snap.state.live_item.slides = item.slides.slice();
      if (snap.state.live_slide_index >= item.slides.length) {
        snap.state.live_slide_index = Math.max(0, item.slides.length - 1);
      }
    }

    const nextSlideIdx = Math.min(slideIdx, item.slides.length - 1);
    activeSelectedSlide = {
      itemIndex: itemIdx,
      slideIndex: nextSlideIdx,
      context: activeSelectedSlide.context
    };

    lastRenderedScheduleKey = '';
    lastRenderedPreviewKey = '';
    lastRenderedLiveKey = '';
    renderSchedule(sched);
    renderPreviewDeck(snap.state);
    renderLiveDeck(snap.state);
    renderPreviewOutput(snap.state);

    showToast(`✓ Removed slide (${slideTag})`, 'info');
    return true;
  } else {
    // Only 1 slide left in the item: removing it removes the entire item from schedule
    clearActiveSelectedSlide();
    deleteScheduleItemByIndex(itemIdx, item.title);
    return true;
  }
}

/**
 * Deletes currently selected item across Schedule, Preview, or Live decks with undo placeholder.
 */
export function deleteActiveSelectedItemWithUndo() {
  const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : null;
  const state = (currentSnapshot && currentSnapshot.state) ? currentSnapshot.state : null;
  if (!sched || !sched.items || sched.items.length === 0) {
    return;
  }

  const resolved = resolveTargetItemToDelete({
    activeContext: lastActiveDeckContext,
    scheduleItems: sched.items,
    selectedItemIndex: state?.selected_item_index,
    expandedScheduleIndex: expandedScheduleIndex,
    stagedItemId: state?.staged_item?.id,
    liveItemId: state?.live_item?.id
  });

  if (!resolved) return;

  deleteScheduleItemByIndex(resolved.index, resolved.title);
}

try {
  globalThis.getActiveSelectedSlide = getActiveSelectedSlide;
  globalThis.setActiveSelectedSlide = setActiveSelectedSlide;
  globalThis.clearActiveSelectedSlide = clearActiveSelectedSlide;
  globalThis.getSlideBadge = getSlideBadge;
  globalThis.getSlideBadgeColor = getSlideBadgeColor;
  globalThis.deleteScheduleItemByIndex = deleteScheduleItemByIndex;
  globalThis.clearActiveUndoPlaceholder = clearActiveUndoPlaceholder;
} catch (_) {}

export function renderSchedule(schedule: Schedule) {
  if (!scheduleListEl) return;
  if (scheduleTitleEl) scheduleTitleEl.textContent = schedule.title || 'UNTITLED';

  const liveItemId = currentSnapshot && currentSnapshot.state.live_item ? currentSnapshot.state.live_item.id : '';
  const liveSlideIdx = currentSnapshot && currentSnapshot.state.live_slide_index !== undefined ? currentSnapshot.state.live_slide_index : -1;
  const itemsCount = schedule.items ? schedule.items.length : 0;
  // `schedule_version` (src/core/models.rs) is an authoritative O(1) proxy for "did any
  // schedule item/slide field change" — it's bumped server-side on every schedule-mutating
  // event, a strict superset of what the old per-call itemsFingerprint (a nested map/join
  // over every item and every slide) detected. Avoids rescanning the whole schedule on every
  // WS message just to find out nothing changed. Call sites that mutate schedule.items
  // in-place and re-render optimistically (before schedule_version's server round-trip lands)
  // reset lastRenderedScheduleKey = '' themselves rather than relying on this key to notice.
  const itemsFingerprint = schedule.schedule_version !== undefined ? schedule.schedule_version : '';
  const phKey = activeUndoPlaceholder ? `${activeUndoPlaceholder.index}:${activeUndoPlaceholder.title}` : '';
  const selSlideKey = activeSelectedSlide ? `sel:${activeSelectedSlide.itemIndex}:${activeSelectedSlide.slideIndex}` : '';
  const selectedItemIndex = currentSnapshot && currentSnapshot.state ? currentSnapshot.state.selected_item_index : null;
  const currentKey = `${schedule.title}|${selectedItemIndex}|${expandedScheduleIndex}|${itemsCount}|${schedule.is_modified}|${liveItemId}|${liveSlideIdx}|${Array.from(collapsedGroupIds).sort().join(',')}|${itemsFingerprint}|${phKey}|${selSlideKey}`;

  if (lastRenderedScheduleKey === currentKey) {
    return;
  }
  lastRenderedScheduleKey = currentKey;

  // Clean up previous Pragmatic DND element listeners
  scheduleDndCleanups.forEach(fn => {
    try { fn(); } catch (_) {}
  });
  scheduleDndCleanups = [];

  // Register Pragmatic DnD Drop Target on main schedule list container
  scheduleDndCleanups.push(
    dropTargetForElements({
      element: scheduleListEl,
      canDrop: ({ source }) => {
        const d = source.data;
        return d.type === 'catalog-item' || d.type === 'media-item' || d.type === 'theme-item';
      },
      onDrop: ({ source, location }) => {
        // If an inner drop target (e.g. itemEl, headerEl, bottomDropZone) was the primary target, let it handle it
        if (!isPrimaryDropTarget(scheduleListEl, location.current.dropTargets)) {
          return;
        }
        const d = source.data;
        if (d.type === 'catalog-item' || d.type === 'media-item' || d.type === 'theme-item') {
          const tab = (d.tabName as string) || (d.type === 'media-item' ? 'media' : (d.type === 'theme-item' ? 'themes' : 'songs'));
          const id = (d.itemId || d.id) as string;
          if (tab && id) {
            addItemToSchedule(tab, id);
            showToast(`✓ Added "${d.title || d.name}" to Schedule`, 'success');
          }
        }
      }
    })
  );

  scheduleListEl.innerHTML = '';

  if (!schedule.items || schedule.items.length === 0) {
    if (activeUndoPlaceholder) {
      scheduleListEl.appendChild(createUndoPlaceholderElement(activeUndoPlaceholder));
    }
    const emptyDiv = document.createElement('div');
    emptyDiv.className = 'ui-empty-state ui-empty-state--compact';
    emptyDiv.innerHTML = `
      <div class="empty-state-icon">📋</div>
      <div class="empty-state-title">Schedule is empty.</div>
      <div class="empty-state-desc">Click <strong>➕</strong> or drag items from library to start building your service.</div>
    `;
    scheduleListEl.appendChild(emptyDiv);
    return;
  }

  // Ensure expandedScheduleIndex stays within bounds
  if (expandedScheduleIndex !== null && expandedScheduleIndex >= schedule.items.length) {
    expandedScheduleIndex = schedule.items.length - 1;
  }

  let currentGroupIsCollapsed = false;

  schedule.items.forEach((item, idx: number) => {
    if (activeUndoPlaceholder && activeUndoPlaceholder.index === idx) {
      scheduleListEl.appendChild(createUndoPlaceholderElement(activeUndoPlaceholder));
    }
    const isSelected = selectedItemIndex === idx;
    const isLive = currentSnapshot && currentSnapshot.state.live_item && currentSnapshot.state.live_item.id === item.id;
    const isExpanded = (expandedScheduleIndex === idx);
    const typeLower = (item.item_type || '').toLowerCase();

    // SECTION / GROUP HEADER ITEM
    if (typeLower === 'header' || typeLower === 'group') {
      let groupItemCount = 0;
      for (let j = idx + 1; j < schedule.items.length; j++) {
        const nextType = (schedule.items[j]?.item_type || '').toLowerCase();
        if (nextType === 'header' || nextType === 'group') break;
        groupItemCount++;
      }

      const isGroupCollapsed = collapsedGroupIds.has(item.id);
      currentGroupIsCollapsed = isGroupCollapsed;

      const headerEl = document.createElement('div');
      headerEl.className = `schedule-item schedule-section-header ${isSelected ? 'selected' : ''}`;
      headerEl.dataset.index = String(idx);
      headerEl.setAttribute('data-schedule-key', `item-${item.id}`);
      // Look comes from .schedule-section-header in style.css; only the
      // position-dependent spacing is set here.
      headerEl.style.marginTop = idx > 0 ? '6px' : '0';
      headerEl.style.marginBottom = '2px';

      headerEl.innerHTML = `
        <span class="group-caret">${isGroupCollapsed ? '▶' : '▼'}</span>
        <span class="schedule-section-icon">🏷️</span>
        <div class="meta">
          <div class="title">
            ${escapeUserHtml(item.title)}
            ${isGroupCollapsed && groupItemCount > 0 ? `<span class="group-count-badge">(${groupItemCount} ${groupItemCount === 1 ? 'item' : 'items'})</span>` : ''}
          </div>
        </div>
        <span class="schedule-drag-handle drag-handle" data-item-idx="${idx}" role="button" tabindex="0" title="Drag handle to reorder" aria-label="Reorder section. Press Alt+Up or Alt+Down to move.">⠿</span>
      `;

      // Toggle group collapse on click (excluding the drag handle)
      headerEl.addEventListener('click', (e) => {
        if ((e.target as Element | null)?.closest('.schedule-drag-handle')) return;
        if (collapsedGroupIds.has(item.id)) {
          collapsedGroupIds.delete(item.id);
        } else {
          collapsedGroupIds.add(item.id);
        }
        lastRenderedScheduleKey = '';
        renderSchedule(schedule);
      });

      headerEl.addEventListener('contextmenu', (e) => {
        e.preventDefault();
        e.stopPropagation();
        currentContextMenuTarget = { type: 'schedule-item', index: idx, item: item };
        updateScheduleMergeMenuVisibility(idx, item, schedule);
        showContextMenu(document.getElementById('schedule-item-context-menu'), e.clientX, e.clientY);
      });

      // Drag Handle Keyboard Navigation (Alt+Up / Alt+Down)
      const headerHandle = headerEl.querySelector<HTMLElement>('.schedule-drag-handle');
      if (headerHandle) {
        headerHandle.addEventListener('click', (e) => e.stopPropagation());
        headerHandle.addEventListener('keydown', (e) => {
          if ((e.altKey || e.ctrlKey) && e.key === 'ArrowUp') {
            e.preventDefault();
            e.stopPropagation();
            if (idx > 0) {
              reorderScheduleItemsAnimated(schedule, idx, idx - 1, `.schedule-drag-handle[data-item-idx="${idx - 1}"]`);
            }
          } else if ((e.altKey || e.ctrlKey) && e.key === 'ArrowDown') {
            e.preventDefault();
            e.stopPropagation();
            const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : schedule;
            if (sched && sched.items && idx < sched.items.length - 1) {
              reorderScheduleItemsAnimated(schedule, idx, idx + 1, `.schedule-drag-handle[data-item-idx="${idx + 1}"]`);
            }
          }
        });

        // Register Pragmatic DnD Draggable on Section Header
        scheduleDndCleanups.push(
          draggable({
            element: headerEl,
            dragHandle: headerHandle,
            getInitialData: () => ({ type: 'schedule-item', index: idx, id: item.id }),
            onDragStart: () => headerEl.classList.add('dragging'),
            onDrop: () => headerEl.classList.remove('dragging')
          })
        );
      }

      // Register Pragmatic DnD Drop Target on Section Header
      scheduleDndCleanups.push(
        dropTargetForElements({
          element: headerEl,
          canDrop: ({ source }) => {
            const d = source.data;
            return d.type === 'schedule-item' || d.type === 'catalog-item' || d.type === 'media-item' || d.type === 'theme-item';
          },
          getData: ({ input, element }) => {
            return attachClosestEdge({ index: idx, type: 'schedule-item' }, {
              input,
              element,
              allowedEdges: ['top', 'bottom']
            });
          },
          onDragEnter: ({ self, source }) => {
            if (source.data.type === 'schedule-item') {
              const fromIdx = source.data.index as number;
              if (fromIdx === idx) return;
              const edge = extractClosestEdge(self.data);
              headerEl.classList.remove('drop-above', 'drop-below');
              const toIdx = getReorderDestinationIndex({ startIndex: fromIdx, indexOfTarget: idx, closestEdgeOfTarget: edge, axis: 'vertical' });
              if (toIdx !== fromIdx) {
                if (edge === 'top') headerEl.classList.add('drop-above');
                else if (edge === 'bottom') headerEl.classList.add('drop-below');
              }
            } else {
              headerEl.classList.add('dragover-active');
            }
          },
          onDrag: ({ self, source }) => {
            if (source.data.type === 'schedule-item') {
              const fromIdx = source.data.index as number;
              if (fromIdx === idx) return;
              const edge = extractClosestEdge(self.data);
              headerEl.classList.remove('drop-above', 'drop-below');
              const toIdx = getReorderDestinationIndex({ startIndex: fromIdx, indexOfTarget: idx, closestEdgeOfTarget: edge, axis: 'vertical' });
              if (toIdx !== fromIdx) {
                if (edge === 'top') headerEl.classList.add('drop-above');
                else if (edge === 'bottom') headerEl.classList.add('drop-below');
              }
            }
          },
          onDragLeave: () => {
            headerEl.classList.remove('drop-above', 'drop-below', 'dragover-active');
          },
          onDrop: ({ self, source }) => {
            headerEl.classList.remove('drop-above', 'drop-below', 'dragover-active');
            const d = source.data;
            if (d.type === 'schedule-item') {
              const fromIdx = d.index as number;
              if (fromIdx !== idx) {
                const edge = extractClosestEdge(self.data);
                const toIdx = getReorderDestinationIndex({ startIndex: fromIdx, indexOfTarget: idx, closestEdgeOfTarget: edge, axis: 'vertical' });
                if (fromIdx !== toIdx) {
                  reorderScheduleItemsAnimated(schedule, fromIdx, toIdx);
                }
              }
            } else if (d.type === 'catalog-item' || d.type === 'media-item' || d.type === 'theme-item') {
              const tab = (d.tabName as string) || (d.type === 'media-item' ? 'media' : (d.type === 'theme-item' ? 'themes' : 'songs'));
              const id = (d.itemId || d.id) as string;
              if (tab && id) {
                addItemToSchedule(tab, id);
                showToast(`✓ Added "${d.title || d.name}" to Schedule`, 'success');
              }
            }
          }
        })
      );

      scheduleListEl.appendChild(headerEl);
      return;
    }

    // If current group is collapsed, hide item completely
    if (currentGroupIsCollapsed) {
      return;
    }

    // STANDARD SCHEDULE ITEM (Song, Scripture, Media, Presentation)
    const itemEl = document.createElement('div');
    itemEl.className = `schedule-item ${isSelected ? 'selected' : ''} ${isLive ? 'live-item' : ''}`;
    itemEl.dataset.index = String(idx);
    itemEl.setAttribute('data-schedule-key', `item-${item.id}`);

    let icon = '🎵';
    if (typeLower.includes('scripture')) {
      icon = '📖';
    } else if (typeLower.includes('media')) {
      icon = '🎬';
    } else if (typeLower.includes('presentation')) {
      icon = '📊';
    } else if (typeLower.includes('song')) {
      icon = '🎵';
    }

    let gradient = 'linear-gradient(135deg, #0f2027, #203a43)';
    if (typeLower.includes('scripture')) {
      gradient = 'linear-gradient(135deg, #1f1c18, #473e34)';
    } else if (typeLower.includes('media')) {
      gradient = 'linear-gradient(135deg, #141e30, #243b55)';
    } else if (typeLower.includes('presentation')) {
      gradient = 'linear-gradient(135deg, #2c3e50, #3498db)';
    }

    if (item.slides && item.slides[0] && item.slides[0].background) {
      const bg = item.slides[0].background;
      if (isVideoBackground(bg)) {
        gradient = `linear-gradient(135deg, #001f3f, #0074d9)`;
      } else {
        gradient = formatCssBackground(bg, gradient, availableThemes);
      }
    } else if (item.theme_name) {
      const t = availableThemes.find(x => x.name === item.theme_name);
      if (t) gradient = formatCssBackground(t.bg, gradient, availableThemes);
    }

    const caret = (item.slides && item.slides.length > 1) ? (isExpanded ? '▼' : '▶') : '';

    itemEl.innerHTML = `
      <span class="schedule-caret">${caret}</span>
      <div class="thumb-box" style="background: ${escapeHtml(gradient)};">${icon}</div>
      <div class="meta schedule-item-meta">
        <div class="title">${escapeUserHtml(item.title)}</div>
        <div class="notes">${escapeUserHtml(item.subtitle || 'notes')}</div>
      </div>
      <span class="schedule-drag-handle drag-handle" data-item-idx="${idx}" role="button" tabindex="0" title="Drag handle to reorder" aria-label="Reorder item. Press Alt+Up or Alt+Down to move.">⠿</span>
    `;

    // Caret Click: Toggles expansion
    const caretEl = itemEl.querySelector('.schedule-caret');
    if (caretEl && item.slides && item.slides.length > 1) {
      caretEl.addEventListener('click', (e) => {
        e.stopPropagation();
        if (expandedScheduleIndex === idx) {
          expandedScheduleIndex = null;
        } else {
          expandedScheduleIndex = idx;
        }
        const activeSched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : schedule;
        lastRenderedScheduleKey = '';
        renderSchedule(activeSched);
      });
    }

    // Row Click: Stage item and auto-expand clicked item
    itemEl.addEventListener('click', (e) => {
      if ((e.target as Element | null)?.closest('.schedule-caret') || (e.target as Element | null)?.closest('.schedule-drag-handle')) return;
      if (expandedScheduleIndex !== idx && item.slides && item.slides.length > 1) {
        expandedScheduleIndex = idx;
        const activeSched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : schedule;
        lastRenderedScheduleKey = '';
        renderSchedule(activeSched);
      }
      clearAdhocPreview();
      sendCommand({ StageItem: { item_index: idx, slide_index: 0 } });
    });

    itemEl.addEventListener('dblclick', (e) => {
      if ((e.target as Element | null)?.closest('.schedule-drag-handle')) return;
      sendCommand({ GoLive: { item_index: idx, slide_index: 0 } });
    });

    // Right-Click Context Menu on Schedule Item
    itemEl.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      currentContextMenuTarget = { type: 'schedule-item', index: idx, item: item };
      updateScheduleMergeMenuVisibility(idx, item, schedule);
      showContextMenu(document.getElementById('schedule-item-context-menu'), e.clientX, e.clientY);
    });

    // Drag Handle Keyboard Navigation (Alt+Up / Alt+Down)
    const itemHandle = itemEl.querySelector<HTMLElement>('.schedule-drag-handle');
    if (itemHandle) {
      itemHandle.addEventListener('click', (e) => e.stopPropagation());
      itemHandle.addEventListener('keydown', (e) => {
        if ((e.altKey || e.ctrlKey) && e.key === 'ArrowUp') {
          e.preventDefault();
          e.stopPropagation();
          if (idx > 0) {
            reorderScheduleItemsAnimated(schedule, idx, idx - 1, `.schedule-drag-handle[data-item-idx="${idx - 1}"]`);
          }
        } else if ((e.altKey || e.ctrlKey) && e.key === 'ArrowDown') {
          e.preventDefault();
          e.stopPropagation();
          const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : schedule;
          if (sched && sched.items && idx < sched.items.length - 1) {
            reorderScheduleItemsAnimated(schedule, idx, idx + 1, `.schedule-drag-handle[data-item-idx="${idx + 1}"]`);
          }
        }
      });

      // Register Pragmatic DnD Draggable on Schedule Item
      scheduleDndCleanups.push(
        draggable({
          element: itemEl,
          dragHandle: itemHandle,
          getInitialData: () => ({ type: 'schedule-item', index: idx, id: item.id }),
          onDragStart: () => itemEl.classList.add('dragging'),
          onDrop: () => itemEl.classList.remove('dragging')
        })
      );
    }

    // Register Pragmatic DnD Drop Target on Schedule Item
    scheduleDndCleanups.push(
      dropTargetForElements({
        element: itemEl,
        canDrop: ({ source }) => {
          const d = source.data;
          return d.type === 'schedule-item' || d.type === 'catalog-item' || d.type === 'media-item' || d.type === 'theme-item';
        },
        getData: ({ input, element }) => {
          return attachClosestEdge({ index: idx, type: 'schedule-item' }, {
            input,
            element,
            allowedEdges: ['top', 'bottom']
          });
        },
        onDragEnter: ({ self, source }) => {
          if (source.data.type === 'schedule-item') {
            const fromIdx = source.data.index as number;
            if (fromIdx === idx) return;
            const edge = extractClosestEdge(self.data);
            itemEl.classList.remove('drop-above', 'drop-below');
            const toIdx = getReorderDestinationIndex({ startIndex: fromIdx, indexOfTarget: idx, closestEdgeOfTarget: edge, axis: 'vertical' });
            if (toIdx !== fromIdx) {
              if (edge === 'top') itemEl.classList.add('drop-above');
              else if (edge === 'bottom') itemEl.classList.add('drop-below');
            }
          } else {
            itemEl.classList.add('dragover-active');
          }
        },
        onDrag: ({ self, source }) => {
          if (source.data.type === 'schedule-item') {
            const fromIdx = source.data.index as number;
            if (fromIdx === idx) return;
            const edge = extractClosestEdge(self.data);
            itemEl.classList.remove('drop-above', 'drop-below');
            const toIdx = getReorderDestinationIndex({ startIndex: fromIdx, indexOfTarget: idx, closestEdgeOfTarget: edge, axis: 'vertical' });
            if (toIdx !== fromIdx) {
              if (edge === 'top') itemEl.classList.add('drop-above');
              else if (edge === 'bottom') itemEl.classList.add('drop-below');
            }
          }
        },
        onDragLeave: () => {
          itemEl.classList.remove('drop-above', 'drop-below', 'dragover-active');
        },
        onDrop: ({ self, source }) => {
          itemEl.classList.remove('drop-above', 'drop-below', 'dragover-active');
          const d = source.data;
          if (d.type === 'schedule-item') {
            const fromIdx = d.index as number;
            if (fromIdx !== idx) {
              const edge = extractClosestEdge(self.data);
              const toIdx = getReorderDestinationIndex({ startIndex: fromIdx, indexOfTarget: idx, closestEdgeOfTarget: edge, axis: 'vertical' });
              if (fromIdx !== toIdx) {
                reorderScheduleItemsAnimated(schedule, fromIdx, toIdx);
              }
            }
          } else if (d.type === 'media-item') {
            const bgPath = (d.filePath || '') as string;
            const mType = ((d.mediaType || '') as string).toLowerCase();
            if (mType.includes('image') || mType.includes('still') || mType.includes('video') || isImageBackground(bgPath, availableThemes) || isVideoBackground(bgPath)) {
              sendCommand({
                SetItemTheme: {
                  item_index: idx,
                  theme_name: '',
                  background: bgPath
                }
              });
              showToast(`✓ Set background for "${item.title}" to ${d.name}`, 'success');
            } else {
              showToast('Only image or video media can be set as item backgrounds.', 'warning');
            }
          } else if (d.type === 'theme-item') {
            sendCommand({
              SetItemTheme: {
                item_index: idx,
                theme_name: (d.themeName || '') as string,
                background: (d.bg || '') as string
              }
            });
            showToast(`✓ Applied theme "${d.themeName}" to "${item.title}"`, 'success');
          } else if (d.type === 'catalog-item') {
            const tab = (d.tabName as string) || 'songs';
            const id = (d.itemId || d.id) as string;
            if (tab && id) {
              addItemToSchedule(tab, id);
              showToast(`✓ Added "${d.title || d.name}" to Schedule`, 'success');
            }
          }
        }
      })
    );

    scheduleListEl.appendChild(itemEl);

    // Only render child slides if THIS specific item is expanded
    const effectiveArr = effectiveArrangement(item);
    if (isExpanded && effectiveArr && effectiveArr.length > 1) {
      const childContainer = document.createElement('div');
      childContainer.className = 'schedule-child-slides';

      effectiveArr.forEach((entry, sIdx: number) => {
        const slide = resolveSlideAt(item, sIdx);
        const isCurrentLiveSlide = isLive && currentSnapshot?.state.live_slide_index === sIdx;
        const isSelectedSlide = (activeSelectedSlide &&
          activeSelectedSlide.itemIndex === idx &&
          activeSelectedSlide.slideIndex === sIdx);

        const childEl = document.createElement('div');
        childEl.className = `schedule-child-item ${isCurrentLiveSlide ? 'live-child' : ''} ${isSelectedSlide ? 'selected-child-slide' : ''}`;
        childEl.dataset.slideIndex = String(sIdx);
        childEl.dataset.itemIndex = String(idx);
        childEl.setAttribute('data-schedule-key', `slide-${item.id}-${sIdx}`);

        const badge = entry.section_id || (slide ? getSlideBadge(slide, entry.source_slide_index) : `S${sIdx + 1}`);
        const badgeColor = getSlideBadgeColor(badge);

        const firstLine = (slide ? slide.text || '' : '').split('\n')[0] || '';

        const effectiveBg = entry.background_override || (slide ? slide.background : null);
        let slideMediaBadge = '';
        if (effectiveBg) {
          if (isVideoBackground(effectiveBg)) {
            slideMediaBadge = '<span title="Video background assigned" style="font-size: 10px; margin-left: 4px; opacity: 0.85;">🎬</span>';
          } else if (isAudioMedia(effectiveBg)) {
            slideMediaBadge = '<span title="Audio track assigned" style="font-size: 10px; margin-left: 4px; opacity: 0.85;">🎵</span>';
          } else if (extractImageUrl(effectiveBg, availableThemes)) {
            slideMediaBadge = '<span title="Image background assigned" style="font-size: 10px; margin-left: 4px; opacity: 0.85;">🖼️</span>';
          }
        }

        childEl.innerHTML = `
          <span class="child-slide-drag-handle drag-handle" data-slide-idx="${sIdx}" role="button" tabindex="0" title="Drag handle to reorder verse" aria-label="Reorder verse. Press Alt+Up or Alt+Down to move.">⠿</span>
          <span style="font-weight: 700; color: ${badgeColor}; width: 28px; font-size: 11px;">${escapeHtml(badge)}</span>
          <span style="flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 11px;">${escapeUserHtml(firstLine)}</span>
          ${slideMediaBadge}
        `;

        childEl.addEventListener('click', (e) => {
          if ((e.target as Element | null)?.closest('.child-slide-drag-handle')) return;
          if (childEl.classList.contains('dragging')) return;
          e.stopPropagation();
          lastActiveDeckContext = 'schedule';
          setActiveSelectedSlide({ itemIndex: idx, slideIndex: sIdx, context: 'schedule' });
          clearAdhocPreview();
          sendCommand({ StageItem: { item_index: idx, slide_index: sIdx } });
        });
        childEl.addEventListener('dblclick', (e) => {
          if ((e.target as Element | null)?.closest('.child-slide-drag-handle')) return;
          if (childEl.classList.contains('dragging')) return;
          e.stopPropagation();
          lastActiveDeckContext = 'schedule';
          setActiveSelectedSlide({ itemIndex: idx, slideIndex: sIdx, context: 'schedule' });
          sendCommand({ GoLive: { item_index: idx, slide_index: sIdx } });
        });

        // Child Drag Handle Keyboard Navigation (Alt+Up / Alt+Down)
        const childHandle = childEl.querySelector<HTMLElement>('.child-slide-drag-handle');
        if (childHandle) {
          childHandle.addEventListener('click', (e) => e.stopPropagation());
          childHandle.addEventListener('keydown', (e) => {
            if ((e.altKey || e.ctrlKey) && e.key === 'ArrowUp') {
              e.preventDefault();
              e.stopPropagation();
              if (sIdx > 0) {
                reorderChildSlidesAnimated(schedule, idx, sIdx, sIdx - 1, `.child-slide-drag-handle[data-slide-idx="${sIdx - 1}"]`);
              }
            } else if ((e.altKey || e.ctrlKey) && e.key === 'ArrowDown') {
              e.preventDefault();
              e.stopPropagation();
              if (sIdx < effectiveArr.length - 1) {
                reorderChildSlidesAnimated(schedule, idx, sIdx, sIdx + 1, `.child-slide-drag-handle[data-slide-idx="${sIdx + 1}"]`);
              }
            }
          });
        }

        // Register Pragmatic DnD Draggable on Child Slide (drag handle or row)
        scheduleDndCleanups.push(
          draggable({
            element: childEl,
            getInitialData: () => ({ type: 'child-slide', itemIndex: idx, slideIndex: sIdx, itemId: item.id }),
            onDragStart: () => childEl.classList.add('dragging'),
            onDrop: () => childEl.classList.remove('dragging')
          })
        );

        // Register Pragmatic DnD Drop Target on Child Slide
        scheduleDndCleanups.push(
          dropTargetForElements({
            element: childEl,
            canDrop: ({ source }) => {
              const d = source.data;
              if (d.type === 'child-slide' && d.itemIndex === idx) return true;
              if (d.type === 'schedule-item' && d.index !== idx) return true;
              if (d.type === 'media-item' || d.type === 'theme-item') return true;
              return false;
            },
            getData: ({ input, element }) => {
              return attachClosestEdge({ itemIndex: idx, slideIndex: sIdx, type: 'child-slide' }, {
                input,
                element,
                allowedEdges: ['top', 'bottom']
              });
            },
            onDragEnter: ({ self, source }) => {
              if (source.data.type === 'child-slide' && source.data.itemIndex === idx) {
                const fromIdx = source.data.slideIndex as number;
                if (fromIdx === sIdx) return;
                const edge = extractClosestEdge(self.data);
                childEl.classList.remove('drop-above', 'drop-below');
                const toIdx = computeSafeDestinationIndex(fromIdx, sIdx, edge, 'vertical');
                if (toIdx !== fromIdx) {
                  if (edge === 'top') childEl.classList.add('drop-above');
                  else if (edge === 'bottom') childEl.classList.add('drop-below');
                }
              } else if (source.data.type === 'schedule-item') {
                childEl.classList.add('dragover-active');
              } else {
                childEl.classList.add('dragover-active');
              }
            },
            onDrag: ({ self, source }) => {
              if (source.data.type === 'child-slide' && source.data.itemIndex === idx) {
                const fromIdx = source.data.slideIndex as number;
                if (fromIdx === sIdx) return;
                const edge = extractClosestEdge(self.data);
                childEl.classList.remove('drop-above', 'drop-below');
                const toIdx = computeSafeDestinationIndex(fromIdx, sIdx, edge, 'vertical');
                if (toIdx !== fromIdx) {
                  if (edge === 'top') childEl.classList.add('drop-above');
                  else if (edge === 'bottom') childEl.classList.add('drop-below');
                }
              }
            },
            onDragLeave: () => {
              childEl.classList.remove('drop-above', 'drop-below', 'dragover-active');
            },
            onDrop: ({ self, source }) => {
              childEl.classList.remove('drop-above', 'drop-below', 'dragover-active');
              const d = source.data;
              if (d.type === 'child-slide' && d.itemIndex === idx) {
                const fromIdx = d.slideIndex as number;
                if (fromIdx !== sIdx) {
                  const edge = extractClosestEdge(self.data);
                  const toIdx = computeSafeDestinationIndex(fromIdx, sIdx, edge, 'vertical');
                  if (fromIdx !== toIdx) {
                    reorderChildSlidesAnimated(schedule, idx, fromIdx, toIdx);
                  }
                }
              } else if (d.type === 'schedule-item') {
                const fromIdx = d.index as number;
                if (fromIdx !== idx) {
                  const isLastSlide = sIdx === (item.slides ? item.slides.length - 1 : 0);
                  const edge = extractClosestEdge(self.data);
                  let toIdx = idx;
                  if (fromIdx > idx) {
                    toIdx = (isLastSlide && edge === 'bottom') ? idx + 1 : idx;
                  }
                  if (fromIdx !== toIdx) {
                    reorderScheduleItemsAnimated(schedule, fromIdx, toIdx);
                  }
                }
              } else if (d.type === 'media-item') {
                const bgPath = (d.filePath || '') as string;
                const mType = ((d.mediaType || '') as string).toLowerCase();
                const isAud = mType.includes('audio') || isAudioMedia(bgPath);
                const isVid = mType.includes('video') || isVideoBackground(bgPath);
                const isImg = mType.includes('image') || mType.includes('still') || isImageBackground(bgPath, availableThemes);

                if (isAud || isVid || isImg || bgPath) {
                  sendCommand({
                    SetSlideBackground: {
                      item_index: idx,
                      slide_index: sIdx,
                      background: bgPath
                    }
                  });
                  const mediaKind = isAud ? 'audio track' : (isVid ? 'video background' : 'image background');
                  showToast(`✓ Assigned ${mediaKind} to slide ${sIdx + 1} (${slide.tag || 'Slide ' + (sIdx + 1)}): ${d.name}`, 'success');
                }
              } else if (d.type === 'theme-item') {
                sendCommand({
                  SetSlideBackground: {
                    item_index: idx,
                    slide_index: sIdx,
                    background: (d.bg || '') as string
                  }
                });
                showToast(`✓ Assigned theme "${d.themeName}" to slide ${sIdx + 1}`, 'success');
              }
            }
          })
        );

        // Right-Click Context Menu on Schedule Slide
        childEl.addEventListener('contextmenu', (e) => {
          e.preventDefault();
          e.stopPropagation();
          currentContextMenuTarget = { type: 'slide', itemIndex: idx, slideIndex: sIdx, item: item, slide: slide };
          updateSlideDurationMenuVisibility(item);
          showContextMenu(document.getElementById('slide-context-menu'), e.clientX, e.clientY);
        });

        childContainer.appendChild(childEl);
      });

      scheduleListEl.appendChild(childContainer);
    }
  });

  if (activeUndoPlaceholder && activeUndoPlaceholder.index >= schedule.items.length) {
    scheduleListEl.appendChild(createUndoPlaceholderElement(activeUndoPlaceholder));
  }

  // Dedicated Bottom Drop Zone (Append to end of schedule)
  const bottomDropZone = document.createElement('div');
  bottomDropZone.className = 'schedule-bottom-drop-zone';
  bottomDropZone.innerHTML = `
    <span style="font-size: 12px; opacity: 0.7;">＋</span>
    <span>Drop here to add to end of schedule</span>
  `;

  // Register Pragmatic DnD Drop Target on Bottom Drop Zone
  scheduleDndCleanups.push(
    dropTargetForElements({
      element: bottomDropZone,
      canDrop: ({ source }) => {
        const d = source.data;
        return d.type === 'schedule-item' || d.type === 'catalog-item' || d.type === 'media-item' || d.type === 'theme-item';
      },
      onDragEnter: () => bottomDropZone.classList.add('dragover-active'),
      onDragLeave: () => bottomDropZone.classList.remove('dragover-active'),
      onDrop: ({ source }) => {
        bottomDropZone.classList.remove('dragover-active');
        const d = source.data;
        if (d.type === 'schedule-item') {
          const fromIdx = d.index as number;
          const sched = (currentSnapshot && currentSnapshot.schedule) ? currentSnapshot.schedule : schedule;
          if (sched && sched.items && sched.items.length > 0) {
            const toIdx = sched.items.length - 1;
            if (fromIdx !== toIdx) {
              reorderScheduleItemsAnimated(schedule, fromIdx, toIdx);
            }
          }
        } else if (d.type === 'catalog-item' || d.type === 'media-item' || d.type === 'theme-item') {
          const tab = (d.tabName as string) || (d.type === 'media-item' ? 'media' : (d.type === 'theme-item' ? 'themes' : 'songs'));
          const id = (d.itemId || d.id) as string;
          if (tab && id) {
            addItemToSchedule(tab, id);
            showToast(`✓ Added "${d.title || d.name}" to end of Schedule`, 'success');
          }
        }
      }
    })
  );

  scheduleListEl.appendChild(bottomDropZone);
}

// ============================================================================
// LIVE DECK & OUTPUT MONITOR
// ============================================================================



let isLiveVideoLooping = false;
let liveAudioVolume = 1.0;
let liveAudioMuted = false;

// High-Precision Direct Inter-Window Synchronization Channel (<1ms latency)
// (Implementation delegated to src/core/media_sync.ts)
const mediaSyncManager = new MediaSyncManager(() => liveVideoEl as HTMLVideoElement | null);

function broadcastLocalMediaSync(action: MediaSyncPayload['action'], currentTime?: number, isPlaying?: boolean, isLooping?: boolean, executeAtEpoch?: number | null) {
  mediaSyncManager.setLooping(isLooping !== undefined ? isLooping : isLiveVideoLooping);
  mediaSyncManager.broadcastSync(action, currentTime, isPlaying, isLooping, executeAtEpoch);
}

function stopMediaSyncHeartbeat() {
  mediaSyncManager.stopHeartbeat();
}

const liveMediaControls = document.getElementById('live-media-controls');
const liveVideoSeek = document.getElementById('live-video-seek') as HTMLInputElement | null;
const liveVideoTime = document.getElementById('live-video-time');
const btnLiveVideoPlay = document.getElementById('btn-live-video-play');
const btnLiveVideoStop = document.getElementById('btn-live-video-stop');
const btnLiveVideoLoop = document.getElementById('btn-live-video-loop');
const btnLiveVideoMute = document.getElementById('btn-live-video-mute');
const liveVideoVolumeSlider = document.getElementById('live-video-volume') as HTMLInputElement | null;
const liveVideoEl = document.getElementById('live-canvas-video') as HTMLVideoElement | null;
const liveImageEl = document.getElementById('live-canvas-image');
const liveAudioEl = document.getElementById('live-canvas-audio') as HTMLAudioElement | null;

let appScheduledTimer: ReturnType<typeof setTimeout> | null = null;
let _appPlayStartedAt: number | null = null;
function armClientScheduledPlay(el: HTMLVideoElement | null, executeAtEpoch: number) {
  if (!el) return;
  if (appScheduledTimer) {
    clearTimeout(appScheduledTimer);
    appScheduledTimer = null;
  }

  const execute = () => {
    if (appScheduledTimer) {
      clearTimeout(appScheduledTimer);
      appScheduledTimer = null;
    }
    if (el.paused) {
      _appPlayStartedAt = performance.now(); // Grace period starts now
      const p = el.play();
      if (p && typeof p.catch === 'function') {
        p.catch(() => {
          el.muted = true;
          el.play().catch(() => {});
        });
      }
    }
  };

  const delayMs = Math.max(0, executeAtEpoch - Date.now());
  appScheduledTimer = setTimeout(execute, delayMs);
}

function requestSeamlessResume(targetPts: number, runwayMs = 300) {
  if (!liveVideoEl) return;
  const pts = (typeof targetPts === 'number') ? targetPts : (liveVideoEl.currentTime || 0);

  // 1. Hot Decoder Priming: Ensure target frame is primed in memory
  try { liveVideoEl.currentTime = pts; } catch (_) {}

  // 2. Two-Phase Commit with 300ms Execution Runway
  const executeAtEpoch = Date.now() + runwayMs;
  broadcastLocalMediaSync('play', pts, true, isLiveVideoLooping, executeAtEpoch);
  sendCommand({ MediaScheduledStart: { target_pts: pts, start_at_epoch_ms: executeAtEpoch } });

  // 3. Arm local client unpause trigger at T + 300ms
  armClientScheduledPlay(liveVideoEl, executeAtEpoch);
}

if (btnLiveVideoPlay) {
  btnLiveVideoPlay.addEventListener('click', () => {
    if (!liveVideoEl) return;
    const isPaused = liveVideoEl.paused;
    const curTime = liveVideoEl.currentTime || 0;

    if (isPaused) {
      btnLiveVideoPlay.textContent = '⏸ Pause';
      requestSeamlessResume(curTime);
    } else {
      btnLiveVideoPlay.textContent = '▶ Play';
      liveVideoEl.pause();
      broadcastLocalMediaSync('pause', curTime, false, isLiveVideoLooping);
      sendCommand({ MediaPause: { current_time: curTime } });
    }
  });
}

if (btnLiveVideoStop) {
  btnLiveVideoStop.addEventListener('click', () => {
    if (!liveVideoEl) return;
    liveVideoEl.pause();
    liveVideoEl.currentTime = 0;
    stopMediaSyncHeartbeat();
    if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '▶ Play';
    broadcastLocalMediaSync('stop', 0, false, isLiveVideoLooping);
    sendCommand('MediaStop');
  });
}

if (btnLiveVideoLoop) {
  btnLiveVideoLoop.addEventListener('click', () => {
    isLiveVideoLooping = !isLiveVideoLooping;
    if (liveVideoEl) liveVideoEl.loop = isLiveVideoLooping;
    btnLiveVideoLoop.textContent = isLiveVideoLooping ? '🔁 Loop: ON' : '🔁 Loop: OFF';
    btnLiveVideoLoop.style.color = isLiveVideoLooping ? 'var(--os-success-text)' : '';
    broadcastLocalMediaSync('loop', liveVideoEl ? liveVideoEl.currentTime : 0, liveVideoEl ? !liveVideoEl.paused : false, isLiveVideoLooping);
    sendCommand({ MediaSetLoop: isLiveVideoLooping });
  });
}

if (btnLiveVideoMute) {
  btnLiveVideoMute.addEventListener('click', () => {
    liveAudioMuted = !liveAudioMuted;
    if (liveVideoEl) liveVideoEl.muted = liveAudioMuted;
    if (liveAudioEl) liveAudioEl.muted = liveAudioMuted;
    btnLiveVideoMute.textContent = liveAudioMuted ? '🔇' : '🔊';
    sendCommand({ MediaSetMute: liveAudioMuted });
  });
}

if (liveVideoVolumeSlider) {
  liveVideoVolumeSlider.addEventListener('input', () => {
    liveAudioVolume = Number(liveVideoVolumeSlider.value) / 100;
    if (liveVideoEl) {
      liveVideoEl.volume = liveAudioVolume;
      if (liveAudioVolume > 0 && liveAudioMuted) {
        liveAudioMuted = false;
        liveVideoEl.muted = false;
        if (btnLiveVideoMute) btnLiveVideoMute.textContent = '🔊';
      }
    }
    if (liveAudioEl) {
      liveAudioEl.volume = liveAudioVolume;
      if (liveAudioVolume > 0 && liveAudioMuted) {
        liveAudioMuted = false;
        liveAudioEl.muted = false;
      }
    }
    sendCommand({ MediaSetVolume: liveAudioVolume });
  });
}

let isUserSeeking = false;
let wasPlayingBeforeScrub = false;

if (liveVideoSeek) {
  const onSeekStart = () => {
    isUserSeeking = true;
    wasPlayingBeforeScrub = liveVideoEl ? !liveVideoEl.paused : false;
    if (liveVideoEl && !liveVideoEl.paused) {
      liveVideoEl.pause();
    }
    if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '▶ Play';
  };

  liveVideoSeek.addEventListener('pointerdown', onSeekStart);
  liveVideoSeek.addEventListener('mousedown', onSeekStart);
  liveVideoSeek.addEventListener('touchstart', onSeekStart);

  // Preroll scrub lock while dragging
  liveVideoSeek.addEventListener('input', () => {
    isUserSeeking = true;
    if (liveVideoEl && !liveVideoEl.paused) {
      liveVideoEl.pause();
    }
    if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '▶ Play';

    if (liveVideoEl && liveVideoEl.duration && !isNaN(liveVideoEl.duration)) {
      const seekTime = (Number(liveVideoSeek.value) / 100) * liveVideoEl.duration;
      liveVideoEl.currentTime = seekTime;
      if (liveVideoTime) liveVideoTime.textContent = `${formatMediaTime(seekTime)} / ${formatMediaTime(liveVideoEl.duration)}`;
      broadcastLocalMediaSync('pause', seekTime, false, isLiveVideoLooping);
      sendCommand({ MediaPreroll: { target_pts: seekTime } });
    }
  });

  // When seek head is released after scrubbing -> resume after 300ms buffer sync runway!
  const onSeekRelease = () => {
    isUserSeeking = false;

    if (liveVideoEl && liveVideoEl.duration && !isNaN(liveVideoEl.duration)) {
      const seekTime = (Number(liveVideoSeek.value) / 100) * liveVideoEl.duration;
      liveVideoEl.currentTime = seekTime;
      if (liveVideoTime) liveVideoTime.textContent = `${formatMediaTime(seekTime)} / ${formatMediaTime(liveVideoEl.duration)}`;

      if (wasPlayingBeforeScrub) {
        if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '⏸ Pause';
        // Decisively trigger seamless resume after 300ms runway delay
        requestSeamlessResume(seekTime, 300);
      } else {
        // It was paused before the scrub (e.g. cueing a start point) --
        // stay paused at the new position instead of auto-playing, same as
        // the live drag/`input` handler above already does while dragging.
        if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '▶ Play';
        broadcastLocalMediaSync('pause', seekTime, false, isLiveVideoLooping);
        sendCommand({ MediaPreroll: { target_pts: seekTime } });
      }
    }
    wasPlayingBeforeScrub = false;
  };

  liveVideoSeek.addEventListener('change', onSeekRelease);
  liveVideoSeek.addEventListener('pointerup', onSeekRelease);
  liveVideoSeek.addEventListener('mouseup', onSeekRelease);
  liveVideoSeek.addEventListener('touchend', onSeekRelease);
}

if (liveVideoEl) {
  liveVideoEl.addEventListener('timeupdate', () => {
    if (isUserSeeking) return; // Do not overwrite slider while dragging seek head
    if (liveVideoEl.duration && !isNaN(liveVideoEl.duration)) {
      const pct = (liveVideoEl.currentTime / liveVideoEl.duration) * 100;
      if (liveVideoSeek) liveVideoSeek.value = String(pct);
      if (liveVideoTime) liveVideoTime.textContent = `${formatMediaTime(liveVideoEl.currentTime)} / ${formatMediaTime(liveVideoEl.duration)}`;
    }
  });

  liveVideoEl.addEventListener('play', () => {
    if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '⏸ Pause';
  });

  liveVideoEl.addEventListener('pause', () => {
    if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '▶ Play';
  });

  liveVideoEl.addEventListener('ended', () => {
    if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '▶ Play';
    if (!isLiveVideoLooping) {
      // Fade to black when video is over
      if (canvasBlackoutEl) {
        canvasBlackoutEl.classList.add('fading-black');
      }
      sendCommand('ToggleBlackout');
    }
  });
}

let lastRenderedPreviewKey = '';
let lastRenderedPreviewOutputKey = '';

// Ad-hoc library preview: viewing a resource that isn't (yet) on the Schedule.
// Purely a local rendering concern — never touches engine staged_item state,
// so it can't accidentally be sent Live via a stale item_index.
/** A state with nothing live or staged, for drawing the library preview before the first snapshot has arrived. */
const EMPTY_SHOW_STATE: ShowState = {
  live_item: null, live_slide_index: 0, staged_item: null, staged_slide_index: 0, is_blackout: false, is_clear_text: false,
  is_logo_override: false, alert_message: null, global_theme: null, global_background: null, sequence_number: 0, last_event_id: '', server_time_ms: 0,
};

let adhocPreviewItem: LibraryItem | null = null;
let adhocPreviewTabName = '';
let adhocPreviewSlideIndex = 0;

function clearAdhocPreview() {
  adhocPreviewItem = null;
  adhocPreviewTabName = '';
  adhocPreviewSlideIndex = 0;
}

function setAdhocPreview(item: LibraryItem | null, tabName: string) {
  adhocPreviewItem = item;
  adhocPreviewTabName = tabName;
  adhocPreviewSlideIndex = 0;
  const state = currentSnapshot?.state ?? EMPTY_SHOW_STATE;
  renderPreviewDeck(state);
  renderPreviewOutput(state);
}

/**
 * Normalizes an arbitrary library resource into a flat list of viewable
 * "slides" for the ad-hoc Preview panel. This is a display-only projection —
 * it does not attempt to replicate the server-side AddToSchedule conversion.
 */
function buildAdhocPreviewSlides(item: LibraryItem, tabName: string): Array<{ text: string; background?: string }> {
  if (!item) return [];

  if ((tabName === 'songs' || tabName === 'presentations') && Array.isArray(item.slides) && item.slides.length > 0) {
    return item.slides.map((s: LibrarySlide) => ({ text: s.text || s.content || s.title || '', background: s.background ?? undefined }));
  }

  if (tabName === 'scriptures') {
    if (Array.isArray(item.verses) && item.verses.length > 0) {
      return item.verses.map((v: ScriptureVerse) => ({ text: `${v.verse_number}. ${v.text}` }));
    }
    return [{ text: item.reference || item.text || '' }];
  }

  if (tabName === 'media') {
    return [{ text: '', background: item.file_path || item.url || '' }];
  }

  if (tabName === 'themes') {
    return [{ text: `Sample Theme Typography\n${item.name || ''}`, background: item.background }];
  }

  return [{ text: item.title || item.name || item.reference || '' }];
}

function reorderPreviewSlides(fromIdx: number, toIdx: number) {
  if (fromIdx === toIdx || fromIdx < 0 || toIdx < 0) return;
  const sched = currentSnapshot && currentSnapshot.schedule ? currentSnapshot.schedule : null;
  const state = currentSnapshot && currentSnapshot.state ? currentSnapshot.state : null;
  if (!state) return;
  const stagedItem = state.staged_item;
  if (!stagedItem || !stagedItem.slides || fromIdx >= stagedItem.slides.length || toIdx >= stagedItem.slides.length) return;

  // Reordering addresses its target by schedule index, unlike staging/going
  // live — promote a merely-staged (not yet scheduled) item into the
  // schedule first, same as duplicate/delete/duration do.
  const itemIdx: number = ensureItemInSchedule(stagedItem);

  // Optimistically swap slides in local memory
  const [moved] = stagedItem.slides.splice(fromIdx, 1);
  if (!moved) return;
  stagedItem.slides.splice(toIdx, 0, moved);

  // Sync with schedule item slides if present
  if (sched && itemIdx >= 0 && sched.items[itemIdx] && sched.items[itemIdx].slides) {
    const [m2] = sched.items[itemIdx].slides.splice(fromIdx, 1);
    if (m2) sched.items[itemIdx].slides.splice(toIdx, 0, m2);
    lastRenderedScheduleKey = '';
    renderSchedule(sched);
  }

  // Sync live item if it matches
  if (state && state.live_item && state.live_item.id === stagedItem.id && state.live_item.slides) {
    const [mLive] = state.live_item.slides.splice(fromIdx, 1);
    if (mLive) state.live_item.slides.splice(toIdx, 0, mLive);
    if (state.live_slide_index === fromIdx) {
      state.live_slide_index = toIdx;
    } else if (fromIdx < toIdx && state.live_slide_index > fromIdx && state.live_slide_index <= toIdx) {
      state.live_slide_index -= 1;
    } else if (fromIdx > toIdx && state.live_slide_index >= toIdx && state.live_slide_index < fromIdx) {
      state.live_slide_index += 1;
    }
  }

  if (state.staged_slide_index === fromIdx) {
    state.staged_slide_index = toIdx;
  } else if (fromIdx < toIdx && state.staged_slide_index > fromIdx && state.staged_slide_index <= toIdx) {
    state.staged_slide_index -= 1;
  } else if (fromIdx > toIdx && state.staged_slide_index >= toIdx && state.staged_slide_index < fromIdx) {
    state.staged_slide_index += 1;
  }

  sendCommand({ ReorderItemSlides: { item_index: itemIdx, from: fromIdx, to: toIdx } });

  // Defer DOM re-render to the next animation frame so nodes are not destroyed mid-gesture
  requestAnimationFrame(() => {
    lastRenderedPreviewKey = '';
    renderPreviewDeck(state);
    renderPreviewOutput(state);
  });
}

/** Paints a slide's resolved background onto a deck thumbnail, the way the main canvases do. */
function slideBackgroundPainter(item: any, slideIdx: number, state: any): (el: HTMLElement) => void {
  const bg = resolveBackgroundAt(item, slideIdx, availableThemes, state.active_theme, state.global_theme);
  // An animated pattern on a slide in the decks: make sure its pre-rendered clip exists.
  const patternKey = parseAnimatedPatternMarker(bg);
  if (patternKey) ensurePatternClip(patternKey);
  return (el: HTMLElement) => {
    const imgUrl = extractImageUrl(bg, availableThemes);
    if (imgUrl) {
      el.style.backgroundColor = '#000';
      el.style.backgroundImage = `url("${imgUrl}")`;
      el.style.backgroundSize = 'cover';
      el.style.backgroundPosition = 'center';
      el.style.backgroundRepeat = 'no-repeat';
    } else if (isVideoBackground(bg)) {
      el.style.background = '#000';
    } else {
      applyBackgroundThumbnail(el, bg); // a pattern plays its pre-rendered preview, not the live pattern
    }
  };
}

/** Re-draws both slide decks now (the deck view toggles call this: cards vs thumbnails depends on the view). */
/** Shows a deck in the view it should open in for this item (see ui/deck_view_mode.ts) and lights the matching button. */
function applyDeckMode(matrixEl: HTMLElement, deck: DeckName, itemType: string | null | undefined) {
  const mode = deckModeFor(deck, itemType);
  matrixEl.classList.toggle('list-mode', mode === 'list');
  matrixEl.classList.toggle('matrix-mode', mode === 'matrix');
  document.getElementById(`btn-${deck}-view-matrix`)?.classList.toggle('active', mode === 'matrix');
  document.getElementById(`btn-${deck}-view-list`)?.classList.toggle('active', mode === 'list');
  if (deck === 'live') activeLiveViewMode = mode;
}

export function refreshSlideDecks() {
  lastRenderedPreviewKey = '';
  lastRenderedLiveKey = '';
  const state = currentSnapshot && currentSnapshot.state;
  if (!state) return;
  renderPreviewDeck(state);
  renderLiveDeck(state);
}

function renderPreviewDeck(state: ShowState) {
  if (!previewSlideMatrixEl) return;

  if (adhocPreviewItem) {
    if (previewTitleEl) {
      const label = adhocPreviewItem.title || adhocPreviewItem.name || adhocPreviewItem.reference || 'Preview';
      previewTitleEl.textContent = `${label} (Library Preview)`;
    }
    previewDndCleanups.forEach(fn => { try { fn(); } catch (_) {} });
    previewDndCleanups = [];
    previewSlideMatrixEl.innerHTML = '';

    const slides = buildAdhocPreviewSlides(adhocPreviewItem, adhocPreviewTabName);
    if (slides.length === 0) {
      renderDeckEmptyState(previewSlideMatrixEl, {
        undoPlaceholder: null,
        onUndo: () => {},
        emptyTitle: 'Nothing to preview.',
        emptySubtitle: '',
        escapeHtml
      });
    } else {
      slides.forEach((slide, sIdx) => {
        const isActive = adhocPreviewSlideIndex === sIdx;
        const cardEl = buildSlideCardElement({
          badge: `${sIdx + 1}`,
          badgeColor: '',
          lyricsHtml: formatSlideLyricsHtml(slide.text || ''),
          isActive,
          activeClass: 'active-staged',
          showDragHandle: false,
          extraClass: 'preview-slide-card'
        });
        cardEl.addEventListener('click', () => {
          adhocPreviewSlideIndex = sIdx;
          previewSlideMatrixEl.querySelectorAll('.slide-card').forEach((c, idx) => {
            c.classList.toggle('active-staged', idx === sIdx);
          });
          renderPreviewOutput(state);
        });
        previewSlideMatrixEl.appendChild(cardEl);
      });
    }
    // Read-only ad-hoc preview isn't part of the cached-key fast path; force a
    // full re-render the next time real staged-item rendering resumes.
    lastRenderedPreviewKey = '';
    return;
  }

  const stagedItem = state.staged_item;
  if (previewTitleEl) previewTitleEl.textContent = stagedItem ? stagedItem.title : 'No Staged Item';
  showLicensedLink(previewTitleEl, stagedItem);
  applyDeckMode(previewSlideMatrixEl, 'preview', stagedItem ? stagedItem.item_type : '');

  const itemId = stagedItem ? stagedItem.id : '';
  const arr = effectiveArrangement(stagedItem);
  const slideCount = arr ? arr.length : 0;
  const activeIdx = state.staged_slide_index || 0;
  const slideFingerprint = arr ? arr.map((entry, idx) => {
    const s = resolveSlideAt(stagedItem, idx);
    return `${entry.section_id}:${entry.source_slide_index}:${(s?.text || '').slice(0, 20)}`;
  }).join(';') : '';
  const previewStyle = slideDeckStyle(stagedItem ? stagedItem.item_type : '', previewSlideMatrixEl.classList.contains('list-mode'));
  const currentContentKey = `${itemId}|${slideCount}|${slideFingerprint}|${previewStyle}`;

  if (lastRenderedPreviewKey === currentContentKey && slideCount > 0) {
    const cards = previewSlideMatrixEl.querySelectorAll('.slide-card');
    cards.forEach((card, idx) => {
      card.classList.toggle('active-staged', idx === activeIdx);
    });
    return;
  }
  lastRenderedPreviewKey = currentContentKey;

  // Clean up previous Pragmatic DND listeners on preview cards
  previewDndCleanups.forEach(fn => {
    try { fn(); } catch (_) {}
  });
  previewDndCleanups = [];

  previewSlideMatrixEl.innerHTML = '';

  if (!stagedItem || !stagedItem.slides || stagedItem.slides.length === 0 || slideCount === 0) {
    renderDeckEmptyState(previewSlideMatrixEl, {
      undoPlaceholder: activeUndoPlaceholder,
      onUndo: triggerUndoFromPlaceholder,
      emptyTitle: 'No item staged in Preview.',
      emptySubtitle: 'Click an item in Schedule or Library to Preview.',
      escapeHtml
    });
    return;
  }

  arr.forEach((entry, sIdx) => {
    const slide = resolveSlideAt(stagedItem, sIdx) || stagedItem.slides[0];
    const isActive = (state.staged_slide_index || 0) === sIdx;
    const badge = entry.section_id || (slide ? getSlideBadge(slide, entry.source_slide_index) : `S${sIdx + 1}`);
    const tagColor = getSlideBadgeColor(badge);
    const cardEl = previewStyle === 'thumbnails'
      ? buildSlideThumbElement({
          slide,
          theme: resolveThemeAt(stagedItem, availableThemes, state.global_theme),
          badge: escapeHtml(badge),
          badgeColor: tagColor,
          isActive,
          activeClass: 'active-staged',
          showDragHandle: true,
          extraClass: 'preview-slide-card',
          applyBackground: slideBackgroundPainter(stagedItem, sIdx, state)
        })
      : buildSlideCardElement({
          badge: escapeHtml(badge),
          badgeColor: tagColor,
          lyricsHtml: formatSlideLyricsHtml(slide ? slide.text : ''),
          isActive,
          activeClass: 'active-staged',
          showDragHandle: true,
          extraClass: 'preview-slide-card'
        });

    // Preview Card Handle Keyboard Navigation
    const cardHandle = cardEl.querySelector<HTMLElement>('.preview-card-drag-handle');
    if (cardHandle) {
      cardHandle.addEventListener('click', (e) => e.stopPropagation());
      cardHandle.addEventListener('keydown', (e) => {
        if ((e.altKey || e.ctrlKey) && (e.key === 'ArrowLeft' || e.key === 'ArrowUp')) {
          e.preventDefault();
          e.stopPropagation();
          if (sIdx > 0) {
            reorderPreviewSlides(sIdx, sIdx - 1);
          }
        } else if ((e.altKey || e.ctrlKey) && (e.key === 'ArrowRight' || e.key === 'ArrowDown')) {
          e.preventDefault();
          e.stopPropagation();
          if (sIdx < stagedItem.slides.length - 1) {
            reorderPreviewSlides(sIdx, sIdx + 1);
          }
        }
      });
    }

    // Register Pragmatic DnD Draggable on Preview Card (drag from entire card or handle)
    previewDndCleanups.push(
      draggable({
        element: cardEl,
        getInitialData: () => ({ type: 'preview-slide', slideIndex: sIdx }),
        onDragStart: () => cardEl.classList.add('dragging'),
        onDrop: () => cardEl.classList.remove('dragging')
      })
    );

    // Register Pragmatic DnD Drop Target on Preview Card (supports multi-row grid matrix)
    previewDndCleanups.push(
      dropTargetForElements({
        element: cardEl,
        canDrop: ({ source }) => source.data.type === 'preview-slide',
        getData: ({ input, element }) => {
          return attachClosestEdge({ slideIndex: sIdx, type: 'preview-slide' }, {
            input,
            element,
            allowedEdges: ['left', 'right', 'top', 'bottom']
          });
        },
        onDragEnter: ({ self, source }) => {
          const fromIdx = source.data.slideIndex as number;
          if (fromIdx === sIdx) return;
          const edge = extractClosestEdge(self.data);
          cardEl.classList.remove('drop-left', 'drop-right');
          const toIdx = computeSafeDestinationIndex(fromIdx, sIdx, edge, 'horizontal');
          if (toIdx !== fromIdx) {
            if (edge === 'left' || edge === 'top') cardEl.classList.add('drop-left');
            else if (edge === 'right' || edge === 'bottom') cardEl.classList.add('drop-right');
          }
        },
        onDrag: ({ self, source }) => {
          const fromIdx = source.data.slideIndex as number;
          if (fromIdx === sIdx) return;
          const edge = extractClosestEdge(self.data);
          cardEl.classList.remove('drop-left', 'drop-right');
          const toIdx = computeSafeDestinationIndex(fromIdx, sIdx, edge, 'horizontal');
          if (toIdx !== fromIdx) {
            if (edge === 'left' || edge === 'top') cardEl.classList.add('drop-left');
            else if (edge === 'right' || edge === 'bottom') cardEl.classList.add('drop-right');
          }
        },
        onDragLeave: () => {
          cardEl.classList.remove('drop-left', 'drop-right');
        },
        onDrop: ({ self, source }) => {
          cardEl.classList.remove('drop-left', 'drop-right');
          const fromIdx = source.data.slideIndex as number;
          if (fromIdx !== sIdx) {
            const edge = extractClosestEdge(self.data);
            const toIdx = computeSafeDestinationIndex(fromIdx, sIdx, edge, 'horizontal');
            if (fromIdx !== toIdx) {
              reorderPreviewSlides(fromIdx, toIdx);
            }
          }
        }
      })
    );

    // Single click stages slide
    cardEl.addEventListener('click', (e) => {
      if ((e.target as Element | null)?.closest('.preview-card-drag-handle')) return;
      if (cardEl.classList.contains('dragging')) return;
      lastActiveDeckContext = 'preview';
      state.staged_slide_index = sIdx;
      previewSlideMatrixEl.querySelectorAll('.slide-card').forEach((c, idx) => {
        c.classList.toggle('active-staged', idx === sIdx);
      });
      renderPreviewOutput(state);

      const sched = currentSnapshot && currentSnapshot.schedule ? currentSnapshot.schedule : null;
      const itemIdx = (sched && sched.items && stagedItem) ? sched.items.findIndex((it) => it.id === stagedItem.id) : -1;
      setActiveSelectedSlide({
        itemIndex: itemIdx >= 0 ? itemIdx : 0,
        slideIndex: sIdx,
        context: 'preview'
      });
      clearAdhocPreview();
      sendCommand({ StageItem: { item_index: itemIdx >= 0 ? itemIdx : null, slide_index: sIdx } });
    });

    // Double click sends slide directly to Live
    cardEl.addEventListener('dblclick', (e) => {
      if ((e.target as Element | null)?.closest('.preview-card-drag-handle')) return;
      if (cardEl.classList.contains('dragging')) return;
      const sched = currentSnapshot && currentSnapshot.schedule ? currentSnapshot.schedule : null;
      const itemIdx = (sched && sched.items && stagedItem) ? sched.items.findIndex((it) => it.id === stagedItem.id) : -1;
      sendCommand({ GoLive: { item_index: itemIdx >= 0 ? itemIdx : null, slide_index: sIdx } });
    });

    // Context Menu on Preview Slide
    cardEl.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      lastActiveDeckContext = 'preview';
      const sched = currentSnapshot && currentSnapshot.schedule ? currentSnapshot.schedule : null;
      const itemIdx = (sched && sched.items && stagedItem) ? sched.items.findIndex((it) => it.id === stagedItem.id) : -1;
      setActiveSelectedSlide({
        itemIndex: itemIdx >= 0 ? itemIdx : 0,
        slideIndex: sIdx,
        context: 'preview'
      });
      currentContextMenuTarget = { type: 'slide', itemIndex: itemIdx >= 0 ? itemIdx : null, slideIndex: sIdx, item: stagedItem, slide: slide };
      updateSlideDurationMenuVisibility(stagedItem);
      showContextMenu(document.getElementById('slide-context-menu'), e.clientX, e.clientY);
    });

    previewSlideMatrixEl.appendChild(cardEl);
  });
}

function renderPreviewOutput(state: ShowState) {
  if (!previewCanvasLyricsEl) return;

  if (adhocPreviewItem) {
    // This branch writes the same DOM subtree the staged-item fast path below guards with
    // lastRenderedPreviewOutputKey; invalidate it so returning to the staged item afterward
    // doesn't wrongly skip a re-render because its key looks unchanged from before adhoc ran.
    lastRenderedPreviewOutputKey = '';
    const slides = buildAdhocPreviewSlides(adhocPreviewItem, adhocPreviewTabName);
    const currentSlide = slides[adhocPreviewSlideIndex] || slides[0];

    if (!currentSlide) {
      previewCanvasLyricsEl.textContent = '';
      if (previewCanvasFooterLeftEl) previewCanvasFooterLeftEl.textContent = '';
      if (previewCanvasFooterRightEl) previewCanvasFooterRightEl.textContent = '';
      if (previewSlideCounterTextEl) previewSlideCounterTextEl.textContent = 'Preview';
      if (previewCanvasImageEl) previewCanvasImageEl.style.display = 'none';
      if (previewCanvasEl) { previewCanvasEl.style.animation = ''; previewCanvasEl.style.background = 'linear-gradient(135deg, #181920, #22232c, #2a2228)'; }
      return;
    }

    const currentBg = currentSlide.background || 'linear-gradient(135deg, #0f2027, #203a43, #2c5364)';
    const imgUrl = extractImageUrl(currentBg, availableThemes);
    if (imgUrl || isVideoBackground(currentBg)) {
      if (previewCanvasImageEl) {
        if (imgUrl) {
          previewCanvasImageEl.style.backgroundImage = `url("${imgUrl}")`;
          previewCanvasImageEl.style.backgroundSize = 'cover';
          previewCanvasImageEl.style.backgroundPosition = 'center';
          previewCanvasImageEl.style.backgroundRepeat = 'no-repeat';
          previewCanvasImageEl.style.display = 'block';
        } else {
          previewCanvasImageEl.style.display = 'none';
        }
      }
      if (previewCanvasEl) { previewCanvasEl.style.animation = ''; previewCanvasEl.style.background = '#000'; }
    } else {
      if (previewCanvasImageEl) previewCanvasImageEl.style.display = 'none';
      if (previewCanvasEl) applyResolvedBackground(previewCanvasEl, currentBg);
    }

    const textContent = currentSlide.text || '';
    applyAutoFitLyrics(previewCanvasLyricsEl, formatSlideLyricsHtml(textContent));

    const label = adhocPreviewItem.title || adhocPreviewItem.name || adhocPreviewItem.reference || '';
    if (previewCanvasFooterLeftEl) previewCanvasFooterLeftEl.textContent = label;
    if (previewCanvasFooterRightEl) previewCanvasFooterRightEl.textContent = '';
    if (previewSlideCounterTextEl) {
      previewSlideCounterTextEl.textContent = `Slide ${Math.min(adhocPreviewSlideIndex + 1, slides.length)} of ${slides.length} (Preview Only)`;
    }
    return;
  }

  const stagedItem = state.staged_item;
  const slideIdx = state.staged_slide_index || 0;

  if (!stagedItem || !stagedItem.slides || stagedItem.slides.length === 0) {
    lastRenderedPreviewOutputKey = '';
    previewCanvasLyricsEl.textContent = '';
    previewCanvasLyricsEl.style.opacity = '1';
    if (previewCanvasElementsEl) { previewCanvasElementsEl.style.display = 'none'; previewCanvasElementsEl.innerHTML = ''; }
    if (previewCanvasFooterLeftEl) previewCanvasFooterLeftEl.textContent = '';
    if (previewCanvasFooterRightEl) previewCanvasFooterRightEl.textContent = '';
    if (previewSlideCounterTextEl) previewSlideCounterTextEl.textContent = 'Preview';
    if (previewCanvasImageEl) previewCanvasImageEl.style.display = 'none';
    if (previewCanvasEl) { previewCanvasEl.style.animation = ''; previewCanvasEl.style.background = 'linear-gradient(135deg, #181920, #22232c, #2a2228)'; }
    return;
  }

  const stagedArr = effectiveArrangement(stagedItem);
  const currentSlide = resolveSlideAt(stagedItem, slideIdx) || stagedItem.slides[0];
  let currentBg = resolveBackgroundAt(stagedItem, slideIdx, availableThemes, state.active_theme, state.global_theme);

  // Cheap O(1) short-circuit: most WS messages (blackout, live nav, alerts, media transport)
  // don't touch the staged item at all, but this function used to unconditionally rewrite
  // DOM styles + innerHTML (a layout/paint cost, pricier than the JS itself) on every one.
  // Same technique already used by renderLiveDeck's lastRenderedLiveKey below.
  const previewOutputKey = `${stagedItem.id}|${slideIdx}|${stagedArr.length}|${currentBg}|${currentSlide ? currentSlide.text : ''}|${currentSlide ? currentSlide.footer : ''}|${stagedItem.title || ''}`;
  if (lastRenderedPreviewOutputKey === previewOutputKey) {
    return;
  }
  lastRenderedPreviewOutputKey = previewOutputKey;

  const imgUrl = extractImageUrl(currentBg, availableThemes);
  if (imgUrl || isVideoBackground(currentBg)) {
    if (previewCanvasImageEl) {
      if (imgUrl) {
        previewCanvasImageEl.style.backgroundImage = `url("${imgUrl}")`;
        previewCanvasImageEl.style.backgroundSize = 'cover';
        previewCanvasImageEl.style.backgroundPosition = 'center';
        previewCanvasImageEl.style.backgroundRepeat = 'no-repeat';
        previewCanvasImageEl.style.display = 'block';
      } else {
        previewCanvasImageEl.style.display = 'none';
      }
    }
    if (previewCanvasEl) { previewCanvasEl.style.animation = ''; previewCanvasEl.style.background = '#000'; }
  } else {
    if (previewCanvasImageEl) previewCanvasImageEl.style.display = 'none';
    if (previewCanvasEl) applyResolvedBackground(previewCanvasEl, currentBg);
  }

  if (previewCanvasEl) {
    const previewTheme = resolveThemeAt(stagedItem, availableThemes, state.global_theme);
    renderSlideVisual(previewCanvasEl, previewCanvasElementsEl, previewCanvasLyricsEl, currentSlide, previewTheme);
  }

  if (previewCanvasFooterLeftEl) previewCanvasFooterLeftEl.textContent = stagedItem.title || '';
  if (previewCanvasFooterRightEl) previewCanvasFooterRightEl.textContent = (currentSlide && currentSlide.footer) || '';
  if (previewSlideCounterTextEl) {
    previewSlideCounterTextEl.textContent = `Slide ${Math.min(slideIdx + 1, stagedArr.length)} of ${stagedArr.length}`;
  }
}

let lastRenderedLiveKey = '';

function renderLiveDeck(state: ShowState) {
  if (!liveSlideMatrixEl) return;
  if (liveTitleEl) liveTitleEl.textContent = state.live_item ? state.live_item.title : 'No Active Item';
  showLicensedLink(liveTitleEl, state.live_item);

  const liveItem = state.live_item;
  applyDeckMode(liveSlideMatrixEl, 'live', liveItem ? liveItem.item_type : '');
  const itemId = liveItem ? liveItem.id : '';
  const arr = effectiveArrangement(liveItem);
  const slideCount = arr ? arr.length : 0;
  const activeIdx = state.live_slide_index || 0;
  const slideFingerprint = arr ? arr.map((entry, idx) => {
    const s = resolveSlideAt(liveItem, idx);
    return `${entry.section_id}:${entry.source_slide_index}:${(s?.text || '').slice(0, 20)}`;
  }).join(';') : '';
  const liveStyle = slideDeckStyle(liveItem ? liveItem.item_type : '', liveSlideMatrixEl.classList.contains('list-mode'));
  const currentLiveKey = `${itemId}|${slideCount}|${slideFingerprint}|${liveStyle}`;

  if (lastRenderedLiveKey === currentLiveKey && slideCount > 0) {
    const cards = liveSlideMatrixEl.querySelectorAll('.slide-card');
    cards.forEach((card, idx) => {
      card.classList.toggle('active', idx === activeIdx);
    });
    return;
  }
  lastRenderedLiveKey = currentLiveKey;

  liveSlideMatrixEl.innerHTML = '';

  if (!liveItem || !liveItem.slides || liveItem.slides.length === 0 || slideCount === 0) {
    renderDeckEmptyState(liveSlideMatrixEl, {
      undoPlaceholder: activeUndoPlaceholder,
      onUndo: triggerUndoFromPlaceholder,
      emptyTitle: 'No active live presentation.',
      emptySubtitle: 'Double-click an item in Schedule or Library to Go Live.',
      escapeHtml
    });
    return;
  }

  arr.forEach((entry, sIdx) => {
    const slide = resolveSlideAt(liveItem, sIdx) || liveItem.slides[0];
    const isActive = state.live_slide_index === sIdx;
    const badge = entry.section_id || (slide ? getSlideBadge(slide, entry.source_slide_index) : `S${sIdx + 1}`);
    const tagColor = getSlideBadgeColor(badge);
    const cardEl = liveStyle === 'thumbnails'
      ? buildSlideThumbElement({
          slide,
          theme: resolveThemeAt(liveItem, availableThemes, state.global_theme),
          badge: escapeHtml(badge),
          badgeColor: tagColor,
          isActive,
          activeClass: 'active',
          showDragHandle: false,
          applyBackground: slideBackgroundPainter(liveItem, sIdx, state)
        })
      : buildSlideCardElement({
          badge: escapeHtml(badge),
          badgeColor: tagColor,
          lyricsHtml: formatSlideLyricsHtml(slide ? slide.text : ''),
          isActive,
          activeClass: 'active',
          showDragHandle: false
        });

    cardEl.addEventListener('click', () => {
      lastActiveDeckContext = 'live';
      const sched = currentSnapshot && currentSnapshot.schedule ? currentSnapshot.schedule : null;
      const itemIdx = (sched && sched.items && liveItem) ? sched.items.findIndex((it) => it.id === liveItem.id) : -1;
      setActiveSelectedSlide({ itemIndex: itemIdx >= 0 ? itemIdx : 0, slideIndex: sIdx, context: 'live' });
      sendCommand({ JumpSlide: sIdx });
    });

    // Right-Click Context Menu on Live Matrix Slide
    cardEl.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      lastActiveDeckContext = 'live';
      const sched = currentSnapshot && currentSnapshot.schedule ? currentSnapshot.schedule : null;
      const itemIdx = (sched && sched.items && liveItem) ? sched.items.findIndex((it) => it.id === liveItem.id) : -1;
      setActiveSelectedSlide({ itemIndex: itemIdx >= 0 ? itemIdx : 0, slideIndex: sIdx, context: 'live' });
      currentContextMenuTarget = { type: 'slide', itemIndex: itemIdx >= 0 ? itemIdx : null, slideIndex: sIdx, item: liveItem, slide: slide };
      updateSlideDurationMenuVisibility(liveItem);
      showContextMenu(document.getElementById('slide-context-menu'), e.clientX, e.clientY);
    });

    liveSlideMatrixEl.appendChild(cardEl);
  });
}

function renderLiveOutput(state: ShowState) {
  if (!canvasLyricsEl) return;
  const liveItem = state.live_item;
  const slideIdx = state.live_slide_index || 0;
  // The ribbon's Live indicator pulses only while something is actually on the screens.
  btnLiveStatus?.classList.toggle('active', !!liveItem);

  const presentationControlsEl = document.getElementById('live-presentation-controls');
  if (presentationControlsEl) {
    presentationControlsEl.style.display = (liveItem && liveItem.item_type === 'presentation') ? 'flex' : 'none';
  }
  if (isPresentationPlaying() && (!liveItem || liveItem.id !== getPresentationPlaybackItemId())) {
    stopPresentationPlayback();
  }
  if (liveItem && liveItem.item_type === 'presentation') {
    updatePresentationControlsUI();
  }

  if (canvasBlackoutEl) {
    if (state.is_blackout) {
      canvasBlackoutEl.style.display = 'block';
      if (canvasLogoEl) canvasLogoEl.style.display = 'none';
    } else if (state.is_logo_override) {
      canvasBlackoutEl.style.display = 'none';
      if (canvasLogoEl) canvasLogoEl.style.display = 'flex';
    } else {
      canvasBlackoutEl.style.display = 'none';
      if (canvasLogoEl) canvasLogoEl.style.display = 'none';
    }
  }

  if (canvasNurseryAlertEl) {
    if (state.alert_message) {
      canvasNurseryAlertEl.style.display = 'block';
      if (canvasNurseryTextEl) canvasNurseryTextEl.textContent = `🔔 ${state.alert_message}`;
    } else {
      canvasNurseryAlertEl.style.display = 'none';
    }
  }

  if (!liveItem || !liveItem.slides || liveItem.slides.length === 0) {
    if (canvasLyricsEl) canvasLyricsEl.textContent = '';
    if (canvasFooterLeftEl) canvasFooterLeftEl.textContent = appOptions.churchName;
    if (canvasFooterRightEl) canvasFooterRightEl.textContent = '';
    if (slideCounterTextEl) slideCounterTextEl.textContent = 'Slide 0 of 0';
    return;
  }

  const liveArr = effectiveArrangement(liveItem);
  const currentSlide = resolveSlideAt(liveItem, slideIdx) || liveItem.slides[0];
  let currentBg = resolveBackgroundAt(liveItem, slideIdx, availableThemes, state.active_theme, state.global_theme);

  if (isVideoBackground(currentBg)) {
    const isDedicatedMedia = liveItem && liveItem.item_type === 'media';
    if (liveMediaControls) liveMediaControls.style.display = 'flex';
    if (liveVideoEl) {
      liveVideoEl.muted = liveAudioMuted;
      liveVideoEl.volume = liveAudioVolume;
      liveVideoEl.loop = isLiveVideoLooping;
      liveVideoEl.style.objectFit = isDedicatedMedia ? 'contain' : 'cover';
      if (liveVideoEl.dataset.src !== currentBg) {
        liveVideoEl.src = currentBg;
        liveVideoEl.dataset.src = currentBg;
        if (canvasBlackoutEl) canvasBlackoutEl.classList.remove('fading-black');
      }
      liveVideoEl.style.display = 'block';

      if (state.media_playback) {
        if (state.media_playback.is_looping !== undefined) {
          isLiveVideoLooping = state.media_playback.is_looping;
          if (liveVideoEl) liveVideoEl.loop = isLiveVideoLooping;
          if (btnLiveVideoLoop) {
            btnLiveVideoLoop.textContent = isLiveVideoLooping ? '🔁 Loop: ON' : '🔁 Loop: OFF';
            btnLiveVideoLoop.style.color = isLiveVideoLooping ? 'var(--os-success-text)' : '';
          }
        }
        if (isUserSeeking) return;
        if (state.media_playback.is_playing) {
          if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '⏸ Pause';
          const now = Date.now();
          const startAnchor = state.media_playback.start_at_epoch_ms || state.media_playback.timestamp_ms;

          // If scheduled for future unpause, hold frame and arm timer
          if (state.media_playback.start_at_epoch_ms && now < state.media_playback.start_at_epoch_ms) {
            if (!liveVideoEl.paused) liveVideoEl.pause();
            if (Math.abs(liveVideoEl.currentTime - state.media_playback.current_time) > 0.04) {
              try { liveVideoEl.currentTime = state.media_playback.current_time; } catch (_) {}
            }
            armClientScheduledPlay(liveVideoEl, state.media_playback.start_at_epoch_ms);
            return;
          }

          const elapsed = Math.max(0, (now - startAnchor) / 1000);
          const targetTime = state.media_playback.current_time + elapsed;

          const applyLiveVideoSync = () => {
            if (!liveVideoEl || liveVideoEl.seeking) return;
            // Skip repositioning during 500ms grace window after gate open to prevent first-frame stutter
            const inGrace = _appPlayStartedAt && (performance.now() - _appPlayStartedAt) < 500;
            if (!inGrace && Math.abs(liveVideoEl.currentTime - targetTime) > 0.25) {
              try { liveVideoEl.currentTime = targetTime; } catch (_) {}
            }
            if (liveVideoEl.paused) {
              liveVideoEl.play().catch((err) => {
                console.warn('Live video autoplay unmuted blocked by browser policy:', err);
                liveVideoEl.muted = true;
                liveAudioMuted = true;
                if (btnLiveVideoMute) btnLiveVideoMute.textContent = '🔇';
                liveVideoEl.play().catch(() => {});
              });
            }
          };

          if (liveVideoEl.readyState < 2) {
            liveVideoEl.addEventListener('canplay', applyLiveVideoSync, { once: true });
          } else {
            applyLiveVideoSync();
          }
        } else {
          if (btnLiveVideoPlay) btnLiveVideoPlay.textContent = '▶ Play';
          if (liveVideoEl && !liveVideoEl.paused) {
            liveVideoEl.pause();
          }
          if (liveVideoEl && Math.abs(liveVideoEl.currentTime - state.media_playback.current_time) > 0.04) {
            try { liveVideoEl.currentTime = state.media_playback.current_time; } catch (_) {}
          }
        }
      }
    }
    if (liveImageEl) liveImageEl.style.display = 'none';
    if (liveCanvasEl) { liveCanvasEl.style.animation = ''; liveCanvasEl.style.background = '#000'; }
  } else if (isAudioMedia(currentBg)) {
    if (liveMediaControls) liveMediaControls.style.display = 'flex';
    if (liveVideoEl) {
      liveVideoEl.style.display = 'none';
      if (!liveVideoEl.paused) liveVideoEl.pause();
      if (liveVideoEl.dataset.src) {
        liveVideoEl.dataset.src = '';
        liveVideoEl.removeAttribute('src');
        liveVideoEl.load();
      }
    }
    if (liveImageEl) liveImageEl.style.display = 'none';
    if (liveCanvasEl) { liveCanvasEl.style.animation = ''; liveCanvasEl.style.background = 'linear-gradient(135deg, #102027, #37474f)'; }
    if (liveAudioEl) {
      liveAudioEl.muted = liveAudioMuted;
      liveAudioEl.volume = liveAudioVolume;
      if (liveAudioEl.dataset.src !== currentBg) {
        liveAudioEl.src = currentBg;
        liveAudioEl.dataset.src = currentBg;
        liveAudioEl.play().catch(() => {});
      }
    }
  } else if (extractImageUrl(currentBg, availableThemes)) {
    const liveImgUrl = extractImageUrl(currentBg, availableThemes);
    if (liveMediaControls) liveMediaControls.style.display = 'none';
    if (liveVideoEl) {
      liveVideoEl.style.display = 'none';
      if (!liveVideoEl.paused) liveVideoEl.pause();
      if (liveVideoEl.dataset.src) {
        liveVideoEl.dataset.src = '';
        liveVideoEl.removeAttribute('src');
        liveVideoEl.load();
      }
    }
    if (liveAudioEl) {
      liveAudioEl.pause();
      liveAudioEl.dataset.src = '';
    }
    if (liveImageEl) {
      liveImageEl.style.backgroundImage = `url("${liveImgUrl}")`;
      liveImageEl.style.backgroundSize = 'cover';
      liveImageEl.style.backgroundPosition = 'center';
      liveImageEl.style.backgroundRepeat = 'no-repeat';
      liveImageEl.style.display = 'block';
    }
    if (liveCanvasEl) { liveCanvasEl.style.animation = ''; liveCanvasEl.style.background = '#000'; }
  } else {
    if (liveMediaControls) liveMediaControls.style.display = 'none';
    if (liveVideoEl) {
      liveVideoEl.style.display = 'none';
      if (!liveVideoEl.paused) liveVideoEl.pause();
      if (liveVideoEl.dataset.src) {
        liveVideoEl.dataset.src = '';
        liveVideoEl.removeAttribute('src');
        liveVideoEl.load();
      }
    }
    if (liveImageEl) liveImageEl.style.display = 'none';
    if (liveAudioEl) {
      liveAudioEl.pause();
      liveAudioEl.dataset.src = '';
    }
    if (liveCanvasEl) applyResolvedBackground(liveCanvasEl, currentBg);
  }

  if (state.is_clear_text) {
    canvasLyricsEl.style.opacity = '0';
    if (canvasFooterLeftEl) canvasFooterLeftEl.style.opacity = '0';
    if (canvasFooterRightEl) canvasFooterRightEl.style.opacity = '0';
    if (liveCanvasElementsEl) liveCanvasElementsEl.style.display = 'none';
  } else {
    if (canvasFooterLeftEl) {
      canvasFooterLeftEl.style.opacity = '1';
      canvasFooterLeftEl.textContent = liveItem.title || '';
    }
    if (canvasFooterRightEl) {
      canvasFooterRightEl.style.opacity = '1';
      canvasFooterRightEl.textContent = currentSlide.footer || liveItem.subtitle || '';
    }

    if (liveCanvasEl) {
      const liveTheme = resolveThemeAt(liveItem, availableThemes, state.global_theme);
      renderSlideVisual(liveCanvasEl, liveCanvasElementsEl, canvasLyricsEl, currentSlide, liveTheme);
    }
  }

  if (slideCounterTextEl) slideCounterTextEl.textContent = `Slide ${Math.min(slideIdx + 1, liveArr.length)} of ${liveArr.length}`;
}

// Right-Click Context Menu on Live Monitor Canvas
if (liveCanvasEl) {
  liveCanvasEl.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    e.stopPropagation();
    showContextMenu(document.getElementById('live-monitor-context-menu'), e.clientX, e.clientY);
  });
}

// ============================================================================
// CONTEXT MENUS ACTIONS DISPATCHER
// ============================================================================
// 1. Schedule Item Context Menu Handlers
on('ctx-sched-golive', 'click', () => {
  if (currentContextMenuTarget) sendCommand({ GoLive: { item_index: currentContextMenuTarget.index, slide_index: 0 } });
  hideAllContextMenus();
});
on('ctx-sched-stage', 'click', () => {
  if (currentContextMenuTarget) {
    clearAdhocPreview();
    sendCommand({ StageItem: { item_index: currentContextMenuTarget.index, slide_index: 0 } });
  }
  hideAllContextMenus();
});
on('ctx-sched-edit', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    uiCallbacks.editExistingItem?.(currentContextMenuTarget.item, { itemIndex: currentContextMenuTarget.index });
  }
  hideAllContextMenus();
});
on('ctx-sched-arrangement', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    openArrangementModal(currentContextMenuTarget.index, currentContextMenuTarget.item);
  }
  hideAllContextMenus();
});
on('ctx-sched-theme', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const item = currentContextMenuTarget.item;
    const itemIdx = currentContextMenuTarget.index;
    openThemePicker(item, (theme) => {
      item.theme_name = theme.name;
      if (item.slides) {
        item.slides.forEach((s: LibrarySlide) => s.background = theme.bg);
      }
      sendCommand({
        SetItemTheme: {
          item_index: itemIdx,
          theme_name: theme.name,
          background: theme.bg
        }
      });
      lastRenderedScheduleKey = '';
      if (currentSnapshot && currentSnapshot.schedule) renderSchedule(currentSnapshot.schedule);
      if (currentSnapshot && currentSnapshot.state) {
        renderLiveDeck(currentSnapshot.state);
        renderLiveOutput(currentSnapshot.state);
      }
    });
  }
  hideAllContextMenus();
});
on('ctx-sched-image', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const item = currentContextMenuTarget.item;
    const itemIdx = currentContextMenuTarget.index;
    openMediaImagePicker({
      currentImage: item.slides && item.slides[0] ? item.slides[0].background : null,
      onSelectSlide: (_filePath, cssBg) => {
        item.theme_name = name;
        if (item.slides) item.slides.forEach((s: LibrarySlide) => s.background = cssBg);
        sendCommand({
          SetItemTheme: {
            item_index: itemIdx,
            theme_name: name,
            background: cssBg
          }
        });
        lastRenderedScheduleKey = '';
        if (currentSnapshot && currentSnapshot.schedule) renderSchedule(currentSnapshot.schedule);
        if (currentSnapshot && currentSnapshot.state) {
          renderLiveDeck(currentSnapshot.state);
          renderLiveOutput(currentSnapshot.state);
        }
      }
    });
  }
  hideAllContextMenus();
});
on('ctx-sched-add-group', 'click', () => {
  let insertIdx = null;
  if (currentContextMenuTarget && typeof currentContextMenuTarget.index === 'number') {
    insertIdx = currentContextMenuTarget.index + 1;
  }
  uiCallbacks.promptNewSectionHeader?.(insertIdx);
  hideAllContextMenus();
});
on('ctx-sched-empty-add-group', 'click', () => {
  uiCallbacks.promptNewSectionHeader?.(null);
  hideAllContextMenus();
});
on('ctx-sched-empty-new-song', 'click', () => {
  uiCallbacks.openSlideEditor?.('song');
  hideAllContextMenus();
});
on('ctx-sched-empty-new-presentation', 'click', () => {
  uiCallbacks.openSlideEditor?.('presentation');
  hideAllContextMenus();
});
on('ctx-sched-empty-new-scripture', 'click', () => {
  uiCallbacks.openSlideEditor?.('scripture');
  hideAllContextMenus();
});
on('ctx-sched-empty-open', 'click', () => {
  resetOpenModal();
  showModal(openModal);
  hideAllContextMenus();
});
on('ctx-sched-empty-clear', 'click', () => {
  sendCommand('NewSchedule');
  showToast('✓ Cleared schedule', 'info');
  hideAllContextMenus();
});
on('ctx-sched-remove', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.index !== undefined && currentContextMenuTarget.index !== null) {
    deleteScheduleItemByIndex(currentContextMenuTarget.index, currentContextMenuTarget.item?.title);
  }
  hideAllContextMenus();
});
on('ctx-sched-merge-prev', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.index > 0) {
    sendCommand({ MergeScheduleItems: { first_item_index: currentContextMenuTarget.index - 1, second_item_index: currentContextMenuTarget.index } });
  }
  hideAllContextMenus();
});
on('ctx-sched-merge-next', 'click', () => {
  if (currentContextMenuTarget) {
    sendCommand({ MergeScheduleItems: { first_item_index: currentContextMenuTarget.index, second_item_index: currentContextMenuTarget.index + 1 } });
  }
  hideAllContextMenus();
});

/**
 * Resolves an item's index in the live schedule, promoting it into the
 * schedule first if it's only staged (e.g. a Library Preview click, staged
 * via the real backend since it now shares the same code path as scheduling
 * — see StageItemDirect) — needed because slide-level mutations
 * (duplicate/delete/reorder/duration) address their target by schedule
 * index, unlike staging or going live, which can operate on a bare staged
 * item with no schedule membership at all. AddToSchedule always appends at
 * schedule.items.length, so that length (captured before sending it) is the
 * item's post-add index; optimistically mirroring the push into
 * currentSnapshot.schedule.items lets the caller's very next sendCommand
 * target that index immediately, without waiting on the round trip.
 */
function ensureItemInSchedule(item: any): number {
  if (!currentSnapshot || !currentSnapshot.schedule || !item) return -1;
  const existingIdx = currentSnapshot.schedule.items.findIndex((it) => it.id === item.id);
  if (existingIdx >= 0) return existingIdx;
  const newIdx = currentSnapshot.schedule.items.length;
  // A deep clone, not the same reference as `item` (state.staged_item/
  // live_item are denormalized copies, not references into schedule.items —
  // pushing the same object would make later independent edits to each
  // double-apply to one shared array).
  currentSnapshot.schedule.items.push(JSON.parse(JSON.stringify(item)));
  sendCommand({ AddToSchedule: item });
  return newIdx;
}

// 2. Slide Context Menu Handlers
on('ctx-slide-golive', 'click', () => {
  if (currentContextMenuTarget) {
    if (currentContextMenuTarget.itemIndex !== null && currentContextMenuTarget.itemIndex !== undefined) {
      sendCommand({ GoLive: { item_index: currentContextMenuTarget.itemIndex, slide_index: currentContextMenuTarget.slideIndex } });
    } else if (currentContextMenuTarget.slideIndex !== null && currentContextMenuTarget.slideIndex !== undefined) {
      sendCommand({ JumpSlide: currentContextMenuTarget.slideIndex });
    }
  }
  hideAllContextMenus();
});
on('ctx-slide-stage', 'click', () => {
  if (currentContextMenuTarget) {
    clearAdhocPreview();
    sendCommand({ StageItem: { item_index: currentContextMenuTarget.itemIndex, slide_index: currentContextMenuTarget.slideIndex } });
  }
  hideAllContextMenus();
});
on('ctx-slide-edit', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const hasItemIndex = currentContextMenuTarget.itemIndex !== null && currentContextMenuTarget.itemIndex !== undefined;
    uiCallbacks.editExistingItem?.(
      currentContextMenuTarget.item,
      hasItemIndex ? { itemIndex: currentContextMenuTarget.itemIndex, slideIndex: currentContextMenuTarget.slideIndex } : null
    );
  }
  hideAllContextMenus();
});
on('ctx-slide-bg', 'click', () => {
  if (currentContextMenuTarget) {
    const target = currentContextMenuTarget;
    const item = target.item;
    const itemIdx = target.itemIndex;
    const slideIdx = target.slideIndex;
    openThemePicker(item, (theme) => {
      if (target.slide) {
        target.slide.background = theme.bg;
      }
      sendCommand({
        SetSlideBackground: {
          item_index: (itemIdx !== undefined && itemIdx !== null) ? itemIdx : null,
          slide_index: (slideIdx !== undefined && slideIdx !== null) ? slideIdx : 0,
          background: theme.bg
        }
      });
      lastRenderedScheduleKey = '';
      if (currentSnapshot && currentSnapshot.schedule) renderSchedule(currentSnapshot.schedule);
      if (currentSnapshot && currentSnapshot.state) {
        renderLiveDeck(currentSnapshot.state);
        renderLiveOutput(currentSnapshot.state);
      }
    });
  }
  hideAllContextMenus();
});
on('ctx-slide-image', 'click', () => {
  if (currentContextMenuTarget) {
    const target = currentContextMenuTarget;
    const itemIdx = target.itemIndex;
    const slideIdx = target.slideIndex;
    openMediaImagePicker({
      currentImage: target.slide ? target.slide.background : null,
      onSelectSlide: (_filePath, cssBg) => {
        if (target.slide) target.slide.background = cssBg;
        sendCommand({
          SetSlideBackground: {
            item_index: (itemIdx !== undefined && itemIdx !== null) ? itemIdx : null,
            slide_index: (slideIdx !== undefined && slideIdx !== null) ? slideIdx : 0,
            background: cssBg
          }
        });
        lastRenderedScheduleKey = '';
        if (currentSnapshot && currentSnapshot.schedule) renderSchedule(currentSnapshot.schedule);
        if (currentSnapshot && currentSnapshot.state) {
          renderLiveDeck(currentSnapshot.state);
          renderLiveOutput(currentSnapshot.state);
        }
      }
    });
  }
  hideAllContextMenus();
});
on('ctx-slide-duplicate', 'click', () => {
  if (currentContextMenuTarget) {
    const target = currentContextMenuTarget;
    let itemIdx = target.itemIndex;
    const slideIdx = target.slideIndex;
    if ((itemIdx === null || itemIdx === undefined) && target.item) {
      itemIdx = ensureItemInSchedule(target.item);
    }
    if (itemIdx !== null && itemIdx !== undefined && itemIdx >= 0 && slideIdx !== undefined && slideIdx !== null) {
      sendCommand({ DuplicateItemSlide: { item_index: itemIdx, slide_index: slideIdx } });
      showToast('✓ Duplicated slide', 'success');
    }
  }
  hideAllContextMenus();
});
on('ctx-slide-duration', 'click', () => {
  if (currentContextMenuTarget) {
    const target = currentContextMenuTarget;
    let itemIdx = target.itemIndex;
    const slideIdx = target.slideIndex;
    if ((itemIdx === null || itemIdx === undefined) && target.item) {
      itemIdx = ensureItemInSchedule(target.item);
    }
    if (itemIdx !== null && itemIdx !== undefined && itemIdx >= 0 && slideIdx !== undefined && slideIdx !== null) {
      const current = target.slide && target.slide.duration_seconds != null ? String(target.slide.duration_seconds) : '';
      const input = prompt('Slide duration in seconds (blank = use the presentation\'s default):', current);
      if (input !== null) {
        const trimmed = input.trim();
        const seconds = trimmed === '' ? null : parseFloat(trimmed);
        if (seconds === null || (!isNaN(seconds) && seconds > 0)) {
          sendCommand({ SetSlideDuration: { item_index: itemIdx, slide_index: slideIdx, duration_seconds: seconds } });
          showToast(seconds === null ? '✓ Cleared slide duration override' : `✓ Slide duration set to ${seconds}s`, 'success');
        } else {
          showToast('Enter a positive number of seconds, or leave blank to clear', 'warning');
        }
      }
    }
  }
  hideAllContextMenus();
});
on('ctx-slide-delete', 'click', () => {
  if (currentContextMenuTarget) {
    const target = currentContextMenuTarget;
    let itemIdx = target.itemIndex;
    const slideIdx = target.slideIndex;
    if ((itemIdx === null || itemIdx === undefined) && target.item) {
      itemIdx = ensureItemInSchedule(target.item);
    }
    if (itemIdx !== null && itemIdx !== undefined && itemIdx >= 0 && slideIdx !== undefined && slideIdx !== null) {
      if (window.confirm(`Are you sure you want to delete Slide ${(slideIdx || 0) + 1}?`)) {
        sendCommand({ RemoveItemSlide: { item_index: itemIdx, slide_index: slideIdx } });
        showToast('✓ Removed slide', 'info');
      }
    }
  }
  hideAllContextMenus();
});
on('ctx-slide-copy', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.slide) {
    copyToClipboard(currentContextMenuTarget.slide.text || '', '✓ Copied slide text to clipboard!');
  }
  hideAllContextMenus();
});

// 3. Resource Library Context Menu Handlers
on('ctx-lib-add-sched', 'click', () => {
  if (currentContextMenuTarget) {
    if (currentContextMenuTarget.item) {
      if (currentContextMenuTarget.tab === 'scriptures' && isDualBibleMode) {
        const item = currentContextMenuTarget.item;
        const pVersion = (item.version || activeBibleVersion || 'Primary').toUpperCase();
        addDualScriptureToSchedule(item, pVersion, secondaryBibleVersion, item.verses, currentDualSecondaryVerses);
      } else {
        addItemToSchedule(currentContextMenuTarget.tab, currentContextMenuTarget.item.id);
      }
    }
  }
  hideAllContextMenus();
});
on('ctx-lib-add-dual', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const item = currentContextMenuTarget.item;
    const pVersion = (item.version || activeBibleVersion || 'Primary').toUpperCase();
    addDualScriptureToSchedule(item, pVersion, secondaryBibleVersion, item.verses, currentDualSecondaryVerses);
  }
  hideAllContextMenus();
});
on('ctx-lib-golive', 'click', () => {
  if (currentContextMenuTarget) {
    if (currentContextMenuTarget.item) {
      if (currentContextMenuTarget.tab === 'scriptures' && isDualBibleMode) {
        const item = currentContextMenuTarget.item;
        const pVersion = (item.version || activeBibleVersion || 'Primary').toUpperCase();
        addDualScriptureToSchedule(item, pVersion, secondaryBibleVersion, item.verses, currentDualSecondaryVerses);
      } else {
        addItemToSchedule(currentContextMenuTarget.tab, currentContextMenuTarget.item.id);
      }
      setTimeout(() => sendCommand({ GoLive: { item_index: null, slide_index: 0 } }), 100);
    }
  }
  hideAllContextMenus();
});
on('ctx-lib-edit', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    uiCallbacks.editExistingItem?.(currentContextMenuTarget.item);
  }
  hideAllContextMenus();
});
on('ctx-lib-theme', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const item = currentContextMenuTarget.item;
    const tab = currentContextMenuTarget.tab || currentTab;
    openThemePicker(item, async (theme) => {
      item.theme_name = theme.name;
      if (item.slides) {
        item.slides.forEach((s: LibrarySlide) => s.background = theme.bg);
      }
      try {
        if (tab === 'songs') {
          await fetch('/api/songs', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(item)
          });
        } else if (tab === 'presentations') {
          await fetch('/api/presentations', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(item)
          });
        }
      } catch (e) {
        console.error('Error persisting theme:', e);
      }
      filterAndRenderCatalog();
      if (selectedLibraryItem && selectedLibraryItem.id === item.id) {
        renderAssetPreview(item, tab);
      }
    });
  }
  hideAllContextMenus();
});
on('ctx-lib-image', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const item = currentContextMenuTarget.item;
    const tab = currentContextMenuTarget.tab || currentTab;
    openMediaImagePicker({
      onSelectSlide: async (_filePath, cssBg) => {
        item.theme_name = name;
        if (item.slides) {
          item.slides.forEach((s: LibrarySlide) => s.background = cssBg);
        }
        try {
          if (tab === 'songs') {
            await fetch('/api/songs', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify(item)
            });
          } else if (tab === 'presentations') {
            await fetch('/api/presentations', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify(item)
            });
          }
        } catch (e) {
          console.error('Error persisting theme:', e);
        }
        filterAndRenderCatalog();
        if (selectedLibraryItem && selectedLibraryItem.id === item.id) {
          renderAssetPreview(item, tab);
        }
      }
    });
  }
  hideAllContextMenus();
});
on('ctx-lib-copy', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const it = currentContextMenuTarget.item;
    const text = it.slides ? it.slides.map((s: LibrarySlide) => s.text).join('\n\n') : (it.name || it.title || it.reference || '');
    copyToClipboard(text, '✓ Copied library item content to clipboard!');
  }
  hideAllContextMenus();
});
// Shown on every library item's context menu (same pattern as "Add Dual Comparison" above), but
// only meaningful for a presentation; a click from any other tab is a silent no-op.
on('ctx-lib-export-odp', 'click', async () => {
  const target = currentContextMenuTarget;
  hideAllContextMenus();
  if (!target || target.tab !== 'presentations' || !target.item) return;
  const pres = target.item;
  const title = pres.title || 'Presentation';
  try {
    const res = await fetch(`/api/export/odp/${encodeURIComponent(pres.id)}`);
    if (!res.ok) {
      showToast(`Export failed: ${await res.text()}`, 'error');
      return;
    }
    const blob = await res.blob();
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${title}.odp`;
    document.body.appendChild(a);
    a.click();
    a.remove();
    URL.revokeObjectURL(url);
    showToast(`✓ Exported "${title}" (.odp)`, 'success');
  } catch (e) {
    showToast(`Export failed: ${errorMessage(e)}`, 'error');
  }
});
// Songs and presentations save one file each; a Bible saves the whole translation the chapter belongs to.
// (The menu entry is hidden on the other tabs: see `prepareBundleMenuItem` in library_panel.ts.)
on('ctx-lib-export-bundle', 'click', async () => {
  const target = currentContextMenuTarget;
  hideAllContextMenus();
  if (!target || !target.item) return;
  const item = target.item;
  if (target.tab === 'songs') await exportBundle({ kind: 'song', ids: [item.id] }, item.title || 'Song');
  else if (target.tab === 'presentations') await exportBundle({ kind: 'presentation', ids: [item.id] }, item.title || 'Presentation');
  else if (target.tab === 'scriptures' && item.version) await exportBundle({ kind: 'bible', version: item.version }, item.version);
});
on('ctx-lib-delete', 'click', () => {
  if (currentContextMenuTarget && currentContextMenuTarget.item) {
    const target = currentContextMenuTarget;
    const it = target.item;
    const tab = target.tab || currentTab;
    const title = it.title || it.name || it.reference || 'Item';
    const typeLabel = tab.endsWith('s') ? tab.slice(0, -1) : tab;

    showConfirmDialog(
      `🗑️ Delete ${typeLabel.charAt(0).toUpperCase() + typeLabel.slice(1)}`,
      `Are you sure you want to permanently delete "${title}" from your ${tab} library?`,
      'This item will be removed from your database and cannot be undone.',
      async () => {
        try {
          await resolveHostSessionToken();
          const res = await fetch(`/api/${tab}/${encodeURIComponent(it.id)}`, {
            method: 'DELETE',
            headers: hostTokenHeader(),
          });
          if (res.ok || res.status === 204) {
            await loadLibraryTab(tab);
            showToast(`✓ Deleted "${title}" from ${tab}`, 'info');
          } else {
            const err = await res.text();
            showToast(`Could not delete item: ${err}`, 'error');
          }
        } catch (e) {
          showToast(`Delete error: ${e instanceof Error ? e.message : String(e)}`, 'error');
        }
      },
      '🗑️ Delete Permanently'
    );
  }
  hideAllContextMenus();
});

// 4. Live Output Monitor Context Menu Handlers
on('ctx-mon-black', 'click', () => { sendCommand('ToggleBlackout'); hideAllContextMenus(); });
on('ctx-mon-clear', 'click', () => { sendCommand('ToggleClearText'); hideAllContextMenus(); });
on('ctx-mon-logo', 'click', () => { sendCommand('ToggleLogo'); hideAllContextMenus(); });
on('ctx-mon-alert', 'click', () => {
  resetAlertModal();
  showModal(alertModal);
  if (alertTextInput) alertTextInput.focus();
  hideAllContextMenus();
});
on('ctx-mon-foh', 'click', () => {
  window.open('/live.html', 'OpenSanctuaryFOH', 'width=1280,height=720,menubar=no,toolbar=no');
  hideAllContextMenus();
});

// 5. Bible Versions Tree Context Menu Handlers
on('ctx-set-default-bible', 'click', () => {
  if (contextMenuTargetBible) {
    appOptions.defaultBibleVersion = contextMenuTargetBible.id;
    saveAppOptions({ defaultBibleVersion: contextMenuTargetBible.id });
    installedBibles.forEach(b => b.isDefault = areTranslationsEquivalent(b.id, contextMenuTargetBible.id));
    renderCategoryTree('scriptures');
    updateSearchModeUI();
    filterAndRenderCatalog();
    showToast(`✓ Set ${contextMenuTargetBible.abbreviation} (${contextMenuTargetBible.name}) as default translation`, 'success');
  }
  hideAllContextMenus();
});
on('ctx-compare-bible', 'click', () => {
  if (contextMenuTargetBible) {
    secondaryBibleVersion = contextMenuTargetBible.id;
    isDualBibleMode = true;
    if (btnToggleDualBible) {
      btnToggleDualBible.textContent = `👥 Dual: ${activeBibleVersion.toUpperCase()} | ${secondaryBibleVersion.toUpperCase()}`;
      btnToggleDualBible.classList.add('active');
    }
    if (selectedLibraryItem) renderAssetPreview(selectedLibraryItem, 'scriptures');
  }
  hideAllContextMenus();
});
on('ctx-install-more-bibles', 'click', () => {
  uiCallbacks.openImportModal?.();
  hideAllContextMenus();
});

// ============================================================================
// LIBRARY / RESOURCE PANEL (delegated to src/ui/library_panel.ts)
// ============================================================================
initLibraryPanel({
  getCurrentTab: () => currentTab,
  setCurrentTab: (tab) => { currentTab = tab; },
  getSelectedLibraryItem: () => selectedLibraryItem,
  setSelectedLibraryItem: (item) => { selectedLibraryItem = item; },
  getActiveBibleVersion: () => activeBibleVersion,
  setActiveBibleVersion: (v) => { activeBibleVersion = v; },
  getSecondaryBibleVersion: () => secondaryBibleVersion,
  setSecondaryBibleVersion: (v) => { secondaryBibleVersion = v; },
  getIsDualBibleMode: () => isDualBibleMode,
  setIsDualBibleMode: (v) => { isDualBibleMode = v; },
  getInstalledBibles: () => installedBibles,
  getAppOptions: () => appOptions,
  getAvailableThemes: () => typeof availableThemes !== 'undefined' ? availableThemes : [],
  getCurrentSnapshot: () => currentSnapshot,
  showToast: showToast,
  sendCommand: sendCommand,
  escapeHtml: escapeHtml,
  formatCssBackground: formatCssBackground,
  showContextMenu: showContextMenu,
  setContextMenuTarget: (target) => { currentContextMenuTarget = target; },
  setContextMenuTargetBible: (v) => { contextMenuTargetBible = v; },
  setBibleVersionUserSelected: (v) => { bibleVersionUserSelected = v; },
  applyDefaultBibleVersionIfUnset: applyDefaultBibleVersionIfUnset,
  formatMediaTime: formatMediaTime,
  setAdhocPreview: setAdhocPreview,
});


// Setter functions for mutable state app_ui.ts needs to write, not just read — a
// real `import { X }` of an exported `let` gives a live-reading binding, but JS
// forbids assigning to an imported binding, so writes go through these instead.
export function setActiveBibleVersion(v: typeof activeBibleVersion) { activeBibleVersion = v; }
export function setActiveImportMode(v: typeof activeImportMode) { activeImportMode = v; }
export function setActiveLiveViewMode(v: typeof activeLiveViewMode) { activeLiveViewMode = v; }
export function setCurrentContextMenuTargetState(v: typeof currentContextMenuTarget) { currentContextMenuTarget = v; }
export function setCurrentEditorType(v: typeof currentEditorType) { currentEditorType = v; }
export function setCurrentSnapshot(v: typeof currentSnapshot) { currentSnapshot = v; }
export function setEditingItemId(v: typeof editingItemId) { editingItemId = v; }
export function setExpandedScheduleIndex(v: typeof expandedScheduleIndex) { expandedScheduleIndex = v; }
export function setIsDualBibleMode(v: typeof isDualBibleMode) { isDualBibleMode = v; }
export function resetLastRenderedScheduleKey() { lastRenderedScheduleKey = ''; }

// Schedule Auto-Save (Every 2 minutes)
setInterval(() => {
  try {
    if (currentSnapshot && currentSnapshot.schedule && currentSnapshot.schedule.items && currentSnapshot.schedule.items.length > 0) {
      localStorage.setItem('opensanctuary_autosave_schedule', JSON.stringify({
        savedAt: new Date().toISOString(),
        // Licensed Bible text may be shown but not kept: only its reference is saved.
        schedule: stripLicensedText(currentSnapshot.schedule),
      }));
    }
  } catch (err) {
    console.warn('Auto-save failed:', err);
  }
}, 120000);
