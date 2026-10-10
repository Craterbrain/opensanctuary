import { SlideEditor, EditorSlide, projectTextFromElements, applyArchetypeToSlide, splitTextToSlides, stripChordsAndAnnotations, BulkSplitRule } from './editor/index';
import { resolveKeyboardShortcut, buildPairingUrl, buildRemoteUrl, escapeHtml } from './core/presentation_helpers.ts';
import { api } from './core/api_client.ts';
import { SETTINGS_SCHEMA } from './core/settings_schema';
import { setupScheduleDesktopDrop, uploadScheduleFile, fileToBase64 } from './core/schedule_drop';
import { resolveHostSessionToken, hostTokenHeader } from './core/host_session.ts';
import { initFirstTimeSetup, maybeShowFirstTimeSetup, showFirstTimeSetup } from './ui/first_time_setup.ts';
import QRCode from 'qrcode';
import {
  addDualScriptureToSchedule,
  addItemToSchedule,
  filterAndRenderCatalog,
  loadLibraryTab,
  renderCategoryTree,
  selectAndPreviewItem,
  tryHandleOnlineImagesEnter,
  updateSearchModeUI,
  getActiveLibraryItems,
  getActiveResourceViewMode,
  getFilteredLibraryItems,
  setActiveCategory,
  setActiveResourceViewMode,
} from './ui/library_panel.ts';
import { parseScriptureReference } from './core/bible_parser.ts';
import { resetOptionsModal, onSettingsSearchInput, saveNetworkSettings, refreshPairedDevicesAfterAdbProvision } from './ui/settings_dialog.ts';
import { showToast } from './core/ui_utils.ts';
import { openThemeEditor } from './ui/theme_editor.ts';
import {
  closeModal,
  closeTopmostModal,
  hideAllContextMenus,
  initWebSocket,
  loadAppOptions,
  on,
  refreshAvailableThemes,
  refreshInstalledBibles,
  renderAll,
  renderSchedule,
  resetAlertModal,
  resetImportModal,
  resetOpenModal,
  resetSaveModal,
  resetWebModal,
  saveAppOptions,
  sendCommand,
  showContextMenu,
  showModal,
  registerUiCallbacks,
  aboutModal,
  activeBibleVersion,
  activeImportMode,
  activeLiveViewMode,
  activeUndoPlaceholder,
  alertModal,
  alertTextInput,
  appOptions,
  availableThemes,
  btnAlert,
  btnBlack,
  btnClear,
  btnGoLive,
  btnImport,
  btnLiveStatus,
  btnLogo,
  btnNew,
  btnOpen,
  btnRemote,
  btnSave,
  btnStore,
  btnToggleDualBible,
  btnWeb,
  collapsedGroupIds,
  createItemAuthor,
  createItemContent,
  createItemCopyright,
  createItemTitle,
  createModal,
  currentContextMenuTarget,
  currentDualSecondaryVerses,
  currentEditorType,
  currentSnapshot,
  currentTab,
  deleteActiveSelectedItemWithUndo,
  deleteSelectedSlideOrItem,
  editingItemId,
  expandedScheduleIndex,
  handleImportBack,
  handleOptionsBack,
  importModal,
  installedBibles,
  isDualBibleMode,
  liveSlideMatrixEl,
  onlineBibleCatalog,
  openModal,
  optionsModal,
  previewSlideMatrixEl,
  remoteModal,
  resetLastRenderedScheduleKey,
  saveModal,
  scheduleArticlesModal,
  scheduleListEl,
  secondaryBibleVersion,
  selectedLibraryItem,
  setActiveBibleVersion,
  setActiveImportMode,
  setActiveLiveViewMode,
  setCurrentContextMenuTargetState,
  setCurrentEditorType,
  setCurrentSnapshot,
  setEditingItemId,
  setExpandedScheduleIndex,
  setIsDualBibleMode,
  setOnlineBibleCatalog,
  shortcutsModal,
  storeModal,
  triggerUndoFromPlaceholder,
  webModal,
} from './app_core.ts';

// These DOM elements are also declared in app_core.ts / library_panel.ts — a second,
// idempotent document.getElementById lookup here is safe (both resolve to the same
// stable node) and avoids needing a cross-module getter for elements that never change.
const resourceSearchInput = document.getElementById('resource-search') as HTMLInputElement | null;
const btnSearchClear = document.getElementById('btn-search-clear');
const searchModeDropdownWrap = document.getElementById('search-mode-dropdown-wrap');
const catalogTableBody = document.getElementById('catalog-table-body');
const catalogGrid = document.getElementById('catalog-grid');

// ============================================================================
// SEARCH INPUT ERGONOMICS & KEYSTROKE LISTENERS
// ============================================================================
if (resourceSearchInput) {
  resourceSearchInput.addEventListener('input', () => {
    filterAndRenderCatalog();
  });

  resourceSearchInput.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      resourceSearchInput.value = '';
      if (btnSearchClear) btnSearchClear.style.display = 'none';
      filterAndRenderCatalog();
    } else if (e.key === 'ArrowDown' || e.key === 'Tab') {
      e.preventDefault();
      if (getActiveResourceViewMode() === 'table') {
        const firstRow = catalogTableBody ? catalogTableBody.querySelector('tr') : null;
        if (firstRow) firstRow.focus();
      } else {
        const firstCard = catalogGrid ? catalogGrid.querySelector<HTMLElement>('.catalog-grid-card') : null;
        if (firstCard) firstCard.focus();
      }
    } else if (e.key === 'Enter') {
      if (tryHandleOnlineImagesEnter()) return;
      const promptRow = catalogTableBody ? catalogTableBody.querySelector<HTMLElement>('.catalog-fetch-prompt-row') : null;
      if (promptRow) {
        promptRow.click();
      } else {
        const filtered = getFilteredLibraryItems();
        if (filtered.length > 0) {
          addItemToSchedule(currentTab, filtered[0].id);
        }
      }
    }
  });
}

// ============================================================================
// SLIDE & PRESENTATION CREATOR / EDITOR
// ============================================================================
// FREE-FORM CANVAS SLIDE EDITOR CONTROLLER
// ============================================================================
let studioSlides: EditorSlide[] = [];
let canvasSlideEditor: SlideEditor | null = null;
// Set when editing an item that's a known member of the live Schedule (see
// ctx-sched-edit / ctx-slide-edit below) — lets saveEditorItem additionally
// push the active slide's edits live via BatchSlideEdit, on top of the normal
// REST save to the library, so an operator sees the change immediately on
// whatever's live/staged/scheduled rather than only in the catalog.
let editingScheduleContext: { itemIndex: number; slideIndex?: number } | null = null;

let lastApplyResult: { slides: EditorSlide[]; batchOps: any[]; hasChanges: boolean; activeIndex: number } | null = null;

function getCanvasSlideEditor(): SlideEditor {
  if (!canvasSlideEditor) {
    const mount = document.getElementById('canvas-editor-mount');
    canvasSlideEditor = new SlideEditor({
      container: mount as HTMLElement,
      onCancel: () => closeModal(createModal),
      onApply: (result) => { lastApplyResult = result; }
    });
  }
  return canvasSlideEditor;
}

// Builds a blank EditorSlide seeded with a sensible starting layout for the
// given item type, via the same archetype system the canvas editor's own
// "Apply Layout" toolbar action uses.
function buildBlankEditorSlide(type: string, index: number): EditorSlide {
  let archetype = 'lyric';
  if (type === 'presentation') archetype = index === 0 ? 'title' : 'title_body';
  else if (type === 'scripture') archetype = 'scripture';

  const slide: EditorSlide = {
    id: `slide-${Date.now()}-${index}`,
    text: '',
    elements: [],
    speaker_notes: '',
    ccli_metadata: {},
    slide_document_version: 2
  };
  // Apply this category's default theme (Themes tab > right-click > Set as Default)
  // to newly created slides, rather than leaving background unset and falling
  // through to the resolution cascade's generic fallback gradient.
  const themeCategory = type === 'scripture' ? 'scripture' : (type === 'presentation' ? 'presentation' : 'song');
  const defaultTheme = (typeof availableThemes !== 'undefined' ? availableThemes : []).find(t => (t.category || 'song') === themeCategory && t.isDefault);
  if (defaultTheme) slide.background = defaultTheme.bg;
  slide.elements = applyArchetypeToSlide(archetype, slide);
  slide.text = projectTextFromElements(slide.elements);
  return slide;
}

function openSlideEditor(type = 'song') {
  setCurrentEditorType(type);
  setEditingItemId(null);

  document.querySelectorAll<HTMLElement>('.type-pill').forEach(pill => {
    pill.classList.toggle('active', pill.dataset.type === type);
  });

  if (createItemTitle) {
    createItemTitle.value = '';
    if (type === 'song') createItemTitle.placeholder = 'Song Title (e.g. 10,000 Reasons)';
    else if (type === 'scripture') createItemTitle.placeholder = 'Passage Reference (e.g. John 3:16-17)';
    else createItemTitle.placeholder = 'Presentation / Sermon Title (e.g. Sunday Message)';
  }
  if (createItemAuthor) {
    createItemAuthor.value = '';
    if (type === 'song') createItemAuthor.placeholder = 'Author / Composer';
    else if (type === 'scripture') createItemAuthor.placeholder = 'Translation / Version';
    else createItemAuthor.placeholder = 'Speaker / Ministry';
  }
  if (createItemCopyright) createItemCopyright.value = '';
  if (createItemContent) createItemContent.value = '';

  editingScheduleContext = null;
  studioSlides = [buildBlankEditorSlide(type, 0)];
  closeBulkOverlay();
  if (createModal) showModal(createModal);
  getCanvasSlideEditor().open(studioSlides, 0, currentEditorType);
}

function editExistingItem(item: any, scheduleContext: { itemIndex: number; slideIndex?: number } | null = null) {
  if (!item) return;
  setEditingItemId(item.id);
  editingScheduleContext = scheduleContext;

  const isPres = item.slides && item.slides[0] && item.slides[0].content !== undefined;
  setCurrentEditorType(item.item_type ? item.item_type.toLowerCase() : (isPres ? 'presentation' : 'song'));

  document.querySelectorAll<HTMLElement>('.type-pill').forEach(pill => {
    pill.classList.toggle('active', pill.dataset.type === currentEditorType);
  });

  if (createItemTitle) createItemTitle.value = item.title || item.name || item.reference || '';
  if (createItemAuthor) createItemAuthor.value = item.author || item.version || '';
  if (createItemCopyright) createItemCopyright.value = item.copyright || item.ccli_number || '';
  if (createItemContent) createItemContent.value = '';

  let editorSlides: EditorSlide[] = [];
  if (item.slides && item.slides.length > 0) {
    editorSlides = item.slides.map((s: any, idx: number) => {
      const bodyText = s.text || s.content || '';
      const layout = s.layout || (currentEditorType === 'presentation' ? (idx === 0 ? 'title' : 'title_body') : 'lyric');
      const base: EditorSlide = {
        id: s.id || `slide-${idx}`,
        text: bodyText,
        header: s.header,
        label: s.label || s.title || `Slide ${idx + 1}`,
        tag: s.tag,
        notes: s.notes,
        background: s.background,
        elements: [],
        speaker_notes: s.speaker_notes || s.notes || '',
        ccli_metadata: s.ccli_metadata || {
          title: item.title,
          author: item.author,
          copyright: item.copyright,
          ccli_number: item.ccli_number
        },
        background_v2: s.background_v2,
        transition: s.transition,
        slide_document_version: s.slide_document_version || 2,
        duration_seconds: s.duration_seconds
      };
      base.elements = (s.elements && s.elements.length > 0) ? s.elements : applyArchetypeToSlide(layout, base);
      return base;
    });
  } else if (item.verses && item.verses.length > 0) {
    editorSlides = item.verses.map((v: any, idx: number) => {
      const base: EditorSlide = {
        id: `verse-${idx}`,
        text: v.text,
        label: `${item.title || item.name || 'Passage'} : ${v.verse_number}`,
        reference_label: `${item.title || item.name || 'Passage'} ${v.verse_number}`,
        elements: [],
        speaker_notes: '',
        ccli_metadata: {},
        slide_document_version: 2
      };
      base.elements = applyArchetypeToSlide('scripture', base);
      return base;
    });
  } else {
    editorSlides = [buildBlankEditorSlide(currentEditorType, 0)];
  }

  studioSlides = editorSlides;
  closeBulkOverlay();
  if (createModal) showModal(createModal);
  getCanvasSlideEditor().open(studioSlides, 0, currentEditorType);
}

// The Bulk Paste textarea is a temporary overlay above the canvas, not a
// persistent "mode" — the canvas editor (and its properties/notes panes)
// stays mounted underneath it the whole time, so there's no view-switching
// to keep in sync. Converting builds/replaces the slide deck and closes the
// overlay; there's no separate "back to visual" step.
function updateBulkSlideCountBadge(): void {
  const badge = document.getElementById('bulk-slide-count-badge');
  if (!badge) return;
  const textarea = document.getElementById('create-item-content') as HTMLTextAreaElement | null;
  const raw = textarea ? textarea.value : '';
  const splitRuleSelect = document.getElementById('bulk-split-rule') as HTMLSelectElement | null;
  const splitRule = (splitRuleSelect?.value || 'paragraphs') as BulkSplitRule;
  const res = splitTextToSlides(raw, {
    splitRule,
    editorType: (currentEditorType || 'song') as any,
    extractMetadata: true,
  });
  const count = res.slides.length;
  badge.textContent = `⚡ ${count} Slide${count === 1 ? '' : 's'}`;
}

// The Bulk Paste textarea is a temporary overlay above the canvas, not a
// persistent "mode" — the canvas editor (and its properties/notes panes)
// stays mounted underneath it the whole time, so there's no view-switching
// to keep in sync. Converting builds/replaces the slide deck and closes the
// overlay; there's no separate "back to visual" step.
function openBulkOverlay(): void {
  const bulkView = document.getElementById('studio-bulk-view');
  if (bulkView) {
    bulkView.style.display = 'flex';
    const isSong = (currentEditorType || 'song').toLowerCase() === 'song';
    const tagsRow = document.getElementById('studio-bulk-tags-row');
    if (tagsRow) tagsRow.style.display = isSong ? 'flex' : 'none';
  }
  // Populate from the deck's current slide text so opening the overlay after
  // already having slides doesn't start from a blank textarea.
  const textarea = document.getElementById('create-item-content') as HTMLTextAreaElement | null;
  if (textarea) {
    const cur = canvasSlideEditor ? canvasSlideEditor.getAllSlides() : studioSlides;
    textarea.value = cur.map(s => s.text || '').join('\n\n');
  }
  updateBulkSlideCountBadge();
}

function closeBulkOverlay(): void {
  const bulkView = document.getElementById('studio-bulk-view');
  if (bulkView) bulkView.style.display = 'none';
}

function isBulkOverlayOpen(): boolean {
  const bulkView = document.getElementById('studio-bulk-view');
  return !!bulkView && bulkView.style.display !== 'none';
}

function cleanBulkChords(): void {
  const textarea = document.getElementById('create-item-content') as HTMLTextAreaElement | null;
  if (!textarea) return;
  const original = textarea.value;
  if (!original.trim()) {
    showToast('Textarea is empty.', 'info');
    return;
  }
  const cleaned = stripChordsAndAnnotations(original);
  textarea.value = cleaned;
  updateBulkSlideCountBadge();
  showToast('Cleaned chords and performance cues.', 'info');
}

function convertBulkTextToSlides() {
  const textarea = document.getElementById('create-item-content') as HTMLTextAreaElement | null;
  const raw = textarea ? textarea.value.trim() : '';
  if (!raw) {
    showToast('Please enter or paste lyrics/text to convert.', 'warning');
    return;
  }

  const splitRuleSelect = document.getElementById('bulk-split-rule') as HTMLSelectElement | null;
  const splitRule = (splitRuleSelect?.value || 'paragraphs') as BulkSplitRule;
  const actionModeSelect = document.getElementById('bulk-action-mode') as HTMLSelectElement | null;
  const actionMode = actionModeSelect?.value || 'replace';

  const { slides: parsedSlides, metadata } = splitTextToSlides(raw, {
    splitRule,
    editorType: (currentEditorType || 'song') as any,
    extractMetadata: true,
    autoCleanChords: true
  });

  if (parsedSlides.length === 0) {
    showToast('No slides generated from the text.', 'warning');
    return;
  }

  // Auto-fill author & copyright if detected and fields are empty
  if (metadata.author && createItemAuthor && !createItemAuthor.value.trim()) {
    createItemAuthor.value = metadata.author;
  }
  if (metadata.copyright && createItemCopyright && !createItemCopyright.value.trim()) {
    createItemCopyright.value = metadata.copyright;
  } else if (metadata.ccli_number && createItemCopyright && !createItemCopyright.value.trim()) {
    createItemCopyright.value = `CCLI Song # ${metadata.ccli_number}`;
  }

  const archetype = currentEditorType === 'presentation' ? 'title_body' : (currentEditorType === 'scripture' ? 'scripture' : 'lyric');

  const newSlides: EditorSlide[] = parsedSlides.map((ps, idx) => {
    const base: EditorSlide = {
      id: `slide-${Date.now()}-${idx}-${Math.random().toString(36).substring(2, 6)}`,
      text: ps.text,
      header: ps.header,
      tag: ps.tag,
      label: ps.label,
      elements: [],
      speaker_notes: '',
      ccli_metadata: metadata.ccli_number ? { ccli_number: metadata.ccli_number, author: metadata.author, copyright: metadata.copyright } : {},
      slide_document_version: 2
    };
    base.elements = applyArchetypeToSlide(archetype, base);
    return base;
  });

  const curSlides = canvasSlideEditor ? canvasSlideEditor.getAllSlides() : studioSlides;
  const isOnlyBlankPlaceholder = curSlides.length === 1 && !curSlides[0].text?.trim() && (!curSlides[0].elements || curSlides[0].elements.length === 0);

  if (actionMode === 'append' && !isOnlyBlankPlaceholder) {
    const combined = [...curSlides, ...newSlides];
    studioSlides = combined;
    closeBulkOverlay();
    getCanvasSlideEditor().open(studioSlides, curSlides.length, currentEditorType);
    showToast(`Appended ${newSlides.length} slide${newSlides.length === 1 ? '' : 's'} to deck.`, 'success');
  } else {
    studioSlides = newSlides;
    closeBulkOverlay();
    getCanvasSlideEditor().open(studioSlides, 0, currentEditorType);
    showToast(`Converted ${newSlides.length} slide${newSlides.length === 1 ? '' : 's'}.`, 'success');
  }
}

on('btn-open-bulk-paste', 'click', () => openBulkOverlay());
on('btn-cancel-bulk-paste', 'click', () => closeBulkOverlay());
on('btn-bulk-to-slides', 'click', () => convertBulkTextToSlides());
on('btn-bulk-clean-chords', 'click', () => cleanBulkChords());
on('create-item-content', 'input', () => updateBulkSlideCountBadge());
on('bulk-split-rule', 'change', () => updateBulkSlideCountBadge());

document.querySelectorAll<HTMLElement>('.type-pill').forEach(pill => {
  pill.addEventListener('click', () => {
    const newType = (pill as HTMLElement).dataset.type || 'song';
    if (createModal && createModal.style.display !== 'none' && canvasSlideEditor) {
      setCurrentEditorType(newType);
      document.querySelectorAll<HTMLElement>('.type-pill').forEach(p => {
        p.classList.toggle('active', p.dataset.type === newType);
      });
      if (createItemTitle && (!createItemTitle.value || createItemTitle.value.startsWith('Untitled'))) {
        if (newType === 'song') createItemTitle.placeholder = 'Song Title (e.g. 10,000 Reasons)';
        else if (newType === 'scripture') createItemTitle.placeholder = 'Passage Reference (e.g. John 3:16-17)';
        else createItemTitle.placeholder = 'Presentation / Sermon Title (e.g. Sunday Message)';
      }
      if (createItemAuthor && !createItemAuthor.value) {
        if (newType === 'song') createItemAuthor.placeholder = 'Author / Composer';
        else if (newType === 'scripture') createItemAuthor.placeholder = 'Translation / Version';
        else createItemAuthor.placeholder = 'Speaker / Ministry';
      }
      canvasSlideEditor.setMode(newType);
      return;
    }
    openSlideEditor(newType);
  });
});

on('btn-insert-verse', 'click', () => insertTagIntoBulkEditor('Verse 1'));
on('btn-insert-verse-2', 'click', () => insertTagIntoBulkEditor('Verse 2'));
on('btn-insert-chorus', 'click', () => insertTagIntoBulkEditor('Chorus 1'));
on('btn-insert-chorus-2', 'click', () => insertTagIntoBulkEditor('Chorus 2'));
on('btn-insert-bridge', 'click', () => insertTagIntoBulkEditor('Bridge 1'));
on('btn-insert-prechorus', 'click', () => insertTagIntoBulkEditor('Pre-Chorus'));
on('btn-insert-intro', 'click', () => insertTagIntoBulkEditor('Intro'));
on('btn-insert-ending', 'click', () => insertTagIntoBulkEditor('Ending'));
on('btn-insert-blank', 'click', () => insertTagIntoBulkEditor('---'));

function insertTagIntoBulkEditor(tag: string) {
  const textarea = createItemContent;
  if (!textarea) return;
  const tagStr = tag === '---' ? `\n\n---\n\n` : `\n\n${tag}\n`;
  const start = textarea.selectionStart ?? 0;
  const end = textarea.selectionEnd ?? 0;
  textarea.value = textarea.value.substring(0, start) + tagStr + textarea.value.substring(end);
  const newPos = start + tagStr.length;
  textarea.setSelectionRange(newPos, newPos);
  textarea.focus();
  updateBulkSlideCountBadge();
}

async function saveEditorItem(addToSchedule = false) {
  const title = createItemTitle ? createItemTitle.value.trim() : '';
  const author = createItemAuthor ? createItemAuthor.value.trim() : '';
  const copyright = createItemCopyright ? createItemCopyright.value.trim() : '';

  if (!title) {
    showToast('Please enter a Title or Reference.', 'warning');
    if (createItemTitle) createItemTitle.focus();
    return;
  }

  if (isBulkOverlayOpen()) {
    convertBulkTextToSlides();
  }

  const editor = getCanvasSlideEditor();
  lastApplyResult = null;
  editor.apply();
  // `editor.apply()` synchronously invokes the `onApply` callback registered when
  // this editor was opened (line ~189), which reassigns `lastApplyResult` — but
  // TypeScript's control-flow analysis can't see through that closure call, so it
  // still thinks `lastApplyResult` is exactly `null` here. Re-assert the real
  // declared type to read the value the callback actually set.
  const applyResult = lastApplyResult as {
    slides: EditorSlide[]; batchOps: any[]; hasChanges: boolean; activeIndex: number
  } | null;
  studioSlides = editor.getAllSlides();

  if (!studioSlides.length) {
    showToast('Please add at least one slide.', 'warning');
    return;
  }

  // If this editor session is against a known live/staged/scheduled slot,
  // push the active slide's edits live immediately via BatchSlideEdit, on top
  // of the REST save below (which persists all slides to the library). This
  // only covers the slide that was active when Save was clicked — matches
  // SlideEditor's own history model, which tracks one slide's diff at a time.
  if (editingScheduleContext && applyResult && applyResult.hasChanges && applyResult.batchOps.length > 0) {
    sendCommand({
      BatchSlideEdit: {
        item_index: editingScheduleContext.itemIndex,
        slide_index: editingScheduleContext.slideIndex ?? applyResult.activeIndex,
        ops: applyResult.batchOps
      }
    });
  }

  const slides = studioSlides.map((s, idx) => ({
    id: s.id || `${Date.now()}-${idx}`,
    label: s.label || `Slide ${idx + 1}`,
    verse_tag: s.tag || `V${idx + 1}`,
    tag: s.tag || null,
    duration_seconds: s.duration_seconds ?? null,
    text: s.text || '',
    content: s.text || '',
    notes: s.speaker_notes || s.notes || null,
    speaker_notes: s.speaker_notes || s.notes || null,
    ccli_metadata: s.ccli_metadata || null,
    background: s.background || null,
    background_v2: s.background_v2 || null,
    transition: s.transition || null,
    elements: s.elements || [],
    slide_document_version: 2,
    reference_label: s.reference_label || null
  }));

  const payload = {
    id: editingItemId || `item-${Date.now()}`,
    title,
    author: author || 'Media Ministry',
    alternate_title: null,
    copyright: copyright || 'Public Domain',
    ccli_number: null,
    key: null,
    tempo: null,
    verse_order: null,
    theme_name: '',
    slides,
    modified_date: new Date().toISOString()
  };

  try {
    let endpoint = '/api/songs';
    let reqBody = JSON.stringify(payload);
    let savedItemId = payload.id;

    if (currentEditorType === 'presentation') {
      endpoint = '/api/presentations';
      const presId = editingItemId || `pres_${Date.now()}`;
      savedItemId = presId;
      const presPayload = {
        id: presId,
        title: title,
        // The backend's Presentation.author is a required String, not
        // Option<String> — sending null 422s the whole save when the
        // Author/Speaker field is left blank.
        author: author || '',
        slides: slides.map((s, idx) => ({
          id: s.id,
          title: s.label || `Slide ${idx + 1}`,
          content: s.text,
          text: s.text,
          notes: s.notes,
          speaker_notes: s.speaker_notes,
          ccli_metadata: s.ccli_metadata,
          background: s.background,
          background_v2: s.background_v2,
          transition: s.transition,
          elements: s.elements,
          slide_document_version: 2,
          order_index: idx
        })),
        modified_date: new Date().toISOString()
      };
      reqBody = JSON.stringify(presPayload);
    } else if (currentEditorType === 'scripture') {
      endpoint = '/api/scriptures';
      const parsed = parseScriptureReference(title);
      const bookName = (parsed && parsed.isReference && parsed.book) ? parsed.book : (title.replace(/\s+\d+.*$/, '') || title);
      const scriptureId = editingItemId || `scrip_${Date.now()}`;
      savedItemId = scriptureId;

      const scripturePayload = {
        id: scriptureId,
        book: bookName,
        chapter: (parsed && parsed.chapter) ? parsed.chapter : 1,
        verse_start: (parsed && parsed.verseStart) ? parsed.verseStart : 1,
        verse_end: (parsed && parsed.verseEnd) ? parsed.verseEnd : slides.length,
        version: author || 'KJV',
        reference: title,
        verses: slides.map((sl, vIdx) => ({
          verse_number: (parsed && parsed.verseStart) ? (parsed.verseStart + vIdx) : (vIdx + 1),
          text: sl.text
        }))
      };
      reqBody = JSON.stringify(scripturePayload);
    }

    const res = await fetch(endpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: reqBody
    });

    if (res.ok) {
      closeModal(createModal);
      const targetTab = currentEditorType === 'song' ? 'songs' : (currentEditorType === 'scripture' ? 'scriptures' : 'presentations');
      try {
        await loadLibraryTab(targetTab);
      } catch (e) {
        console.warn('Error reloading catalog tab:', e);
      }

      if (addToSchedule) {
        sendCommand({ AddToSchedule: { item_type: currentEditorType, item_id: savedItemId } });
        showToast(`✓ Saved "${title}" to Library & added to Schedule`, 'success');
      } else {
        showToast(`✓ Saved "${title}" to Library`, 'success');
      }
    } else {
      const errText = await res.text();
      showToast(`Server error saving item: ${errText}`, 'error');
    }
  } catch (err) {
    showToast(`Error saving item: ${err instanceof Error ? err.message : String(err)}`, 'error');
  }
}

on('btn-close-create', 'click', () => closeModal(createModal));
on('btn-cancel-create', 'click', () => closeModal(createModal));
on('btn-save-created-item', 'click', () => saveEditorItem(false));
on('btn-save-to-schedule', 'click', () => saveEditorItem(true));
on('btn-add-and-stage-item', 'click', () => saveEditorItem(true));

// ============================================================================
// TOP MENUBAR CONTROLS (File, Edit, Live, Profiles, View, Help)
// ============================================================================
export function initTopMenubar() {
  document.querySelectorAll('.menu-trigger-btn').forEach(btn => {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      hideAllContextMenus();
      const dropdown = btn.parentElement;
      if (!dropdown) return;
      const wasActive = dropdown.classList.contains('active');
      document.querySelectorAll('.menu-item-dropdown').forEach(d => d.classList.remove('active'));
      document.querySelectorAll('.ribbon-split-btn-wrap').forEach(d => d.classList.remove('open'));
      if (searchModeDropdownWrap) searchModeDropdownWrap.classList.remove('open');
      if (!wasActive) dropdown.classList.add('active');
    });
  });

  document.addEventListener('click', () => {
    document.querySelectorAll('.menu-item-dropdown').forEach(d => d.classList.remove('active'));
    document.querySelectorAll('.ribbon-split-btn-wrap').forEach(d => d.classList.remove('open'));
    if (searchModeDropdownWrap) searchModeDropdownWrap.classList.remove('open');
  });

  // File Menu Actions
  on('menu-file-new', 'click', () => sendCommand('NewSchedule'));
  on('menu-file-open', 'click', () => { resetOpenModal(); showModal(openModal); });
  on('menu-file-save', 'click', () => quickSaveSchedule());
  on('menu-file-save-as', 'click', () => { resetSaveModal(); showModal(saveModal); });
  on('menu-file-new-song', 'click', () => openSlideEditor('song'));
  on('menu-file-import', 'click', () => openImportModal());
  on('menu-file-exit', 'click', () => {
    if (confirm('Close OpenSanctuary?')) window.close();
  });

  // Edit Menu Actions
  on('menu-edit-undo', 'click', () => document.execCommand('undo'));
  on('menu-edit-redo', 'click', () => document.execCommand('redo'));
  on('menu-edit-new-slide', 'click', () => openSlideEditor('song'));
  on('menu-edit-clear-schedule', 'click', () => sendCommand('NewSchedule'));
  on('menu-edit-options', 'click', () => { resetOptionsModal(); showModal(optionsModal); });

  // Live Menu Actions
  on('menu-live-golive', 'click', () => sendCommand({ GoLive: { item_index: null, slide_index: null } }));
  on('menu-live-next', 'click', () => sendCommand('NextSlide'));
  on('menu-live-prev', 'click', () => sendCommand('PrevSlide'));
  on('menu-live-black', 'click', () => sendCommand('ToggleBlackout'));
  on('menu-live-clear', 'click', () => sendCommand('ToggleClearText'));
  on('menu-live-logo', 'click', () => sendCommand('ToggleLogo'));
  on('menu-live-alert', 'click', () => {
    resetAlertModal();
    showModal(alertModal);
    if (alertTextInput) alertTextInput.focus();
  });

  // View Menu Actions
  on('menu-view-fullscreen', 'click', () => {
    if (!document.fullscreenElement) {
      document.documentElement.requestFullscreen().catch(() => {});
    } else {
      document.exitFullscreen().catch(() => {});
    }
  });
  on('menu-view-dual-bible', 'click', () => {
    setIsDualBibleMode(!isDualBibleMode);
    if (btnToggleDualBible) btnToggleDualBible.classList.toggle('active', isDualBibleMode);
    if (selectedLibraryItem && currentTab === 'scriptures') {
      selectAndPreviewItem(selectedLibraryItem, 'scriptures');
    }
  });
  on('menu-view-open-foh', 'click', () => {
    window.open('/live.html', 'OpenSanctuaryFOH', 'width=1280,height=720,menubar=no,toolbar=no');
  });
  on('menu-view-open-stage', 'click', () => {
    window.open('/stage.html', 'OpenSanctuaryStage', 'width=1280,height=720,menubar=no,toolbar=no');
  });

  // Help Menu Actions
  on('menu-help-shortcuts', 'click', () => { showModal(shortcutsModal); });
  on('menu-help-bibles', 'click', () => {
    const installedList = installedBibles.map(b => b.abbreviation).join(', ');
    showToast(`Installed Bibles (${installedBibles.length}): ${installedList} (Default: ${appOptions.defaultBibleVersion.toUpperCase()})`, 'info', 5000);
  });
  on('menu-help-about', 'click', () => { showModal(aboutModal); });
}
initTopMenubar();

export function promptNewSectionHeader(insertIndex: number | null) {
  const title = prompt('Enter Service Section / Group Header Title (e.g. "Praise & Worship", "Message & Scripture", "Announcements"):', 'Praise & Worship');
  if (title && title.trim()) {
    const idx = (typeof insertIndex === 'number') ? insertIndex : null;
    sendCommand({ AddScheduleHeader: { title: title.trim(), index: idx } });
    showToast(`✓ Added Section Header "${title.trim()}" to schedule`, 'success');
  }
}

// ============================================================================
// RIBBON BUTTONS & SPLIT DROPDOWNS
// ============================================================================
export function initRibbonControls() {
  if (btnNew) {
    btnNew.addEventListener('click', (e) => {
      e.stopPropagation();
      hideAllContextMenus();
      const wrap = btnNew?.parentElement;
      if (wrap) wrap.classList.toggle('open');
    });
  }

  // Right-click on empty area of schedule list
  if (scheduleListEl) {
    scheduleListEl.addEventListener('contextmenu', (e: any) => {
      if (e.target === scheduleListEl || (e.target.id === 'schedule-items-list' || (!e.target.closest('.schedule-item') && !e.target.closest('.schedule-child-item')))) {
        e.preventDefault();
        setCurrentContextMenuTargetState({ type: 'schedule-panel' });
        showContextMenu(document.getElementById('schedule-empty-context-menu'), e.clientX, e.clientY);
      }
    });

    scheduleListEl.addEventListener('dragover', (e) => {
      e.preventDefault();
      if (e.dataTransfer) {
        e.dataTransfer.dropEffect = 'copy';
      }
    });
  }

  on('rb-new-schedule', 'click', () => sendCommand('NewSchedule'));
  on('rb-new-song', 'click', () => openSlideEditor('song'));
  on('rb-new-presentation', 'click', () => openSlideEditor('presentation'));
  on('rb-new-scripture', 'click', () => openSlideEditor('scripture'));
  on('rb-new-video', 'click', async () => {
    const url = prompt('Enter Video File Path or Stream URL:');
    if (!url || !url.trim()) return;
    const trimmed = url.trim();
    const videoItem = {
      id: `video_${Date.now()}`,
      name: `🎥 ${trimmed.replace(/^https?:\/\//, '').split('/').pop() || trimmed}`,
      file_path: trimmed,
      media_type: 'video',
      duration_seconds: null,
      thumbnail_path: null,
      loop_playback: false
    };
    try {
      await fetch('/api/media', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(videoItem)
      });
    } catch (e) {}

    addItemToSchedule('media', videoItem.id);
    loadLibraryTab('media');
    setActiveCategory('videos');
    updateSearchModeUI();
    filterAndRenderCatalog();
    showToast(`Added "${videoItem.name}" to schedule`, 'success');
  });
  on('rb-new-camera', 'click', () => {
    loadLibraryTab('media');
    setActiveCategory('feeds');
    updateSearchModeUI();
    filterAndRenderCatalog();
    const feed = (getActiveLibraryItems() || []).find(m => (m.media_type || '').toLowerCase().includes('feed') || (m.name || '').toLowerCase().includes('camera'));
    if (feed) {
      addItemToSchedule('media', feed.id);
      showToast(`Added ${feed.name} to schedule`, 'success');
    } else {
      // Live camera capture isn't implemented — this used to silently do
      // nothing when no pre-existing "feed"/"camera" media item existed,
      // which looked like a dead button. Say so honestly instead: add a
      // web-based camera/RTMP stream via the Web Integration modal (the
      // real, working path for a live external feed today).
      showToast('No camera feed is set up yet. Live camera capture isn’t built — add an RTMP/web camera feed instead via the Web button.', 'warning', 6000);
    }
  });
  on('rb-new-web', 'click', () => {
    resetWebModal();
    showModal(webModal);
  });
  on('rb-new-header', 'click', () => promptNewSectionHeader(null));
  on('menu-file-new-header', 'click', () => promptNewSectionHeader(null));
  on('menu-file-open', 'click', () => { resetOpenModal(); showModal(openModal); });
  on('btn-sched-open', 'click', () => { resetOpenModal(); showModal(openModal); });
  on('menu-help-guide', 'click', () => openScheduleGuideModal());

  on(btnOpen, 'click', () => { resetOpenModal(); showModal(openModal); });
  on(btnSave, 'click', () => quickSaveSchedule());
  on(btnImport, 'click', () => openImportModal());
  on(btnStore, 'click', () => { showModal(storeModal); });
  on(btnWeb, 'click', () => { resetWebModal(); showModal(webModal); });
  on('btn-add-web-to-schedule', 'click', async () => {
    const urlInput = document.getElementById('web-stream-url') as HTMLInputElement | null;
    const url = (urlInput ? urlInput.value : '').trim();
    if (!url) {
      showToast('Please enter a web URL or stream link', 'warning');
      return;
    }
    const webItem = {
      id: `web_${Date.now()}`,
      name: `🌐 Web: ${url.replace(/^https?:\/\//, '').split('/')[0] || url}`,
      file_path: url,
      media_type: 'LiveFeed',
      duration_seconds: null,
      thumbnail_path: null,
      loop_playback: true
    };
    try {
      await fetch('/api/media', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(webItem)
      });
    } catch (e) {}

    addItemToSchedule('media', webItem.id);
    closeModal(webModal);
    showToast(`Added ${webItem.name} to schedule!`, 'success');
  });
  on(btnRemote, 'click', async () => {
    showModal(remoteModal);
    await updateRemoteQrCode();
  });

  on('btn-copy-remote-url', 'click', async () => {
    const disp = document.getElementById('remote-url-display');
    if (disp && disp.textContent) {
      try {
        await navigator.clipboard.writeText(disp.textContent.trim());
        showToast('Remote URL copied to clipboard!', 'success');
      } catch (_) {
        showToast('Could not copy URL', 'warning');
      }
    }
  });

  on(btnGoLive, 'click', () => sendCommand({ GoLive: { item_index: null, slide_index: null } }));
  on(btnAlert, 'click', () => {
    resetAlertModal();
    showModal(alertModal);
    if (alertTextInput) {
      alertTextInput.focus();
      alertTextInput.select();
    }
  });
  on(btnBlack, 'click', () => sendCommand('ToggleBlackout'));
  on(btnClear, 'click', () => sendCommand('ToggleClearText'));
  on(btnLogo, 'click', () => sendCommand('ToggleLogo'));
  on(btnLiveStatus, 'click', () => {
    showToast('Live projection is actively broadcasting to FOH monitors & Stage Foldback.', 'info');
  });
}
initRibbonControls();

// Schedule View & Icon Size Customizer (Summary, Small, Medium, Large)
const SCHED_VIEW_CLASSES = ['sched-view-summary', 'sched-view-small', 'sched-view-medium', 'sched-view-large'];
let currentSchedIconLevel = 2; // Default: Medium (2)

function setScheduleIconViewLevel(level: string | number, notify = false) {
  currentSchedIconLevel = Math.max(0, Math.min(3, parseInt(String(level), 10) || 0));
  try {
    localStorage.setItem('opensanctuary_sched_icon_level', currentSchedIconLevel.toString());
  } catch (e) {}

  const listEl = scheduleListEl;
  if (listEl) {
    SCHED_VIEW_CLASSES.forEach(cls => listEl.classList.remove(cls));
    listEl.classList.add(SCHED_VIEW_CLASSES[currentSchedIconLevel]);
  }

  const slider = document.getElementById('sched-view-slider') as HTMLInputElement | null;
  if (slider) slider.value = String(currentSchedIconLevel);

  document.querySelectorAll<HTMLElement>('.sched-tick-label').forEach(el => {
    if (parseInt(el.dataset.level || '', 10) === currentSchedIconLevel) {
      el.classList.add('active');
    } else {
      el.classList.remove('active');
    }
  });

  document.querySelectorAll<HTMLElement>('.sched-view-btn').forEach(btn => {
    if (parseInt(btn.dataset.level || '', 10) === currentSchedIconLevel) {
      btn.classList.add('active');
    } else {
      btn.classList.remove('active');
    }
  });

  const names = ['Summary View', 'Small Icons', 'Medium Icons', 'Large Icons'];
  if (notify) {
    showToast(`✓ Schedule view set to ${names[currentSchedIconLevel]}`, 'info');
  }
}

// Load saved view preference on boot
try {
  const savedLevel = localStorage.getItem('opensanctuary_sched_icon_level');
  if (savedLevel !== null) {
    currentSchedIconLevel = parseInt(savedLevel, 10) || 2;
  }
} catch (e) {}
setScheduleIconViewLevel(currentSchedIconLevel, false);

// Dropdown toggle
const schedViewMenuBtn = document.getElementById('btn-sched-view-menu');
const schedViewDropdown = document.getElementById('sched-view-menu-dropdown');

function positionSchedViewDropdown() {
  if (!schedViewMenuBtn || !schedViewDropdown) return;
  const rect = schedViewMenuBtn.getBoundingClientRect();
  const dropdownWidth = 224;
  schedViewDropdown.style.top = `${rect.bottom + 4}px`;

  let left = rect.right - dropdownWidth;
  if (left < 8) {
    left = Math.max(8, rect.left);
  }
  if (left + dropdownWidth > window.innerWidth - 8) {
    left = Math.max(8, window.innerWidth - dropdownWidth - 8);
  }
  schedViewDropdown.style.left = `${left}px`;
}

if (schedViewMenuBtn && schedViewDropdown) {
  schedViewMenuBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    const willShow = !schedViewDropdown.classList.contains('show');
    if (willShow) {
      positionSchedViewDropdown();
      schedViewDropdown.classList.add('show');
    } else {
      schedViewDropdown.classList.remove('show');
    }
  });

  schedViewDropdown.addEventListener('click', (e) => {
    e.stopPropagation();
  });

  window.addEventListener('click', () => {
    if (schedViewDropdown.classList.contains('show')) {
      schedViewDropdown.classList.remove('show');
    }
  });

  window.addEventListener('resize', () => {
    if (schedViewDropdown.classList.contains('show')) {
      positionSchedViewDropdown();
    }
  });

  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && schedViewDropdown.classList.contains('show')) {
      schedViewDropdown.classList.remove('show');
    }
  });
}

// Slider input
const schedSlider = document.getElementById('sched-view-slider');
if (schedSlider) {
  schedSlider.addEventListener('input', (e) => {
    setScheduleIconViewLevel((e.target as HTMLInputElement).value, false);
  });
  schedSlider.addEventListener('change', (e) => {
    setScheduleIconViewLevel((e.target as HTMLInputElement).value, true);
  });
}

// Tick and preset clicks
document.querySelectorAll<HTMLElement>('.sched-tick-label, .sched-view-btn').forEach(el => {
  el.addEventListener('click', (e) => {
    e.stopPropagation();
    const lvl = parseInt(el.dataset.level || '', 10);
    if (!isNaN(lvl)) {
      setScheduleIconViewLevel(lvl, true);
    }
  });
});

// Schedule Panel Action Buttons
on('btn-sched-collapse-all', 'click', () => {
  const sched: any = currentSnapshot && currentSnapshot.schedule ? currentSnapshot.schedule : null;
  const hasHeaders = sched && sched.items && sched.items.some((it: any) => {
    const t = (it.item_type || '').toLowerCase();
    return t === 'header' || t === 'group';
  });

  if (hasHeaders) {
    if (collapsedGroupIds.size > 0) {
      collapsedGroupIds.clear(); // Expand all groups
      showToast('✓ Expanded all groups', 'info');
    } else {
      // Collapse all groups
      sched.items.forEach((it: any) => {
        const t = (it.item_type || '').toLowerCase();
        if (t === 'header' || t === 'group') collapsedGroupIds.add(it.id);
      });
      setExpandedScheduleIndex(null);
      showToast('✓ Collapsed all groups', 'info');
    }
  } else {
    if (expandedScheduleIndex !== null) {
      setExpandedScheduleIndex(null); // collapse all
    } else {
      const selIdx = currentSnapshot && currentSnapshot.state ? currentSnapshot.state.selected_item_index : null;
      setExpandedScheduleIndex((selIdx !== null && selIdx !== undefined) ? selIdx : 0);
    }
  }
  resetLastRenderedScheduleKey();
  if (sched) renderSchedule(sched);
});
on('btn-sched-add', 'click', () => openSlideEditor('song'));
on('schedule-title-display', 'click', () => { resetSaveModal(); showModal(saveModal); });
on('schedule-title-wrap', 'click', () => { resetSaveModal(); showModal(saveModal); });

// Preview Panel Action Buttons
on('btn-preview-golive', 'click', () => sendCommand({ GoLive: { item_index: null, slide_index: null } }));
on('btn-preview-to-schedule', 'click', () => {
  if (selectedLibraryItem) {
    if (currentTab === 'scriptures' && isDualBibleMode) {
      const pVersion = selectedLibraryItem.version || (activeBibleVersion !== 'all' ? activeBibleVersion : (installedBibles[0] ? installedBibles[0].id : 'KJV'));
      addDualScriptureToSchedule(selectedLibraryItem, pVersion, secondaryBibleVersion, selectedLibraryItem.verses, currentDualSecondaryVerses);
    } else {
      addItemToSchedule(currentTab, selectedLibraryItem.id);
    }
  } else if (currentSnapshot && currentSnapshot.state && currentSnapshot.state.staged_item) {
    const staged = currentSnapshot.state.staged_item;
    addItemToSchedule('presentations', staged.id);
  }
});

// Preview View Mode Switching (Matrix vs List)
on('btn-preview-view-matrix', 'click', () => {
  const bM = document.getElementById('btn-preview-view-matrix');
  const bL = document.getElementById('btn-preview-view-list');
  if (bM) bM.classList.add('active');
  if (bL) bL.classList.remove('active');
  if (previewSlideMatrixEl) previewSlideMatrixEl.className = 'slide-matrix matrix-mode';
});
on('btn-preview-view-list', 'click', () => {
  const bM = document.getElementById('btn-preview-view-matrix');
  const bL = document.getElementById('btn-preview-view-list');
  if (bL) bL.classList.add('active');
  if (bM) bM.classList.remove('active');
  if (previewSlideMatrixEl) previewSlideMatrixEl.className = 'slide-matrix list-mode';
});

// Live Deck View Mode Switching (Matrix vs List)
on('btn-live-view-matrix', 'click', () => {
  setActiveLiveViewMode('matrix');
  const bM = document.getElementById('btn-live-view-matrix');
  const bL = document.getElementById('btn-live-view-list');
  if (bM) bM.classList.add('active');
  if (bL) bL.classList.remove('active');
  if (liveSlideMatrixEl) liveSlideMatrixEl.className = 'slide-matrix matrix-mode';
});
on('btn-live-view-list', 'click', () => {
  setActiveLiveViewMode('list');
  const bM = document.getElementById('btn-live-view-matrix');
  const bL = document.getElementById('btn-live-view-list');
  if (bL) bL.classList.add('active');
  if (bM) bM.classList.remove('active');
  if (liveSlideMatrixEl) liveSlideMatrixEl.className = 'slide-matrix list-mode';
});

// Resource View Mode Switching (Table vs Grid)
on('btn-resource-view-table', 'click', () => {
  setActiveResourceViewMode('table');
  const bT = document.getElementById('btn-resource-view-table');
  const bG = document.getElementById('btn-resource-view-grid');
  if (bT) bT.classList.add('active');
  if (bG) bG.classList.remove('active');
  filterAndRenderCatalog();
});
on('btn-resource-view-grid', 'click', () => {
  setActiveResourceViewMode('grid');
  const bT = document.getElementById('btn-resource-view-table');
  const bG = document.getElementById('btn-resource-view-grid');
  if (bG) bG.classList.add('active');
  if (bT) bT.classList.remove('active');
  filterAndRenderCatalog();
});

// Resource Add Button
on('btn-resource-add', 'click', () => {
  if (currentTab === 'themes') {
    openThemeEditor(null);
    return;
  }
  const typeMap: Record<string, string> = { songs: 'song', scriptures: 'scripture', presentations: 'presentation' };
  openSlideEditor(typeMap[currentTab] || 'song');
});

// Open FOH Screen Link
on('btn-open-foh-screen', 'click', async () => {
  try {
    window.open('/live.html', 'OpenSanctuaryFOH', 'width=1280,height=720,menubar=no,toolbar=no');
  } catch (e) {}

  try {
    const port = location.port ? parseInt(location.port, 10) : 8080;
    await fetch('/api/system/open-screen', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ screen: 'live', port: port })
    });
  } catch (e) {}

  showToast('🖥️ Opened Live FOH Projection Screen', 'info');
});

// Nursery & Alert Modal Handlers (F8)
on('btn-close-alert', 'click', () => closeModal(alertModal));
on('btn-close-alert-back', 'click', () => closeModal(alertModal));
on('btn-send-alert', 'click', () => {
  const msg = alertTextInput ? alertTextInput.value.trim() : '';
  sendCommand({ SetAlert: msg ? msg : null });
  closeModal(alertModal);
});
on('btn-clear-alert', 'click', () => {
  sendCommand({ SetAlert: null });
  if (alertTextInput) alertTextInput.value = '';
  closeModal(alertModal);
});
if (alertTextInput) {
  alertTextInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      const btnSend = document.getElementById('btn-send-alert');
      if (btnSend) btnSend.click();
    }
  });
}

// Open Schedule Modal Handlers
on('btn-close-open', 'click', () => closeModal(openModal));
on('btn-cancel-open', 'click', () => closeModal(openModal));

function getSelectedScheduleMode(): 'replace' | 'append' {
  const appendRadio = document.getElementById('sched-mode-append') as HTMLInputElement | null;
  return appendRadio?.checked ? 'append' : 'replace';
}

async function loadScheduleFile(file: File, mode: 'replace' | 'append') {
  showToast(`Loading ${file.name}...`, 'info');
  try {
    const data = await uploadScheduleFile(file, mode);
    setCurrentSnapshot(data);
    renderAll(data);
    if (mode === 'append') {
      showToast(`✓ Successfully appended items from '${file.name}' to schedule`, 'success');
    } else {
      showToast(`✓ Successfully loaded schedule '${data.schedule?.title || file.name}'`, 'success');
    }
    closeModal(openModal);
  } catch (err: any) {
    showToast(`Error loading schedule: ${err.message || err}`, 'error');
  }
}

on('btn-load-sample-ewsx', 'click', async () => {
  const mode = getSelectedScheduleMode();
  try {
    const res = await fetch('/test1.ewsx');
    if (!res.ok) throw new Error('Could not fetch test1.ewsx');
    const buffer = await res.arrayBuffer();
    const bytes = new Uint8Array(buffer);
    let binaryStr = '';
    const chunkSize = 8192;
    for (let i = 0; i < bytes.length; i += chunkSize) {
      binaryStr += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunkSize)));
    }
    const b64 = btoa(binaryStr);
    await resolveHostSessionToken();
    const apiRes = await fetch('/api/schedule/open', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', ...hostTokenHeader() },
      body: JSON.stringify({ file_name: 'test1.ewsx', file_data_base64: b64, mode })
    });
    if (apiRes.ok) {
      const data = await apiRes.json();
      setCurrentSnapshot(data);
      renderAll(data);
      showToast(mode === 'append' ? "✓ Appended EasyWorship sample schedule 'test1.ewsx' items" : "✓ Loaded EasyWorship sample schedule 'test1.ewsx'", 'success');
      closeModal(openModal);
    } else {
      const err = await apiRes.text();
      showToast(`Could not load sample: ${err}`, 'error');
    }
  } catch (err: any) {
    showToast(`Error loading test1.ewsx: ${err.message || err}`, 'error');
  }
});

on('btn-load-sunday-service', 'click', async () => {
  const mode = getSelectedScheduleMode();
  if (mode === 'replace') {
    sendCommand('NewSchedule');
  }
  try {
    const res = await fetch('/api/songs');
    if (res.ok) {
      const songs = await res.json();
      if (Array.isArray(songs) && songs.length > 0) {
        sendCommand({ AddToSchedule: { item_type: 'song', item_id: songs[0].id } });
        if (songs.length > 1) {
          sendCommand({ AddToSchedule: { item_type: 'song', item_id: songs[1].id } });
        }
      }
    }
  } catch (err) {
    console.warn('Error loading sunday service songs:', err);
  }
  showToast(mode === 'append' ? '✓ Appended Sunday Service schedule template' : '✓ Loaded Sunday Service schedule template', 'success');
  closeModal(openModal);
});

on('btn-load-communion-service', 'click', () => {
  const mode = getSelectedScheduleMode();
  if (mode === 'replace') {
    sendCommand('NewSchedule');
    showToast('✓ Created new Communion Service schedule', 'info');
  } else {
    showToast('✓ Retained schedule with Communion Service settings', 'info');
  }
  closeModal(openModal);
});

on('file-schedule-input', 'change', async (e: Event) => {
  const target = e.target as HTMLInputElement;
  const file = target.files?.[0];
  if (!file) return;
  const mode = getSelectedScheduleMode();
  try {
    await loadScheduleFile(file, mode);
  } finally {
    target.value = '';
  }
});

const scheduleDropzone = document.getElementById('schedule-dropzone');
if (scheduleDropzone) {
  scheduleDropzone.addEventListener('click', () => {
    const fInput = document.getElementById('file-schedule-input');
    if (fInput) fInput.click();
  });
  scheduleDropzone.addEventListener('dragover', (e) => {
    e.preventDefault();
    scheduleDropzone.classList.add('dragover-active');
  });
  scheduleDropzone.addEventListener('dragleave', () => {
    scheduleDropzone.classList.remove('dragover-active');
  });
  scheduleDropzone.addEventListener('drop', async (e: DragEvent) => {
    e.preventDefault();
    scheduleDropzone.classList.remove('dragover-active');
    const files = e.dataTransfer?.files;
    if (files && files.length > 0) {
      const mode = getSelectedScheduleMode();
      await loadScheduleFile(files[0], mode);
    }
  });
}

on('btn-open-schedule-hub', 'click', () => {
  closeModal(importModal);
  showModal(openModal);
});

// Setup native OS desktop file drag-and-drop on #schedule-panel
setupScheduleDesktopDrop({
  showToast,
  onLoaded: (snapshot) => {
    setCurrentSnapshot(snapshot);
    renderAll(snapshot);
  }
});

// EasyWorship/OpenLP schedule import now happens exclusively through the unified
// Open Schedule Hub (openModal, wired below near "Open Schedule Modal Handlers") —
// drag-and-drop or its own file picker, with an inline Replace/Append toggle. This
// used to be a second, separate path (a dedicated file input here that popped a
// second stacked "import-mode-modal" dialog to ask Replace/Append) — removed in
// favor of the one hub, reached from here via the "Open Schedule Hub" button.

// Save Schedule Modal Handlers
//
// "Save" (Ctrl+S / the ribbon Save button) and "Save As" (Ctrl+Shift+S) used
// to be literally identical — both just opened this same modal every time,
// with no quick re-save once a title/format was already chosen. Real
// "Save" now reuses the last title+format silently; "Save As" always opens
// the modal to pick a new one. The .ewsx branch used to download a bare
// JSON dump with a ".ewsx" extension stapled on — not a real EWSX file, so
// it wouldn't open in EasyWorship or via this app's own .ewsx-specific
// import path. It now downloads the real zip+SQLite package from
// GET /api/schedule/export (see src/api/routes.rs / src/storage/ewsx.rs).
// .osj (OpenLP's own JSON service-plan format) is genuinely JSON, so that
// branch is unchanged.
let lastSavedTitle: string | null = null;
let lastSavedFormat: string = 'ewsx';

async function performSave(title: string, format: string): Promise<void> {
  const ext = format === 'osj' ? 'osj' : 'ewsx';
  try {
    if (format === 'osj') {
      const dataStr = "data:text/json;charset=utf-8," + encodeURIComponent(JSON.stringify(currentSnapshot ? currentSnapshot.schedule : {}, null, 2));
      const downloadAnchor = document.createElement('a');
      downloadAnchor.setAttribute("href", dataStr);
      downloadAnchor.setAttribute("download", `${title}.${ext}`);
      document.body.appendChild(downloadAnchor);
      downloadAnchor.click();
      downloadAnchor.remove();
    } else {
      const res = await fetch(`/api/schedule/export?title=${encodeURIComponent(title)}`);
      if (!res.ok) {
        showToast(`Save failed: ${await res.text()}`, 'error');
        return;
      }
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const downloadAnchor = document.createElement('a');
      downloadAnchor.href = url;
      downloadAnchor.download = `${title}.${ext}`;
      document.body.appendChild(downloadAnchor);
      downloadAnchor.click();
      downloadAnchor.remove();
      URL.revokeObjectURL(url);
    }
  } catch (e: any) {
    showToast(`Save failed: ${e.message || e}`, 'error');
    return;
  }
  lastSavedTitle = title;
  lastSavedFormat = format;
  showToast(`✓ Saved "${title}" (.${ext})`, 'success');
}

// Ctrl+S / the ribbon Save button: reuse the last title+format silently;
// only fall back to the modal when nothing's been saved yet this session.
function quickSaveSchedule(): void {
  if (lastSavedTitle) {
    performSave(lastSavedTitle, lastSavedFormat);
  } else {
    resetSaveModal();
    showModal(saveModal);
  }
}

on('btn-close-save', 'click', () => closeModal(saveModal));
on('btn-cancel-save', 'click', () => closeModal(saveModal));
on('btn-confirm-save', 'click', async () => {
  const titleEl = document.getElementById('save-schedule-title') as HTMLInputElement | null;
  const title = (titleEl ? titleEl.value.trim() : '') || 'Schedule';
  const formatEl = document.getElementById('save-format-select') as HTMLSelectElement | null;
  const format = formatEl ? formatEl.value : 'ewsx';
  await performSave(title, format);
  closeModal(saveModal);
});

// Catalog Zoom Slider
on('catalog-zoom-slider', 'input', (e: Event) => {
  const sizeMap: Record<string, string> = { '1': '105px', '2': '140px', '3': '180px', '4': '225px' };
  const val = (e.target as HTMLInputElement).value;
  const grid = document.getElementById('catalog-grid');
  if (grid) {
    grid.style.gridTemplateColumns = `repeat(auto-fill, minmax(${sizeMap[val] || '140px'}, 1fr))`;
  }
});

// Store Modal Handlers
on('btn-close-store', 'click', () => closeModal(storeModal));
on('btn-close-store-footer', 'click', () => closeModal(storeModal));
on('btn-store-open-bibles', 'click', () => {
  closeModal(storeModal);
  openImportModal();
});

// Web Streamer Modal Handlers
function normalizeWebStreamUrl(rawUrl: string) {
  try {
    const u = new URL(rawUrl);
    const host = u.hostname.replace(/^www\./, '').replace(/^m\./, '');
    if (host === 'youtube.com' || host === 'youtu.be') {
      let videoId = null;
      if (host === 'youtu.be') {
        videoId = u.pathname.slice(1).split('/')[0];
      } else if (u.pathname === '/watch') {
        videoId = u.searchParams.get('v');
      } else if (u.pathname.startsWith('/live/')) {
        videoId = u.pathname.split('/')[2];
      } else if (u.pathname.startsWith('/embed/')) {
        return rawUrl;
      }
      if (videoId) {
        return `https://www.youtube-nocookie.com/embed/${videoId}?autoplay=1&mute=1&controls=0&rel=0`;
      }
    }
    return rawUrl;
  } catch (_) {
    return rawUrl;
  }
}
on('btn-close-web', 'click', () => closeModal(webModal));
on('btn-close-web-footer', 'click', () => closeModal(webModal));
on('btn-stop-web-stream', 'click', () => {
  sendCommand({ SetWebStream: null });
  showToast('Web stream stopped', 'info');
  closeModal(webModal);
});
on('btn-apply-web-stream', 'click', () => {
  const urlEl = document.getElementById('web-stream-url') as HTMLInputElement | null;
  const modeEl = document.getElementById('web-overlay-mode') as HTMLSelectElement | null;
  const url = urlEl ? urlEl.value.trim() : '';
  const mode = modeEl ? modeEl.value : 'background';
  if (!url) {
    showToast('Enter a web page or YouTube URL first', 'warning');
    return;
  }
  const embedUrl = normalizeWebStreamUrl(url);
  sendCommand({ SetWebStream: { url: embedUrl, mode } });
  showToast(`Projecting web stream (${mode} mode)`, 'success');
  closeModal(webModal);
});

// Remote Modal Handlers
on('btn-close-remote', 'click', () => closeModal(remoteModal));
on('btn-close-remote-footer', 'click', () => closeModal(remoteModal));

export async function updateRemoteQrCode(): Promise<string> {
  const disp = document.getElementById('remote-url-display');
  const canvas = document.getElementById('remote-qr-canvas') as HTMLCanvasElement | null;
  const openBtn = document.getElementById('btn-open-mobile-remote') as HTMLAnchorElement | null;
  let targetUrl = buildRemoteUrl({}, location);

  try {
    const res = await fetch('/api/network/info');
    if (res.ok) {
      const data = await res.json();
      targetUrl = buildRemoteUrl(data, location);
    }
  } catch (_) {}

  // Mint a fresh paired-device token for this QR — /remote requires it (see
  // remote_client.ts) so a phone can't reach live control just by guessing
  // the URL; it has to actually scan this code. Carried in the URL fragment
  // (not a query param) so it never gets sent to the server in a Referer
  // header or logged by a naive access log. Console-only server-side —
  // api.pairing.remoteSession() attaches this page's host token
  // automatically.
  try {
    const pairData = await api.pairing.remoteSession();
    if (pairData && pairData.token) {
      targetUrl = `${targetUrl}#token=${encodeURIComponent(pairData.token)}`;
    }
  } catch (_) {}

  if (disp) disp.textContent = targetUrl;
  if (openBtn) openBtn.href = targetUrl;

  if (canvas) {
    try {
      await QRCode.toCanvas(canvas, targetUrl, {
        width: 220,
        margin: 1,
        color: {
          dark: '#000000',
          light: '#ffffff'
        },
        errorCorrectionLevel: 'M'
      });
    } catch (err) {
      console.error('Failed to render QR code canvas:', err);
    }
  }
  return targetUrl;
}
try { globalThis.updateRemoteQrCode = updateRemoteQrCode; } catch (_) {}

// Modal Tabs: Mobile Remote vs Pair TV App vs Install via ADB
const REMOTE_MODAL_TABS = [
  { tabId: 'tab-remote-control', panelId: 'panel-remote-control' },
  { tabId: 'tab-pair-tv', panelId: 'panel-pair-tv' },
  { tabId: 'tab-adb-provision', panelId: 'panel-adb-provision' },
];

function activateRemoteModalTab(activeTabId: string) {
  for (const { tabId, panelId } of REMOTE_MODAL_TABS) {
    const tab = document.getElementById(tabId);
    const panel = document.getElementById(panelId);
    if (!tab || !panel) continue;
    const active = tabId === activeTabId;
    tab.style.background = active ? 'var(--os-accent, #ff5722)' : 'transparent';
    tab.style.color = active ? '#ffffff' : 'var(--text-muted)';
    panel.style.display = active ? 'block' : 'none';
  }
}

export function switchToRemoteTab() {
  activateRemoteModalTab('tab-remote-control');
}

export function switchToPairingTab() {
  activateRemoteModalTab('tab-pair-tv');
  updatePairingQrCode();
}

export function switchToAdbProvisionTab() {
  activateRemoteModalTab('tab-adb-provision');
  initAdbProvisionPanel();
}
try { globalThis.switchToRemoteTab = switchToRemoteTab; } catch (_) {}

on('tab-remote-control', 'click', switchToRemoteTab);
on('tab-pair-tv', 'click', switchToPairingTab);
on('tab-adb-provision', 'click', switchToAdbProvisionTab);

let activePairingSessionToken: string | null = null;

export async function updatePairingQrCode(forceNew: boolean = false): Promise<string> {
  const disp = document.getElementById('pairing-url-display');
  const canvas = document.getElementById('pairing-qr-canvas') as HTMLCanvasElement | null;
  const openBtn = document.getElementById('btn-open-pairing-tool') as HTMLAnchorElement | null;

  let netInfo: any = {};
  try {
    const res = await fetch('/api/network/info');
    if (res.ok) {
      netInfo = await res.json();
    }
  } catch (_) {}

  if (forceNew || !activePairingSessionToken) {
    try {
      const sess = await api.pairing.createSession();
      activePairingSessionToken = sess.session_token;
    } catch (err) {
      console.error('Failed to create pairing session:', err);
    }
  }

  const targetUrl = buildPairingUrl(netInfo, location, activePairingSessionToken);

  if (disp) disp.textContent = targetUrl;
  if (openBtn) openBtn.href = targetUrl;

  if (canvas) {
    try {
      await QRCode.toCanvas(canvas, targetUrl, {
        width: 220,
        margin: 1,
        color: {
          dark: '#000000',
          light: '#ffffff'
        },
        errorCorrectionLevel: 'M'
      });
    } catch (err) {
      console.error('Failed to render pairing QR canvas:', err);
    }
  }
  return targetUrl;
}
try { globalThis.updatePairingQrCode = updatePairingQrCode; } catch (_) {}

on('btn-refresh-pairing-session', 'click', () => {
  updatePairingQrCode(true);
  showToast('New pairing session generated', 'info');
});

on('btn-copy-pairing-url', 'click', async () => {
  const disp = document.getElementById('pairing-url-display');
  if (disp && disp.textContent) {
    try {
      await navigator.clipboard.writeText(disp.textContent.trim());
      showToast('Pairing URL copied to clipboard!', 'success');
    } catch (_) {
      showToast('Could not copy URL', 'warning');
    }
  }
});

// Install via ADB (docs/CLIENT_PAIRING.md's ADB-driven sideload flow) — the
// third tab of #remote-modal, alongside Mobile Remote / Pair TV App. Same
// POST-to-start/GET-to-poll/`is_complete` shape as the yt-dlp background
// import flow (see ytdlpPollingInterval in this file), not a new pattern.
let adbProvisionPollingInterval: ReturnType<typeof setInterval> | null = null;
let adbProvisionStatusChecked = false;

function stopAdbProvisionPolling() {
  if (adbProvisionPollingInterval) {
    clearInterval(adbProvisionPollingInterval);
    adbProvisionPollingInterval = null;
  }
}

async function initAdbProvisionPanel() {
  const startBtn = document.getElementById('btn-adb-provision-start') as HTMLButtonElement | null;
  const statusEl = document.getElementById('adb-provision-status');
  const warningEl = document.getElementById('adb-provision-unavailable');
  if (adbProvisionStatusChecked) return;
  adbProvisionStatusChecked = true;

  try {
    const status = await api.tvProvision.status();
    if (!status.adb_available || !status.apk_available) {
      if (warningEl) {
        warningEl.style.display = 'block';
        warningEl.textContent = !status.adb_available
          ? '⚠️ adb was not found on this machine. Install Android platform-tools to use this feature.'
          : '⚠️ No Android TV build is bundled with this install.';
      }
      if (startBtn) startBtn.disabled = true;
    }
  } catch (_) {
    if (statusEl) statusEl.textContent = 'Could not check ADB availability.';
  }
}

async function startAdbProvisioning() {
  const ipInput = document.getElementById('adb-provision-ip') as HTMLInputElement | null;
  const stageCheckbox = document.getElementById('adb-provision-stage-mode') as HTMLInputElement | null;
  const nameInput = document.getElementById('adb-provision-name') as HTMLInputElement | null;
  const startBtn = document.getElementById('btn-adb-provision-start') as HTMLButtonElement | null;
  const statusEl = document.getElementById('adb-provision-status');

  const ip = ipInput ? ipInput.value.trim() : '';
  if (!ip) {
    showToast('Enter the TV\'s IP address first.', 'warning');
    return;
  }

  if (startBtn) startBtn.disabled = true;
  if (statusEl) statusEl.innerHTML = '<span style="color: #00e5ff;">⏳ Starting…</span>';

  try {
    const startResult = await api.tvProvision.start({
      ip,
      is_stage_mode: stageCheckbox ? stageCheckbox.checked : false,
      device_name: nameInput && nameInput.value.trim() ? nameInput.value.trim() : undefined,
    });
    if (!startResult || !startResult.task_id) {
      throw new Error('Server did not return a task id for TV provisioning.');
    }
    const { task_id } = startResult;

    stopAdbProvisionPolling();
    let failCount = 0;
    adbProvisionPollingInterval = setInterval(async () => {
      try {
        const progressResult = await api.tvProvision.progress(task_id);
        failCount = 0;
        if (!progressResult || !progressResult.progress) return;
        const { progress } = progressResult;

        if (statusEl) {
          const color = progress.status === 'failed' ? '#ff5252' : progress.status === 'completed' ? '#00e676' : '#00e5ff';
          statusEl.innerHTML = `<span style="color: ${color};">${escapeHtml(progress.message)}</span>`;
        }

        if (progress.is_complete) {
          stopAdbProvisionPolling();
          if (startBtn) startBtn.disabled = false;
          if (progress.status === 'completed') {
            showToast(progress.message, 'success');
            // Refresh the Paired Devices list behind this modal so the
            // newly self-authorized TV shows up without a manual re-open.
            refreshPairedDevicesAfterAdbProvision();
          } else if (progress.error) {
            showToast(progress.error, 'error');
          }
        }
      } catch (_) {
        failCount++;
        if (failCount >= 5) {
          stopAdbProvisionPolling();
          if (startBtn) startBtn.disabled = false;
          if (statusEl) statusEl.innerHTML = '<span style="color: #ff5252;">❌ Lost connection while checking progress.</span>';
        }
      }
    }, 1500);
  } catch (err: any) {
    if (startBtn) startBtn.disabled = false;
    const message = (err && err.message) || 'Could not start provisioning.';
    if (statusEl) statusEl.innerHTML = `<span style="color: #ff5252;">❌ ${escapeHtml(message)}</span>`;
  }
}

on('btn-adb-provision-start', 'click', startAdbProvisioning);

// Settings Modal Handlers (Edit > Options) — rendering is schema-driven (see
// renderSettingsSidebar/renderSettingsContent in app_core.ts); this just wires the
// search box and the generic Save that reads whatever the schema rendered.
on('settings-search', 'input', () => {
  const el = document.getElementById('settings-search') as HTMLInputElement | null;
  onSettingsSearchInput(el ? el.value : '');
});
on('btn-close-options', 'click', () => closeModal(optionsModal));
on('btn-options-back', 'click', handleOptionsBack);
on('btn-cancel-options', 'click', () => closeModal(optionsModal));
on('btn-save-options', 'click', () => {
  const newOpts: Record<string, string> = {};
  SETTINGS_SCHEMA.forEach(def => {
    if (def.control === 'readonly' || def.control === 'action' || def.control === 'panel') return;
    const el = document.getElementById(`setting-${def.key}`) as HTMLInputElement | HTMLSelectElement | null;
    if (!el) return;
    const value = 'value' in el ? el.value : '';
    if (def.control === 'password') {
      newOpts[def.key] = value.trim();
    } else if (value) {
      newOpts[def.key] = value;
    }
  });
  saveAppOptions(newOpts);
  if (typeof saveNetworkSettings === 'function') {
    saveNetworkSettings();
  }
  closeModal(optionsModal);
});

// Shortcuts, About & Guide Modal Handlers
on('btn-close-shortcuts', 'click', () => closeModal(shortcutsModal));
on('btn-close-shortcuts-footer', 'click', () => closeModal(shortcutsModal));
on('btn-close-about', 'click', () => closeModal(aboutModal));
on('btn-close-about-footer', 'click', () => closeModal(aboutModal));
on('btn-close-articles', 'click', () => closeModal(scheduleArticlesModal));
on('btn-close-articles-footer', 'click', () => closeModal(scheduleArticlesModal));

const ARTICLES_DATA: Record<string, string> = {
  'article-overview': `
    <h2 style="color: #ffa726; margin-top: 0; display: flex; align-items: center; gap: 8px;">
      <span>🌟</span> 1. Schedule & Worship "Set List" Overview
    </h2>
    <p style="font-size: 14px; line-height: 1.6;">
      The <strong>Schedule Area</strong> in OpenSanctuary is the operational heart of your service. It acts as your dynamic playlist or "set list", organizing everything you plan to present on the main Front-of-House (FOH) projection display and Stage Confidence monitors during worship.
    </p>
    <div style="background: rgba(255, 167, 38, 0.08); border-left: 4px solid var(--os-brand-amber); padding: 14px 18px; border-radius: 4px; margin: 16px 0;">
      <strong style="font-size: 13.5px; color: #fff;">Supported Worship Schedule Items:</strong>
      <ul style="margin: 10px 0 0 18px; padding: 0; line-height: 1.7;">
        <li><strong>🎵 Songs with Backgrounds</strong> — Lyrics with verse tags (V1, C1, B1), motion video loops, photo backgrounds, and customizable themes.</li>
        <li><strong>📖 Scripture Passages</strong> — Specific single verses, multi-verse passages, and side-by-side Dual Translation comparison mode.</li>
        <li><strong>📊 PowerPoint & Sermon Slides</strong> — Multi-slide sermon outlines, bullet points, and imported presentation decks.</li>
        <li><strong>📡 Live Camera Feeds</strong> — Direct USB webcams, PTZ sanctuary cameras, and RTSP/NDI feeds with overlayed lyrics.</li>
        <li><strong>🎥 Mini-Movies & Video Countdowns</strong> — Offering mini-movies, sermon illustrations, and mission highlights with frame-accurate sync.</li>
        <li><strong>🌐 Websites & Live Web Streams</strong> — Live YouTube broadcasts, online interactive scripture portals, and web stream overlays.</li>
        <li><strong>🏷️ Section / Group Headers</strong> — Liturgical service sections (<em>Praise & Worship, Scripture Reading, Sermon, Offering, Benediction</em>) with collapsible item counts.</li>
      </ul>
    </div>
  `,

  'article-building': `
    <h2 style="color: #ffa726; margin-top: 0; display: flex; align-items: center; gap: 8px;">
      <span>📋</span> 2. Building Your First Schedule
    </h2>
    <p>Creating a schedule in OpenSanctuary is fast and intuitive. You can build your service order using multiple quick workflows:</p>
    
    <h3 style="color: #fff; margin-top: 18px;">Method 1: From the Catalog Library (Double-Click / ➕ / Drag-and-Drop)</h3>
    <ol style="margin-left: 18px; line-height: 1.7;">
      <li>Select any resource tab in the center panel (<strong>Songs, Scriptures, Media, Presentations</strong>).</li>
      <li>Search for your title, lyrics, or Bible reference (e.g. <code>Amazing Grace</code>, <code>John 3:16</code>, <code>Sunday Welcome</code>).</li>
      <li>Click the <strong>➕ Add to Schedule</strong> button, double-click the row, or drag the card directly into the Schedule list on the left.</li>
    </ol>

    <h3 style="color: #fff; margin-top: 18px;">Method 2: From the Top "New ▾" Ribbon Dropdown</h3>
    <ul style="margin-left: 18px; line-height: 1.7;">
      <li>Click <strong>New ▾ -> 🎵 New Song...</strong> (<kbd>Ctrl+Shift+N</kbd>) to author a custom song.</li>
      <li>Click <strong>New ▾ -> 📊 New Presentation / Sermon Slides...</strong> to design sermon teaching points in the Creator Studio.</li>
      <li>Click <strong>New ▾ -> 📖 New Scripture Passage...</strong> to lookup and stage specific verses.</li>
      <li>Click <strong>New ▾ -> 🎥 New Video / Mini-Movie...</strong> to stage an offering countdown or sermon video.</li>
      <li>Click <strong>New ▾ -> 📡 New Live Camera Feed...</strong> to stage live webcam or sanctuary PTZ video.</li>
      <li>Click <strong>New ▾ -> 🏷️ New Section / Group Header...</strong> to organize your service sections.</li>
    </ul>

    <h3 style="color: #fff; margin-top: 18px;">Method 3: Staging & Previewing Before Going Live</h3>
    <p>
      Clicking any item in the Schedule stages it in the <strong>Preview Deck</strong> (center-right). You can inspect all slide tiles, verify lyrics and background contrast, and then hit <kbd>Enter</kbd> or click <strong>▶ GO LIVE</strong> when your pastor or worship leader is ready!
    </p>
  `,

  'article-content-types': `
    <h2 style="color: #ffa726; margin-top: 0; display: flex; align-items: center; gap: 8px;">
      <span>🎬</span> 3. Songs, Scriptures, Feeds & Media
    </h2>
    
    <h3 style="color: #fff; margin-top: 14px;">1. Songs with Dynamic Backgrounds</h3>
    <p>
      Every song can use default global themes or custom per-song / per-slide backgrounds. Right-click any schedule item and choose <strong>🎨 Change Theme / Background...</strong> or <strong>🖼 Select Background Photo...</strong> to apply scenic mountains, motion video loops, or subtle dark gradients.
    </p>

    <h3 style="color: #fff; margin-top: 14px;">2. Scriptures & Dual-Translation Comparison</h3>
    <p>
      Type single verses (<code>Philippians 4:6</code>) or ranges (<code>Romans 8:28-30</code>) to add precisely those verses to your schedule. Toggle <strong>👥 Compare</strong> to project two translations side-by-side (e.g. KJV and ASV or WEB).
    </p>

    <h3 style="color: #fff; margin-top: 14px;">3. PowerPoint & Sermon Presentations</h3>
    <p>
      Build rich sermon slides using the built-in Presentation Studio. Choose between Title & Body, 2 Columns, Scripture Quote, or Bullet List layouts. Import slide decks and presentations from PowerPoint (<code>.pptx</code>), FreeShow (<code>.show</code>), or OpenLP via the <strong>📥 Import</strong> menu — FreeShow brings in rich text formatting, arrangement sections, and backgrounds.
    </p>

    <h3 style="color: #fff; margin-top: 14px;">4. Live Camera Feeds (PTZ & Webcams)</h3>
    <p>
      Select <strong>Media -> 📡 Live Camera Feeds</strong> or <strong>New ▾ -> 📡 New Live Camera Feed...</strong> to stream local capture devices and USB webcams straight to the sanctuary screen, with worship lyrics rendered cleanly over the live video!
    </p>

    <h3 style="color: #fff; margin-top: 14px;">5. Mini-Movies & Sermon Illustrations</h3>
    <p>
      Add videos directly to the schedule for countdowns, offering mini-movies, or sermon illustrations. Supports full playback transport controls (Play, Pause, Scrub bar, Loop toggle, Volume). Download web clips directly via the built-in <strong>📥 yt-dlp Video Importer</strong>.
    </p>

    <h3 style="color: #fff; margin-top: 14px;">6. Websites & Live Web Streams</h3>
    <p>
      Use the <strong>🌐 Web</strong> button to embed live stream links, church announcements, or YouTube broadcasts into your worship presentation.
    </p>
  `,

  'article-reordering': `
    <h2 style="color: #ffa726; margin-top: 0; display: flex; align-items: center; gap: 8px;">
      <span>⠿</span> 4. Editing, Re-ordering & Organizing Items
    </h2>

    <h3 style="color: #fff; margin-top: 14px;">1. Drag-and-Drop Schedule Re-ordering</h3>
    <p>
      Grab the <strong>⠿ drag handle</strong> on any schedule item or section header and drag it up or down to change the service sequence. The schedule reorders smoothly in real time.
    </p>

    <h3 style="color: #fff; margin-top: 14px;">2. Section Headers & Collapsible Groups</h3>
    <p>
      Insert Section Headers (e.g. <code>🏷️ PRAISE & WORSHIP</code>, <code>🏷️ SERMON</code>, <code>🏷️ OFFERING</code>). Clicking any header expands or collapses all child items underneath it, displaying an item count badge (e.g. <em>(4 items)</em>) to keep large worship services tidy and organized.
    </p>

    <h3 style="color: #fff; margin-top: 14px;">3. Verse & Slide-Level Reordering</h3>
    <p>
      Click the <strong>▶ caret</strong> next to any multi-slide song to reveal its individual verses (<em>Verse 1, Chorus, Verse 2, Bridge</em>). Drag any slide up or down using the child drag handle to rearrange verses on the fly without editing the original song in your library!
    </p>

    <h3 style="color: #fff; margin-top: 14px;">4. Right-Click Context Menu Actions</h3>
    <p>
      Right-click any schedule item for instant management options:
    </p>
    <ul style="margin-left: 18px; line-height: 1.7;">
      <li><strong>✏️ Edit Item & Slides...</strong> — Open the slide studio to modify text or formatting.</li>
      <li><strong>📋 Duplicate Item</strong> — Duplicate an item for responsive call-and-response liturgy.</li>
      <li><strong>🗑️ Remove from Schedule</strong> — Remove the item from the current service queue.</li>
      <li><strong><span data-no-iconify>⬆ Move Up / ⬇ Move Down</span></strong> — Single-step keyboard/mouse shifting.</li>
    </ul>
  `,

  'article-saving': `
    <h2 style="color: #ffa726; margin-top: 0; display: flex; align-items: center; gap: 8px;">
      <span>💾</span> 5. Saving, Exporting & Sharing Schedules
    </h2>
    <p>
      OpenSanctuary schedules are fully self-contained portable files designed for seamless sharing across church production teams and computers.
    </p>

    <h3 style="color: #fff; margin-top: 14px;">1. Native EasyWorship 7 (.ewsx) Standard Package Format</h3>
    <p>
      When you save a schedule (<kbd>Ctrl+S</kbd> or <strong>File -> Save Schedule</strong>), OpenSanctuary writes a standard <code>.ewsx</code> zip bundle containing:
    </p>
    <ul style="margin-left: 18px; line-height: 1.7;">
      <li>All schedule items, slide texts, and verse tags.</li>
      <li>Embedded song records and scripture references.</li>
      <li>Embedded background images, photos, and theme styling.</li>
      <li>Section header structure and grouping states.</li>
    </ul>

    <h3 style="color: #fff; margin-top: 14px;">2. Sharing with Other Operators & Computers</h3>
    <p>
      To share a schedule with your pastor, sound booth team, or a backup laptop:
    </p>
    <ol style="margin-left: 18px; line-height: 1.7;">
      <li>Click <strong>Save As...</strong> and save your schedule (e.g. <code>Sunday_Morning_Worship.ewsx</code>).</li>
      <li>Copy the <code>.ewsx</code> file to a USB thumb drive, church cloud folder, or email.</li>
      <li>On the sanctuary presentation computer, click <strong>📂 Open</strong> (or drag and drop the <code>.ewsx</code> file directly into OpenSanctuary). All songs, slides, and backgrounds load instantly!</li>
    </ol>

    <h3 style="color: #fff; margin-top: 14px;">3. Exporting to OpenLP (.osj) Format</h3>
    <p>
      Need to share with another church using OpenLP? Use <strong>File -> Export Schedule -> OpenLP Service (.osj)</strong> for cross-platform compatibility.
    </p>
  `,

  'article-live-ops': `
    <h2 style="color: #ffa726; margin-top: 0; display: flex; align-items: center; gap: 8px;">
      <span>⚡</span> 6. Live Service Operations, Hotkeys & Stage Monitors
    </h2>
    
    <h3 style="color: #fff; margin-top: 14px;">1. Emergency Live Override Hotkeys</h3>
    <div style="display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 10px; margin: 12px 0;">
      <div style="background: rgba(255,255,255,0.05); padding: 10px; border-radius: 4px; border: 1px solid var(--border-color);">
        <strong style="color: #ef5350;">⬛ Blackout (<kbd>F5</kbd>)</strong><br>
        <span style="font-size: 11px; color: var(--text-dim);">Instantly blacks out the screen.</span>
      </div>
      <div style="background: rgba(255,255,255,0.05); padding: 10px; border-radius: 4px; border: 1px solid var(--border-color);">
        <strong style="color: #42a5f5;">🧹 Clear Text (<kbd>F6</kbd>)</strong><br>
        <span style="font-size: 11px; color: var(--text-dim);">Hides lyrics, keeps background.</span>
      </div>
      <div style="background: rgba(255,255,255,0.05); padding: 10px; border-radius: 4px; border: 1px solid var(--border-color);">
        <strong style="color: #ab47bc;">🏛 Logo Screen (<kbd>F7</kbd>)</strong><br>
        <span style="font-size: 11px; color: var(--text-dim);">Displays church logo screen.</span>
      </div>
      <div style="background: rgba(255,255,255,0.05); padding: 10px; border-radius: 4px; border: 1px solid var(--border-color);">
        <strong style="color: #ffa726;">{icon:alert} Alert Banner (<kbd>F8</kbd>)</strong><br>
        <span style="font-size: 11px; color: var(--text-dim);">Nursery & parking emergency alerts.</span>
      </div>
    </div>

    <h3 style="color: #fff; margin-top: 14px;">2. Stage Foldback & Confidence Monitor</h3>
    <p>
      Musicians and speakers on stage can view the <strong>Stage Foldback Monitor</strong> by opening <code>/stage.html</code> in any browser or on a dedicated stage display. It shows large high-contrast current lyrics, NEXT slide preview, and real-time clock.
    </p>

    <h3 style="color: #fff; margin-top: 14px;">3. Mobile Remote Control</h3>
    <p>
      Worship leaders and pastors can change slides from their smartphone or tablet on the church Wi-Fi using the mobile-optimized web console at <code>http://[your-ip]:8080/</code>.
    </p>
  `
};

function renderArticle(articleId: string) {
  const content = ARTICLES_DATA[articleId] || ARTICLES_DATA['article-overview'];
  const pane = document.getElementById('articles-content-pane');
  if (pane) {
    pane.innerHTML = content;
    pane.scrollTop = 0;
  }
  document.querySelectorAll<HTMLElement>('.article-nav-item').forEach(item => {
    if (item.dataset.article === articleId) {
      item.classList.add('active');
      item.style.color = '#fff';
      item.style.borderLeft = '3px solid var(--os-brand-amber)';
      item.style.background = 'rgba(255, 167, 38, 0.08)';
    } else {
      item.classList.remove('active');
      item.style.color = 'var(--text-muted)';
      item.style.borderLeft = '3px solid transparent';
      item.style.background = 'transparent';
    }
  });
}

function openScheduleGuideModal(initialArticle = 'article-overview') {
  renderArticle(initialArticle);
  showModal(scheduleArticlesModal);
}

document.querySelectorAll<HTMLElement>('.article-nav-item').forEach(navEl => {
  navEl.addEventListener('click', () => {
    const artId = navEl.dataset.article;
    if (artId) renderArticle(artId);
  });
});

// ============================================================================
// FREESHOW-STYLE BIBLE IMPORTER MODAL
// ============================================================================
const tabBtnApi = document.getElementById('tab-btn-api');
const tabBtnGithub = document.getElementById('tab-btn-github');
const tabBtnYtdlp = document.getElementById('tab-btn-ytdlp');
const tabBtnLocal = document.getElementById('tab-btn-local');
const importViewApi = document.getElementById('import-view-api');
const importViewGithub = document.getElementById('import-view-github');
const importViewYtdlp = document.getElementById('import-view-ytdlp');
const importViewLocal = document.getElementById('import-view-local');
const apiBibleCatalogList = document.getElementById('api-bible-catalog-list');
const apiBibleSearchInput = document.getElementById('api-bible-search-input') as HTMLInputElement | null;
const apiBibleLangFilter = document.getElementById('api-bible-lang-filter') as HTMLInputElement | null;
const apiBibleCountLabel = document.getElementById('api-bible-count-label');

const ghRepoInput = document.getElementById('gh-repo-input') as HTMLInputElement | null;
const ghVersionsPathInput = document.getElementById('gh-versions-path-input') as HTMLInputElement | null;
const btnGhBrowse = document.getElementById('btn-gh-browse');
const ghBibleCountLabel = document.getElementById('gh-bible-count-label');
const ghBibleSourceLabel = document.getElementById('gh-bible-source-label');
const ghBibleCatalogList = document.getElementById('gh-bible-catalog-list');

const ytdlpUrlInput = document.getElementById('ytdlp-url-input') as HTMLInputElement | null;
const btnYtdlpPaste = document.getElementById('btn-ytdlp-paste') as HTMLButtonElement | null;
const ytdlpBrowserSelect = document.getElementById('ytdlp-browser-select') as HTMLInputElement | null;
const ytdlpAudioOnlyCheckbox = document.getElementById('ytdlp-audio-only-checkbox') as HTMLInputElement | null;
const ytdlpSponsorblockCheckbox = document.getElementById('ytdlp-sponsorblock-checkbox') as HTMLInputElement | null;
const btnYtdlpStartDownload = document.getElementById('btn-ytdlp-start-download') as HTMLButtonElement | null;
const btnYtdlpDownloadAndLive = document.getElementById('btn-ytdlp-download-and-live') as HTMLButtonElement | null;
const ytdlpDownloadStatus = document.getElementById('ytdlp-download-status');
const ytdlpProgressContainer = document.getElementById('ytdlp-progress-container');
const ytdlpProgressFill = document.getElementById('ytdlp-progress-fill');

if (btnYtdlpPaste) {
  btnYtdlpPaste.addEventListener('click', async () => {
    let pastedText = '';

    // 1. Try Desktop Backend Clipboard API (works directly on system clipboard)
    try {
      const res = await fetch('/api/system/clipboard');
      if (res.ok) {
        const data = await res.json();
        if (data.text && data.text.trim()) {
          pastedText = data.text.trim();
        }
      }
    } catch (e) {
      // ignore
    }

    // 2. Fallback to navigator.clipboard.readText()
    if (!pastedText && navigator.clipboard && navigator.clipboard.readText) {
      try {
        const text = await navigator.clipboard.readText();
        if (text && text.trim()) {
          pastedText = text.trim();
        }
      } catch (e) {
        // ignore
      }
    }

    // 3. If pastedText was obtained, populate and provide visual confirmation
    if (pastedText) {
      if (ytdlpUrlInput) {
        ytdlpUrlInput.value = pastedText;
        ytdlpUrlInput.focus();
        btnYtdlpPaste.textContent = '✓ Pasted!';
        setTimeout(() => {
          btnYtdlpPaste.textContent = '📋 Paste';
        }, 1200);
      }
      return;
    }

    // 4. Fallback: focus input and prompt user to press Ctrl+V
    if (ytdlpUrlInput) {
      ytdlpUrlInput.focus();
      ytdlpUrlInput.select();
      if (ytdlpDownloadStatus) {
        ytdlpDownloadStatus.innerHTML = `<span style="color: #00e5ff;">📋 Please press <strong>Ctrl+V</strong> to paste your URL.</span>`;
        setTimeout(() => {
          if (ytdlpDownloadStatus && ytdlpDownloadStatus.textContent.includes('Ctrl+V')) {
            ytdlpDownloadStatus.innerHTML = '';
          }
        }, 3000);
      }
    }
  });
}

function openImportModal(initialMode = 'api') {
  if (importModal) showModal(importModal);
  resetImportModal();
  if (initialMode) setImportMode(initialMode);
  loadOnlineBibleCatalog();
}

function setImportMode(mode: string) {
  setActiveImportMode(mode);
  if (tabBtnApi) tabBtnApi.classList.toggle('active', mode === 'api');
  if (tabBtnGithub) tabBtnGithub.classList.toggle('active', mode === 'github');
  if (tabBtnYtdlp) tabBtnYtdlp.classList.toggle('active', mode === 'ytdlp');
  if (tabBtnLocal) tabBtnLocal.classList.toggle('active', mode === 'local');
  if (importViewApi) importViewApi.style.display = mode === 'api' ? 'flex' : 'none';
  if (importViewGithub) importViewGithub.style.display = mode === 'github' ? 'flex' : 'none';
  if (importViewYtdlp) importViewYtdlp.style.display = mode === 'ytdlp' ? 'flex' : 'none';
  if (importViewLocal) importViewLocal.style.display = mode === 'local' ? 'flex' : 'none';
}

if (tabBtnApi) on(tabBtnApi, 'click', () => setImportMode('api'));
if (tabBtnGithub) on(tabBtnGithub, 'click', () => {
  setImportMode('github');
  if (ghBibleCatalogList && (!ghBibleCatalogList.children || ghBibleCatalogList.children.length <= 1) && ghRepoInput && ghRepoInput.value.trim()) {
    browseGitHubBibleRepo();
  }
});
if (tabBtnYtdlp) on(tabBtnYtdlp, 'click', () => setImportMode('ytdlp'));
if (tabBtnLocal) on(tabBtnLocal, 'click', () => setImportMode('local'));

let ytdlpPollingInterval: ReturnType<typeof setInterval> | null = null;

function stopYtdlpPolling() {
  if (ytdlpPollingInterval) {
    clearInterval(ytdlpPollingInterval);
    ytdlpPollingInterval = null;
  }
}

async function handleYtDlpDownload(goLiveAfter = false) {
  const url = ytdlpUrlInput ? ytdlpUrlInput.value.trim() : '';
  if (!url) {
    if (ytdlpDownloadStatus) {
      ytdlpDownloadStatus.innerHTML = `<span style="color: #ff5252;">⚠️ Please enter or paste a video URL (e.g. YouTube, Vimeo, direct link).</span>`;
    }
    if (ytdlpUrlInput) ytdlpUrlInput.focus();
    return;
  }

  const browser = ytdlpBrowserSelect ? ytdlpBrowserSelect.value : 'auto';
  const isAudio = ytdlpAudioOnlyCheckbox ? ytdlpAudioOnlyCheckbox.checked : false;
  const sponsorblock = ytdlpSponsorblockCheckbox ? ytdlpSponsorblockCheckbox.checked : true;

  if (ytdlpProgressContainer) ytdlpProgressContainer.style.display = 'block';
  if (ytdlpProgressFill) ytdlpProgressFill.style.width = '0%';

  if (ytdlpDownloadStatus) {
    ytdlpDownloadStatus.innerHTML = `
      <div style="display: flex; align-items: center; justify-content: space-between; gap: 8px; color: #00e5ff;">
        <span>⏳ Initializing <strong>yt-dlp</strong> background download...</span>
        <span style="font-weight: 700; color: #00e676;">0%</span>
      </div>`;
  }

  if (btnYtdlpStartDownload) btnYtdlpStartDownload.disabled = true;
  if (btnYtdlpDownloadAndLive) btnYtdlpDownloadAndLive.disabled = true;
  if (btnYtdlpPaste) btnYtdlpPaste.disabled = true;

  try {
    const res = await fetch('/api/media/ytdlp/download', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        url,
        browser,
        sponsorblock_remove_all: sponsorblock,
        audio_only: isAudio
      })
    });

    if (!res.ok) {
      const errText = await res.text();
      throw new Error(errText || 'Download failed to initialize');
    }

    const initProgress = await res.json();
    const taskId = initProgress.task_id;

    if (ytdlpPollingInterval) clearInterval(ytdlpPollingInterval);
    let ytdlpFailCount = 0;

    ytdlpPollingInterval = setInterval(async () => {
      try {
        const progRes = await fetch(`/api/media/ytdlp/progress/${encodeURIComponent(taskId)}`);
        if (!progRes.ok) {
          ytdlpFailCount++;
          if (ytdlpFailCount >= 5) {
            stopYtdlpPolling();
            if (btnYtdlpStartDownload) btnYtdlpStartDownload.disabled = false;
            if (btnYtdlpDownloadAndLive) btnYtdlpDownloadAndLive.disabled = false;
            if (btnYtdlpPaste) btnYtdlpPaste.disabled = false;
            if (ytdlpDownloadStatus) {
              ytdlpDownloadStatus.innerHTML = `<div style="color: #ff5252;">❌ Download status polling connection lost.</div>`;
            }
          }
          return;
        }

        ytdlpFailCount = 0;
        const progress = await progRes.json();
        const pct = Math.min(Math.max(progress.percent || 0, 0), 100);

        if (ytdlpProgressFill) {
          ytdlpProgressFill.style.width = `${pct}%`;
        }

        const speedStr = progress.speed ? ` • ${progress.speed}` : '';
        const etaStr = progress.eta ? ` • ETA ${progress.eta}` : '';

        if (ytdlpDownloadStatus) {
          ytdlpDownloadStatus.innerHTML = `
            <div style="display: flex; align-items: center; justify-content: space-between; gap: 8px;">
              <span style="color: #00e5ff;">${escapeHtml(progress.status || 'Downloading...')}${escapeHtml(speedStr)}${escapeHtml(etaStr)}</span>
              <span style="font-weight: 700; color: #00e676; font-variant-numeric: tabular-nums;">${pct.toFixed(1)}%</span>
            </div>`;
        }

        if (progress.is_complete) {
          stopYtdlpPolling();

          if (btnYtdlpStartDownload) btnYtdlpStartDownload.disabled = false;
          if (btnYtdlpDownloadAndLive) btnYtdlpDownloadAndLive.disabled = false;
          if (btnYtdlpPaste) btnYtdlpPaste.disabled = false;

          if (progress.error) {
            if (ytdlpDownloadStatus) {
              ytdlpDownloadStatus.innerHTML = `
                <div style="color: #ff5252;">
                  ❌ Download failed: ${escapeHtml(progress.error)}
                </div>`;
            }
          } else {
            const mediaItem = progress.media_item;
            const title = mediaItem ? mediaItem.name : 'Media File';

            if (ytdlpProgressFill) ytdlpProgressFill.style.width = '100%';
            if (ytdlpDownloadStatus) {
              ytdlpDownloadStatus.innerHTML = `
                <div style="color: #4caf50; font-weight: 600;">
                  ✓ 100% — Successfully imported <strong>${escapeHtml(title)}</strong> into Media Library!
                </div>`;
            }

            if (ytdlpUrlInput) ytdlpUrlInput.value = '';

            // Ensure downloaded file appears in Media library
            await loadLibraryTab('media');

            // If go live requested
            if (goLiveAfter && mediaItem && mediaItem.id) {
              addItemToSchedule('media', mediaItem.id);
              setTimeout(() => {
                sendCommand({ GoLive: { item_index: null, slide_index: 0 } });
                closeModal(importModal);
              }, 400);
            }
          }
        }
      } catch (pollErr) {
        console.warn('Progress poll error:', pollErr);
      }
    }, 350);
  } catch (err) {
    if (ytdlpPollingInterval) {
      clearInterval(ytdlpPollingInterval);
      ytdlpPollingInterval = null;
    }
    if (btnYtdlpStartDownload) btnYtdlpStartDownload.disabled = false;
    if (btnYtdlpDownloadAndLive) btnYtdlpDownloadAndLive.disabled = false;
    if (btnYtdlpPaste) btnYtdlpPaste.disabled = false;

    if (ytdlpDownloadStatus) {
      ytdlpDownloadStatus.innerHTML = `
        <div style="color: #ff5252;">
          ❌ Initialization error: ${escapeHtml(err instanceof Error ? err.message : String(err))}
        </div>`;
    }
  }
}

if (btnYtdlpStartDownload) on(btnYtdlpStartDownload, 'click', () => handleYtDlpDownload(false));
if (btnYtdlpDownloadAndLive) on(btnYtdlpDownloadAndLive, 'click', () => handleYtDlpDownload(true));
if (ytdlpUrlInput) {
  ytdlpUrlInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      handleYtDlpDownload(false);
    }
  });
}


const ISO_LANG_MAP: Record<string, string> = {
  eng: "English", en: "English",
  spa: "Spanish", es: "Spanish", esp: "Spanish",
  fra: "French", fr: "French", fre: "French",
  deu: "German", de: "German", ger: "German",
  por: "Portuguese", pt: "Portuguese",
  rus: "Russian", ru: "Russian",
  ukr: "Ukrainian", uk: "Ukrainian",
  zho: "Chinese", chi: "Chinese", zh: "Chinese", cmn: "Chinese",
  lat: "Latin", la: "Latin",
  grc: "Greek", ell: "Greek", el: "Greek",
  heb: "Hebrew", he: "Hebrew",
  ita: "Italian", it: "Italian",
  kor: "Korean", ko: "Korean",
  tgl: "Tagalog", tl: "Tagalog", fil: "Tagalog",
  ara: "Arabic", ar: "Arabic",
  vie: "Vietnamese", vi: "Vietnamese",
  hin: "Hindi", hi: "Hindi",
  nld: "Dutch", nl: "Dutch", dut: "Dutch",
  swe: "Swedish", sv: "Swedish",
  nor: "Norwegian", no: "Norwegian",
  dan: "Danish", da: "Danish",
  fin: "Finnish", fi: "Finnish",
  pol: "Polish", pl: "Polish",
  ces: "Czech", cs: "Czech", cze: "Czech",
  ron: "Romanian", ro: "Romanian", rum: "Romanian",
  hun: "Hungarian", hu: "Hungarian",
  afr: "Afrikaans", af: "Afrikaans",
  ind: "Indonesian", id: "Indonesian",
  msa: "Malay", ms: "Malay",
  swh: "Swahili", sw: "Swahili",
  tam: "Tamil", ta: "Tamil",
  tel: "Telugu", te: "Telugu",
  ben: "Bengali", bn: "Bengali",
  jpn: "Japanese", ja: "Japanese"
};

function cleanLanguageName(raw: any) {
  if (!raw) return "English";
  const str = String(raw).trim();
  const lower = str.toLowerCase();
  if (ISO_LANG_MAP[lower]) return ISO_LANG_MAP[lower];
  for (const [k, v] of Object.entries(ISO_LANG_MAP)) {
    if (k.length > 2 && lower.includes(k)) return v;
  }
  const firstWord = str.split(/[\s/,\(\)]+/)[0] || str;
  if (ISO_LANG_MAP[firstWord.toLowerCase()]) return ISO_LANG_MAP[firstWord.toLowerCase()];
  return firstWord.charAt(0).toUpperCase() + firstWord.slice(1);
}

async function loadOnlineBibleCatalog() {
  if (onlineBibleCatalog.length > 0) {
    populateBibleLanguageFilter(onlineBibleCatalog);
    renderOnlineBibleCatalog(onlineBibleCatalog);
    return;
  }

  if (apiBibleCatalogList) {
    apiBibleCatalogList.innerHTML = `<div style="padding: 24px; text-align: center; color: var(--text-dim);">Connecting to ChurchApps & OpenLP Bible Catalog...</div>`;
  }

  try {
    const res = await fetch('/api/bibles/online/catalog');
    if (res.ok) {
      const data = await res.json();
      const rawList = Array.isArray(data) ? data : (data.catalog || getPopularBibleCatalog());
      setOnlineBibleCatalog(deduplicateBibleCatalog(rawList));
      populateBibleLanguageFilter(onlineBibleCatalog);
      renderOnlineBibleCatalog(onlineBibleCatalog);
    } else {
      setOnlineBibleCatalog(deduplicateBibleCatalog(getPopularBibleCatalog()));
      populateBibleLanguageFilter(onlineBibleCatalog);
      renderOnlineBibleCatalog(onlineBibleCatalog);
    }
  } catch (err) {
    setOnlineBibleCatalog(deduplicateBibleCatalog(getPopularBibleCatalog()));
    populateBibleLanguageFilter(onlineBibleCatalog);
    renderOnlineBibleCatalog(onlineBibleCatalog);
  }
}

function getPopularBibleCatalog() {
  return [
    { id: 'kjv', abbreviation: 'KJV', name: 'King James Version (1769)', language: 'English' },
    { id: 'nkjv', abbreviation: 'NKJV', name: 'New King James Version', language: 'English' },
    { id: 'esv', abbreviation: 'ESV', name: 'English Standard Version', language: 'English' },
    { id: 'niv', abbreviation: 'NIV', name: 'New International Version', language: 'English' },
    { id: 'nlt', abbreviation: 'NLT', name: 'New Living Translation', language: 'English' },
    { id: 'asv', abbreviation: 'ASV', name: 'American Standard Version (1901)', language: 'English' },
    { id: 'bsb', abbreviation: 'BSB', name: 'Berean Standard Bible', language: 'English' },
    { id: 'web', abbreviation: 'WEB', name: 'World English Bible', language: 'English' },
    { id: 'bbe', abbreviation: 'BBE', name: 'Bible in Basic English', language: 'English' },
    { id: 'rvr1960', abbreviation: 'RVR1960', name: 'Reina-Valera 1960', language: 'Spanish' },
    { id: 'lsg', abbreviation: 'LSG', name: 'Louis Segond 1910', language: 'French' },
    { id: 'lut', abbreviation: 'LUT', name: 'Lutherbibel 1912', language: 'German' },
    { id: 'cle', abbreviation: 'CLE', name: 'Biblia Sacra Vulgata (Clementine Latin)', language: 'Latin' },
    { id: 'almeida', abbreviation: 'ALM', name: 'João Ferreira de Almeida', language: 'Portuguese' },
    { id: 'synod', abbreviation: 'SYNOD', name: 'Синодальный перевод (Russian Synodal)', language: 'Russian' },
    { id: 'ubio', abbreviation: 'UBIO', name: 'Біблія в пер. Івана Огієнка', language: 'Ukrainian' },
    { id: 'cuv', abbreviation: 'CUV', name: '和合本 (Chinese Union Version)', language: 'Chinese' },
    { id: 'tag', abbreviation: 'TAG', name: 'Ang Biblia (1905)', language: 'Tagalog' }
  ];
}

function deduplicateBibleCatalog(items: any[]) {
  if (!Array.isArray(items)) return [];
  const map = new Map();

  for (const item of items) {
    const rawAbbr = (item.abbreviation || item.id || '').trim().toUpperCase();
    const lang = cleanLanguageName(item.language);
    const name = (item.name || rawAbbr).trim();
    
    // Only deduplicate if exact abbreviation AND language match
    const key = `${rawAbbr}::${lang.toLowerCase()}::${name.toLowerCase().slice(0, 15)}`;
    const src = item.source || 'ChurchApps';

    if (map.has(key)) {
      const existing = map.get(key);
      if (name.length > existing.name.length) {
        existing.name = name;
      }
      if (!existing.sources.includes(src)) {
        existing.sources.push(src);
      }
      if (!existing.source_key && item.source_key) {
        existing.source_key = item.source_key;
      }
    } else {
      map.set(key, {
        id: item.id || rawAbbr,
        abbreviation: rawAbbr,
        name: name,
        language: lang,
        sources: [src],
        provider: item.provider || 'churchapps',
        source_key: item.source_key || null
      });
    }
  }

  return Array.from(map.values()).sort((a, b) => {
    const aEng = a.language.toLowerCase() === 'english';
    const bEng = b.language.toLowerCase() === 'english';
    if (aEng !== bEng) return bEng ? 1 : -1;
    if (a.language !== b.language) return a.language.localeCompare(b.language);
    return a.name.localeCompare(b.name);
  });
}

function populateBibleLanguageFilter(items: any[]) {
  if (!apiBibleLangFilter) return;
  const currentVal = (apiBibleLangFilter.value || 'all').toLowerCase();

  const counts: Record<string, number> = {};
  for (const item of items) {
    const l = cleanLanguageName(item.language) || 'English';
    counts[l] = (counts[l] || 0) + 1;
  }

  const langs = Object.keys(counts).sort((a, b) => {
    if (a.toLowerCase() === 'english') return -1;
    if (b.toLowerCase() === 'english') return 1;
    if (a.toLowerCase() === 'spanish') return -1;
    if (b.toLowerCase() === 'spanish') return 1;
    return a.localeCompare(b);
  });

  let optionsHtml = `<option value="all">🌐 All Languages (${items.length})</option>`;
  for (const l of langs) {
    const isSelected = l.toLowerCase() === currentVal ? 'selected' : '';
    optionsHtml += `<option value="${escapeHtml(l.toLowerCase())}" ${isSelected}>${escapeHtml(l)} (${counts[l]})</option>`;
  }
  apiBibleLangFilter.innerHTML = optionsHtml;
}

function renderOnlineBibleCatalog(items: any[]) {
  if (!apiBibleCatalogList) return;
  const query = apiBibleSearchInput ? apiBibleSearchInput.value.trim().toLowerCase() : '';
  const selectedLang = apiBibleLangFilter ? apiBibleLangFilter.value.toLowerCase() : 'all';

  const filtered = (items || []).filter((b: any) => {
    const matchQuery = !query || 
      (b.name || '').toLowerCase().includes(query) ||
      (b.abbreviation || '').toLowerCase().includes(query) ||
      (b.language || '').toLowerCase().includes(query);
    
    const matchLang = selectedLang === 'all' || (b.language || '').toLowerCase() === selectedLang;

    return matchQuery && matchLang;
  });

  if (apiBibleCountLabel) {
    apiBibleCountLabel.textContent = `Showing ${filtered.length} of ${items.length} translations`;
  }

  apiBibleCatalogList.innerHTML = '';

  if (filtered.length === 0) {
    apiBibleCatalogList.innerHTML = `<div style="padding: 24px; text-align: center; color: var(--text-dim);">No Bible translations match your filter</div>`;
    return;
  }

  filtered.forEach((b: any) => {
    const itemEl = document.createElement('div');
    itemEl.className = 'freeshow-bible-item';

    const lang = (b.language || 'ENG').toUpperCase();
    const abbr = b.abbreviation ? ` (${b.abbreviation})` : '';
    const sourcesStr = (b.sources && b.sources.length > 0) ? b.sources.join(' • ') : (b.source || 'ChurchApps • OpenLP');

    itemEl.innerHTML = `
      <div class="bible-item-main" style="display: flex; flex-direction: column; gap: 2px;">
        <div style="display: flex; align-items: center; gap: 6px;">
          <span class="bible-lang-badge">${escapeHtml(lang)}</span>
          <span class="bible-name-text" style="font-weight: 600;">${escapeHtml(b.name)}${escapeHtml(abbr)}</span>
        </div>
        <div style="font-size: 10px; color: var(--text-dim); margin-left: 2px;">
          Source: <span style="color: #00e5ff;">${escapeHtml(sourcesStr)}</span>
        </div>
      </div>
      <button class="btn-install-bible" data-abbr="${escapeHtml(b.abbreviation || b.id)}">Install / Download</button>
    `;

    const btn = itemEl.querySelector<HTMLButtonElement>('.btn-install-bible');
    if (btn) {
      btn.addEventListener('click', async () => {
        const transId = (b.abbreviation || b.id || 'kjv').toUpperCase();
        btn.disabled = true;
        btn.textContent = '⏳ Downloading Entire Bible...';
        showToast(`⏳ Downloading entire ${transId} Bible (all 66 books, ~31,000 verses)...`, 'info');

        try {
          const res = await fetch('/api/bibles/online/download', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              translation: transId,
              provider: b.provider || 'churchapps',
              source_key: b.source_key || null
            })
          });

          if (res.ok) {
            const data = await res.json();
            showToast(`✓ Successfully downloaded complete ${data.translation} Bible (${data.chapters_count} chapters, ${data.verses_count} verses)!`, 'success');
            btn.classList.add('installed');
            btn.textContent = '✓ Ready';
            await refreshInstalledBibles();
            setActiveBibleVersion(transId.toLowerCase());
            renderCategoryTree('scriptures');
            updateSearchModeUI();
            await loadLibraryTab('scriptures');
            closeModal(importModal);
            if (resourceSearchInput) resourceSearchInput.focus();
          } else {
            const errText = await res.text();
            showToast(`Bible download error: ${errText}`, 'error');
            btn.disabled = false;
            btn.textContent = 'Install / Download';
          }
        } catch (err) {
          showToast(`Network error downloading Bible: ${err instanceof Error ? err.message : String(err)}`, 'error');
          btn.disabled = false;
          btn.textContent = 'Install / Download';
        }
      });
    }

    apiBibleCatalogList.appendChild(itemEl);
  });
}

let bibleSearchDebounceTimer: ReturnType<typeof setTimeout> | null = null;
on(apiBibleSearchInput, 'input', () => {
  clearTimeout(bibleSearchDebounceTimer ?? undefined);
  bibleSearchDebounceTimer = setTimeout(() => {
    renderOnlineBibleCatalog(onlineBibleCatalog);
  }, 150);
});
if (apiBibleLangFilter) {
  apiBibleLangFilter.addEventListener('change', () => {
    renderOnlineBibleCatalog(onlineBibleCatalog);
  });
}

on('btn-import-back', 'click', handleImportBack);
on('btn-close-import', 'click', () => { stopYtdlpPolling(); closeModal(importModal); });
on('btn-close-import-footer', 'click', () => { stopYtdlpPolling(); closeModal(importModal); });

// ============================================================================
// GITHUB BIBLE REPO IMPORTER
// ============================================================================
async function browseGitHubBibleRepo() {
  const repo = ghRepoInput ? ghRepoInput.value.trim() : 'arron-taylor/bible-versions';
  const versionsPath = ghVersionsPathInput ? ghVersionsPathInput.value.trim() : 'versions';

  if (!repo) {
    showToast('Please enter a GitHub repository (e.g. arron-taylor/bible-versions)', 'error');
    return;
  }

  if (btnGhBrowse) {
    (btnGhBrowse as HTMLButtonElement).disabled = true;
    btnGhBrowse.textContent = '⏳ Loading...';
  }
  if (ghBibleCatalogList) {
    ghBibleCatalogList.innerHTML = `<div style="padding: 24px; text-align: center; color: var(--text-dim);">Connecting to GitHub API for <strong>${escapeHtml(repo)}</strong>...</div>`;
  }
  if (ghBibleCountLabel) ghBibleCountLabel.textContent = 'Fetching repo catalog...';
  if (ghBibleSourceLabel) ghBibleSourceLabel.textContent = repo;

  try {
    const res = await fetch('/api/bibles/github/catalog', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ repo, versions_path: versionsPath })
    });

    const data = await res.json();
    if (data.success && Array.isArray(data.catalog)) {
      renderGitHubBibleCatalog(data.catalog);
    } else {
      if (ghBibleCatalogList) {
        ghBibleCatalogList.innerHTML = `<div style="padding: 24px; text-align: center; color: #ff5252;">Failed to load repo catalog: ${escapeHtml(data.error || 'Unknown error')}</div>`;
      }
      if (ghBibleCountLabel) ghBibleCountLabel.textContent = 'Failed to load';
    }
  } catch (err: any) {
    if (ghBibleCatalogList) {
      ghBibleCatalogList.innerHTML = `<div style="padding: 24px; text-align: center; color: #ff5252;">Network error: ${escapeHtml(err.message)}</div>`;
    }
    if (ghBibleCountLabel) ghBibleCountLabel.textContent = 'Network error';
  } finally {
    if (btnGhBrowse) {
      (btnGhBrowse as HTMLButtonElement).disabled = false;
      btnGhBrowse.textContent = '🔍 Browse';
    }
  }
}

function renderGitHubBibleCatalog(items: any[]) {
  if (!ghBibleCatalogList) return;
  if (ghBibleCountLabel) {
    ghBibleCountLabel.textContent = `Found ${items.length} Bible translation${items.length === 1 ? '' : 's'}`;
  }

  ghBibleCatalogList.innerHTML = '';
  if (items.length === 0) {
    ghBibleCatalogList.innerHTML = `<div style="padding: 24px; text-align: center; color: var(--text-dim);">No .json Bible translations found in this repository path.</div>`;
    return;
  }

  items.forEach(b => {
    const itemEl = document.createElement('div');
    itemEl.className = 'freeshow-bible-item';

    const lang = (b.language || 'ENG').toUpperCase();
    const abbr = b.abbreviation ? ` (${b.abbreviation})` : '';

    itemEl.innerHTML = `
      <div class="bible-item-main" style="display: flex; flex-direction: column; gap: 2px;">
        <div style="display: flex; align-items: center; gap: 6px;">
          <span class="bible-lang-badge">${escapeHtml(lang)}</span>
          <span class="bible-name-text" style="font-weight: 600;">${escapeHtml(b.name)}${escapeHtml(abbr)}</span>
        </div>
        <div style="font-size: 10px; color: var(--text-dim); margin-left: 2px;">
          Source: <span style="color: #00e5ff;">${escapeHtml(b.source || 'GitHub')}</span>
        </div>
      </div>
      <button class="btn-install-bible" data-id="${escapeHtml(b.id)}">Install / Download</button>
    `;

    const btn = itemEl.querySelector('.btn-install-bible') as HTMLButtonElement | null;
    if (btn) {
      btn.addEventListener('click', async () => {
        const transId = (b.abbreviation || b.id || 'BIBLE').toUpperCase();
        btn.disabled = true;
        btn.textContent = '⏳ Downloading from GitHub...';
        showToast(`⏳ Downloading ${b.name} from GitHub...`, 'info');

        try {
          const res = await fetch('/api/bibles/online/download', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              translation: transId,
              provider: 'github',
              source_key: b.source_key
            })
          });

          if (res.ok) {
            const data = await res.json();
            showToast(`✓ Successfully downloaded complete ${data.translation} Bible (${data.chapters_count} chapters, ${data.verses_count} verses)!`, 'success');
            btn.classList.add('installed');
            btn.textContent = '✓ Ready';
            await refreshInstalledBibles();
            setActiveBibleVersion(transId.toLowerCase());
            renderCategoryTree('scriptures');
            updateSearchModeUI();
            await loadLibraryTab('scriptures');
            closeModal(importModal);
            if (resourceSearchInput) resourceSearchInput.focus();
          } else {
            const errText = await res.text();
            showToast(`Bible download error: ${errText}`, 'error');
            btn.disabled = false;
            btn.textContent = 'Install / Download';
          }
        } catch (err: any) {
          showToast(`Network error downloading Bible: ${err instanceof Error ? err.message : String(err)}`, 'error');
          btn.disabled = false;
          btn.textContent = 'Install / Download';
        }
      });
    }

    ghBibleCatalogList.appendChild(itemEl);
  });
}

if (btnGhBrowse) on(btnGhBrowse, 'click', browseGitHubBibleRepo);
if (ghRepoInput) {
  ghRepoInput.addEventListener('keydown', (e: KeyboardEvent) => {
    if (e.key === 'Enter') browseGitHubBibleRepo();
  });
}
if (ghVersionsPathInput) {
  ghVersionsPathInput.addEventListener('keydown', (e: KeyboardEvent) => {
    if (e.key === 'Enter') browseGitHubBibleRepo();
  });
}

// FreeShow .fsb File Import
on('file-freeshow-fsb', 'change', async (e: Event) => {
  const file = (e.target as HTMLInputElement).files?.[0];
  if (!file) return;
  showToast(`Importing ${file.name}...`, 'info');

  try {
    const text = await file.text();
    const res = await fetch('/api/bibles/import/fsb', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ content: text })
    });

    if (res.ok) {
      const items = await res.json();
      showToast(`✓ Imported ${items.length} chapter items from FreeShow Bible (${file.name})!`, 'success');
      await refreshInstalledBibles();
      loadLibraryTab(currentTab);
      closeModal(importModal);
    } else {
      const err = await res.text();
      showToast(`Import error: ${err}`, 'error');
    }
  } catch (err) {
    showToast(`File read error: ${err instanceof Error ? err.message : String(err)}`, 'error');
  }
});

// OpenLP SQLite Import (songs.sqlite / *.sqlite Bible)
on('file-openlp-import', 'change', async (e: Event) => {
  const file = (e.target as HTMLInputElement).files?.[0];
  if (!file) return;
  const statusEl = document.getElementById('openlp-import-status');
  if (statusEl) {
    statusEl.style.display = 'block';
    statusEl.textContent = `Reading ${file.name}...`;
  }
  showToast(`Reading OpenLP file ${file.name}...`, 'info');

  try {
    const b64 = await fileToBase64(file);
    if (statusEl) statusEl.textContent = `Importing ${file.name} into database...`;
    const res = await fetch('/api/import/openlp', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ file_name: file.name, file_data_base64: b64 })
    });

    if (res.ok) {
      const data = await res.json();
      const count = data.imported_count ?? 0;
      const msg = `✓ OpenLP Import Successful: ${data.message || `${count} items imported`}`;
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#00e676';
        statusEl.textContent = msg;
      }
      showToast(msg, 'success');
      await refreshInstalledBibles();
      loadLibraryTab(currentTab);
    } else {
      const err = await res.text();
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#ff5252';
        statusEl.textContent = `Import error: ${err}`;
      }
      showToast(`OpenLP import error: ${err}`, 'error');
    }
  } catch (err: any) {
    if (statusEl) {
      statusEl.style.display = 'block';
      statusEl.style.color = '#ff5252';
      statusEl.textContent = `Error: ${err.message}`;
    }
    showToast(`OpenLP import failed: ${err.message}`, 'error');
  } finally {
    (e.target as HTMLInputElement).value = '';
  }
});

// FreeShow Show (.show / .json) Import
on('file-freeshow-show-import', 'change', async (e: Event) => {
  const file = (e.target as HTMLInputElement).files?.[0];
  if (!file) return;
  const statusEl = document.getElementById('freeshow-show-import-status');
  if (statusEl) {
    statusEl.style.display = 'block';
    statusEl.style.color = 'var(--text-dim)';
    statusEl.textContent = `Reading ${file.name}...`;
  }
  showToast(`Reading FreeShow file ${file.name}...`, 'info');

  try {
    const b64 = await fileToBase64(file);
    if (statusEl) statusEl.textContent = `Importing slides and media from ${file.name}...`;
    const res = await fetch('/api/import/freeshow-show', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ file_name: file.name, file_data_base64: b64 })
    });

    if (res.ok) {
      const data = await res.json();
      const msg = `✓ ${data.message || `Imported '${data.title}' (${data.slide_count} slides)`}`;
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#00e676';
        statusEl.textContent = msg;
      }
      showToast(msg, 'success');
      if (data.type === 'song') {
        loadLibraryTab('songs');
      } else {
        loadLibraryTab('presentations');
      }
    } else {
      const err = await res.text();
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#ff5252';
        statusEl.textContent = `Import error: ${err}`;
      }
      showToast(`FreeShow import error: ${err}`, 'error');
    }
  } catch (err: any) {
    if (statusEl) {
      statusEl.style.display = 'block';
      statusEl.style.color = '#ff5252';
      statusEl.textContent = `Error: ${err.message}`;
    }
    showToast(`FreeShow import failed: ${err.message}`, 'error');
  } finally {
    (e.target as HTMLInputElement).value = '';
  }
});

// PowerPoint (.pptx) Import
on('file-pptx-import', 'change', async (e: Event) => {
  const file = (e.target as HTMLInputElement).files?.[0];
  if (!file) return;
  const statusEl = document.getElementById('pptx-import-status');
  if (statusEl) {
    statusEl.style.display = 'block';
    statusEl.style.color = 'var(--text-dim)';
    statusEl.textContent = `Reading ${file.name}...`;
  }
  showToast(`Reading PowerPoint file ${file.name}...`, 'info');

  try {
    const b64 = await fileToBase64(file);
    if (statusEl) statusEl.textContent = `Extracting slides from ${file.name}...`;
    const res = await fetch('/api/import/pptx', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ file_name: file.name, file_data_base64: b64 })
    });

    if (res.ok) {
      const data = await res.json();
      const msg = `✓ ${data.message || `Imported '${data.title}' (${data.slide_count} slides)`}`;
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#00e676';
        statusEl.textContent = msg;
      }
      showToast(msg, 'success');
      loadLibraryTab('presentations');
    } else {
      const err = await res.text();
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#ff5252';
        statusEl.textContent = `Import error: ${err}`;
      }
      showToast(`PowerPoint import error: ${err}`, 'error');
    }
  } catch (err: any) {
    if (statusEl) {
      statusEl.style.display = 'block';
      statusEl.style.color = '#ff5252';
      statusEl.textContent = `Error: ${err.message}`;
    }
    showToast(`PowerPoint import failed: ${err.message}`, 'error');
  } finally {
    (e.target as HTMLInputElement).value = '';
  }
});

// OpenLP Auto-Detect Local DB
on('btn-openlp-autodetect', 'click', async () => {
  const statusEl = document.getElementById('openlp-import-status');
  if (statusEl) {
    statusEl.style.display = 'block';
    statusEl.style.color = 'var(--text-dim)';
    statusEl.textContent = 'Scanning system for OpenLP databases...';
  }
  showToast('Scanning system for OpenLP databases...', 'info');

  try {
    const res = await fetch('/api/import/openlp', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({})
    });

    if (res.ok) {
      const data = await res.json();
      const count = data.imported_count ?? 0;
      const msg = `✓ ${data.message || `Imported ${count} items from local OpenLP installation`}`;
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#00e676';
        statusEl.textContent = msg;
      }
      showToast(msg, 'success');
      await refreshInstalledBibles();
      loadLibraryTab(currentTab);
    } else {
      const err = await res.text();
      if (statusEl) {
        statusEl.style.display = 'block';
        statusEl.style.color = '#ff5252';
        statusEl.textContent = `Auto-detect error: ${err}`;
      }
      showToast(`OpenLP auto-detect error: ${err}`, 'error');
    }
  } catch (err: any) {
    if (statusEl) {
      statusEl.style.display = 'block';
      statusEl.style.color = '#ff5252';
      statusEl.textContent = `Error: ${err.message}`;
    }
    showToast(`OpenLP auto-detect failed: ${err.message}`, 'error');
  }
});

// Tab Navigation Event Listeners
document.querySelectorAll<HTMLElement>('.tab-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    loadLibraryTab(btn.dataset.tab);
  });
});

// ============================================================================
// COMPLETE KEYBOARD SHORTCUTS MATRIX & ERGONOMICS
// ============================================================================
// Delegates to resolveKeyboardShortcut (core/presentation_helpers.ts) for the pure
// "which key means what" decision, then just carries out that one decision's real
// side effects here — replaces what used to be two independently hand-maintained
// copies of the same shortcut matrix (this dispatch, and that pure function's own
// long-unused twin), which had already drifted apart on what Backspace does.
document.addEventListener('keydown', (e) => {
  const target = e.target as HTMLElement;
  const resolved = resolveKeyboardShortcut({
    key: e.key,
    ctrlKey: e.ctrlKey,
    shiftKey: e.shiftKey,
    altKey: e.altKey,
    metaKey: e.metaKey,
    targetTagName: target?.tagName,
    isContentEditable: target?.isContentEditable,
  });
  if (!resolved) return;
  if (resolved.preventDefault) e.preventDefault();

  switch (resolved.action) {
    case 'search_focus':
      if (resourceSearchInput) {
        resourceSearchInput.focus();
        resourceSearchInput.select();
      }
      break;

    case 'close_modals':
      closeTopmostModal();
      break;

    case 'delete_selected_item':
      // Note: while the Studio editor (create-modal) is open, its own canvas keydown
      // handler (slide_editor.ts) already owns Delete/Backspace for deleting the
      // selected canvas element — nothing further to route here. A dead branch used to
      // sit here trying to route to a whole-slide deletion via a function
      // (deleteStudioSlide) that no longer exists anywhere in this codebase, guarded by
      // an element id (create-item-modal) that also didn't match anything (the real id
      // is create-modal) — removed rather than fixed, since making the id lookup
      // succeed without deleteStudioSlide existing would still do nothing, and wiring
      // up a real "delete the whole slide" action here would double up destructively
      // with the canvas's own selected-element deletion on the same keypress.
      if (deleteSelectedSlideOrItem()) break;
      deleteActiveSelectedItemWithUndo();
      break;

    case 'undo':
      if (activeUndoPlaceholder) {
        triggerUndoFromPlaceholder();
      } else {
        sendCommand(resolved.command);
        showToast('↩ Undo', 'info');
      }
      break;

    case 'redo':
      sendCommand(resolved.command);
      showToast('↪ Redo', 'info');
      break;

    case 'guide':
      openScheduleGuideModal();
      break;

    case 'shortcuts':
      showModal(shortcutsModal);
      break;

    case 'options':
      resetOptionsModal();
      showModal(optionsModal);
      break;

    case 'alert':
      resetAlertModal();
      showModal(alertModal);
      if (alertTextInput) {
        alertTextInput.focus();
        alertTextInput.select();
      }
      break;

    case 'new_schedule':
      sendCommand(resolved.command);
      break;

    case 'open_schedule':
      resetOpenModal();
      showModal(openModal);
      break;

    case 'save_schedule':
      quickSaveSchedule();
      break;

    case 'save_schedule_as':
      resetSaveModal();
      showModal(saveModal);
      break;

    case 'import_modal':
      openImportModal();
      break;

    case 'create_song':
      openSlideEditor('song');
      break;

    // Plain command pass-throughs: the resolved command is already exactly what
    // sendCommand expects.
    case 'blackout':
    case 'clear_text':
    case 'logo':
    case 'go_live':
    case 'next_slide':
    case 'prev_slide':
    case 'next_item':
    case 'prev_item':
    case 'jump_slide':
    case 'jump_section':
      sendCommand(resolved.command);
      break;
  }
});

// ============================================================================
// WORKSPACE SPLITTER RESIZING ENGINE
// ============================================================================
function initWorkspaceResizers() {
  const schedPrevSplitter = document.getElementById('splitter-sched-prev');
  const prevLiveSplitter = document.getElementById('splitter-prev-live');
  const midBottomSplitter = document.getElementById('splitter-mid-bottom');
  const prevMonitorSplitter = document.getElementById('splitter-prev-monitor');
  const liveMonitorSplitter = document.getElementById('splitter-live-monitor');

  const schedulePanel = document.getElementById('schedule-panel');
  const previewPanel = document.getElementById('preview-panel');
  const liveOutputPanel = document.getElementById('live-output-panel');
  const bottomTier = document.getElementById('bottom-tier');
  const previewViewportWrap = document.getElementById('preview-viewport-wrap');
  const liveViewportWrap = document.getElementById('live-viewport-wrap');

  // Load saved layout preferences with sane boundaries
  try {
    const saved = localStorage.getItem('opensanctuary_layout');
    if (saved) {
      const layout = JSON.parse(saved);
      if (layout.schedWidth && schedulePanel) schedulePanel.style.width = layout.schedWidth;
      if (layout.prevFlex && previewPanel) previewPanel.style.flex = layout.prevFlex;
      if (layout.liveFlex && liveOutputPanel) liveOutputPanel.style.flex = layout.liveFlex;

      if (bottomTier) {
        bottomTier.style.flex = 'none';
        const parsedH = layout.bottomHeight ? parseInt(layout.bottomHeight, 10) : NaN;
        const maxH = Math.max(200, Math.floor(window.innerHeight * 0.45));
        if (!isNaN(parsedH) && parsedH >= 120 && parsedH <= maxH) {
          bottomTier.style.height = `${parsedH}px`;
        } else {
          bottomTier.style.height = '250px';
        }
      }

      if (layout.prevMonitorHeight && previewViewportWrap) {
        const ph = parseInt(layout.prevMonitorHeight, 10);
        if (!isNaN(ph) && ph >= 80 && ph <= 350) previewViewportWrap.style.height = `${ph}px`;
      }
      if (layout.liveMonitorHeight && liveViewportWrap) {
        const lh = parseInt(layout.liveMonitorHeight, 10);
        if (!isNaN(lh) && lh >= 80 && lh <= 350) liveViewportWrap.style.height = `${lh}px`;
      }
    } else if (bottomTier) {
      bottomTier.style.flex = 'none';
      bottomTier.style.height = '250px';
    }
  } catch (e) {
    console.warn('Could not load layout settings:', e);
    if (bottomTier) {
      bottomTier.style.flex = 'none';
      bottomTier.style.height = '250px';
    }
  }

  function saveLayout() {
    try {
      const layout = {
        schedWidth: schedulePanel ? schedulePanel.style.width : null,
        prevFlex: previewPanel ? previewPanel.style.flex : null,
        liveFlex: liveOutputPanel ? liveOutputPanel.style.flex : null,
        bottomHeight: bottomTier ? bottomTier.style.height : null,
        prevMonitorHeight: previewViewportWrap ? previewViewportWrap.style.height : null,
        liveMonitorHeight: liveViewportWrap ? liveViewportWrap.style.height : null,
      };
      localStorage.setItem('opensanctuary_layout', JSON.stringify(layout));
    } catch (e) {}
  }

  // 1. Schedule vs Preview Vertical Resizing
  if (schedPrevSplitter && schedulePanel) {
    const splitter = schedPrevSplitter;
    const panel = schedulePanel;
    splitter.addEventListener('mousedown', (e) => {
      e.preventDefault();
      splitter.classList.add('active-dragging');
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
      const startX = e.clientX;
      const startWidth = panel.getBoundingClientRect().width;

      function onMouseMove(ev: MouseEvent) {
        const delta = ev.clientX - startX;
        const newWidth = Math.max(140, Math.min(window.innerWidth - 450, startWidth + delta));
        panel.style.width = `${newWidth}px`;
      }

      function onMouseUp() {
        splitter.classList.remove('active-dragging');
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        saveLayout();
      }

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }

  // 2. Preview vs Live Vertical Resizing
  if (prevLiveSplitter && previewPanel && liveOutputPanel) {
    const splitter = prevLiveSplitter;
    const prevPanel = previewPanel;
    const livePanel = liveOutputPanel;
    splitter.addEventListener('mousedown', (e) => {
      e.preventDefault();
      splitter.classList.add('active-dragging');
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
      const startX = e.clientX;
      const startPrevWidth = prevPanel.getBoundingClientRect().width;
      const startLiveWidth = livePanel.getBoundingClientRect().width;
      const totalWidth = startPrevWidth + startLiveWidth;

      function onMouseMove(ev: MouseEvent) {
        const delta = ev.clientX - startX;
        const newPrevWidth = Math.max(180, Math.min(totalWidth - 180, startPrevWidth + delta));
        const newLiveWidth = totalWidth - newPrevWidth;
        prevPanel.style.flex = `${newPrevWidth}`;
        livePanel.style.flex = `${newLiveWidth}`;
      }

      function onMouseUp() {
        splitter.classList.remove('active-dragging');
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        saveLayout();
      }

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }

  // 3. Middle Tier vs Bottom Tier Vertical Resizing (Dragging UP increases bottom tier, DOWN decreases it)
  if (midBottomSplitter && bottomTier) {
    const splitter = midBottomSplitter;
    const tier = bottomTier;
    splitter.addEventListener('mousedown', (e) => {
      e.preventDefault();
      splitter.classList.add('active-dragging');
      document.body.style.cursor = 'row-resize';
      document.body.style.userSelect = 'none';
      const startY = e.clientY;
      const startHeight = tier.getBoundingClientRect().height;

      function onMouseMove(ev: MouseEvent) {
        const delta = startY - ev.clientY; // UP is positive, DOWN is negative
        const maxAllowed = Math.max(220, window.innerHeight - 240);
        const newHeight = Math.max(120, Math.min(maxAllowed, startHeight + delta));
        tier.style.flex = 'none';
        tier.style.height = `${newHeight}px`;
      }

      function onMouseUp() {
        splitter.classList.remove('active-dragging');
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        saveLayout();
      }

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }

  // 4. Preview Monitor Height Resizing
  if (prevMonitorSplitter && previewViewportWrap) {
    const splitter = prevMonitorSplitter;
    const viewportWrap = previewViewportWrap;
    splitter.addEventListener('mousedown', (e) => {
      e.preventDefault();
      splitter.classList.add('active-dragging');
      document.body.style.cursor = 'row-resize';
      document.body.style.userSelect = 'none';
      const startY = e.clientY;
      const startHeight = viewportWrap.getBoundingClientRect().height;

      function onMouseMove(ev: MouseEvent) {
        const delta = ev.clientY - startY; // DOWN is positive, UP is negative
        const panelH = previewPanel ? previewPanel.getBoundingClientRect().height : 500;
        const maxAllowed = Math.max(120, Math.floor(panelH * 0.65));
        const newHeight = Math.max(80, Math.min(maxAllowed, startHeight + delta));
        viewportWrap.style.height = `${newHeight}px`;
      }

      function onMouseUp() {
        splitter.classList.remove('active-dragging');
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        saveLayout();
      }

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }

  // 5. Live Monitor Height Resizing
  if (liveMonitorSplitter && liveViewportWrap) {
    const splitter = liveMonitorSplitter;
    const viewportWrap = liveViewportWrap;
    splitter.addEventListener('mousedown', (e) => {
      e.preventDefault();
      splitter.classList.add('active-dragging');
      document.body.style.cursor = 'row-resize';
      document.body.style.userSelect = 'none';
      const startY = e.clientY;
      const startHeight = viewportWrap.getBoundingClientRect().height;

      function onMouseMove(ev: MouseEvent) {
        const delta = ev.clientY - startY; // DOWN is positive, UP is negative
        const panelH = liveOutputPanel ? liveOutputPanel.getBoundingClientRect().height : 500;
        const maxAllowed = Math.max(120, Math.floor(panelH * 0.65));
        const newHeight = Math.max(80, Math.min(maxAllowed, startHeight + delta));
        viewportWrap.style.height = `${newHeight}px`;
      }

      function onMouseUp() {
        splitter.classList.remove('active-dragging');
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        saveLayout();
      }

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }
}

// Polls the background yt-dlp self-updater's last result
// (src/network/ytdlp_updater.rs) and surfaces a toast only for the
// noteworthy cases -- a real update or a real error -- not the routine
// silent majority (already current, or deferring to a system install).
// Deduped via localStorage so a page reload (or the 20-minute re-poll
// below) doesn't re-toast the same outcome repeatedly.
async function checkYtdlpUpdaterStatus() {
  const STORAGE_KEY = 'os_ytdlp_updater_last_toasted';
  try {
    const { status } = await api.ytdlpUpdater.status();
    if (!status || (!status.updated && !status.error)) return;

    const marker = status.error ? `error:${status.message}` : `updated:${status.version}`;
    let lastToasted: string | null = null;
    try {
      lastToasted = localStorage.getItem(STORAGE_KEY);
    } catch (_) { /* private browsing / storage blocked -- just skip dedupe */ }
    if (marker === lastToasted) return;

    if (status.error) {
      showToast(`yt-dlp updater: ${status.message}`, 'warning', 6000);
    } else {
      showToast(`✓ yt-dlp updated to ${status.version}`, 'success');
    }
    try {
      localStorage.setItem(STORAGE_KEY, marker);
    } catch (_) { /* best-effort dedupe only */ }
  } catch (_) {
    // Best-effort background notification, not a critical path -- a failed
    // status poll shouldn't itself show an error toast.
  }
}

// Initialize on page load
initWebSocket();
loadAppOptions();
refreshAvailableThemes();
refreshInstalledBibles();
loadLibraryTab('songs');
initWorkspaceResizers();
loadOnlineBibleCatalog();
initFirstTimeSetup();
checkYtdlpUpdaterStatus();
setInterval(checkYtdlpUpdaterStatus, 20 * 60 * 1000);
maybeShowFirstTimeSetup();

// Expose for inline HTML onclick handlers
(window as any).showToast = showToast;

// Not the raw `ytdlpPollingInterval` variable -- a one-time snapshot of it
// would go stale the instant the real (module-local) interval changes, which
// is exactly what left app_core.ts's resetImportModal() unable to actually
// cancel the poll. Expose the function itself instead: it always closes
// over this module's live variable.

// Hands these back to app_core.ts once all are declared -- see
// registerUiCallbacks in app_core.ts for why this isn't a static import.
registerUiCallbacks({
  stopYtdlpPolling,
  editExistingItem,
  openImportModal,
  openSlideEditor,
  promptNewSectionHeader,
  renderOnlineBibleCatalog,
  setImportMode,
  switchToPairingTab,
  switchToAdbProvisionTab,
  showFirstTimeSetup,
});

// Exposed on window for Playwright E2E tests that drive the running app
// directly (tests/e2e_bulk_paste.test.ts, e2e_slide_templates.test.ts,
// e2e_image_crop.test.ts, e2e_rich_text_selection.test.ts, etc.), outside this
// module graph entirely -- not needed by any in-repo module.
try { (globalThis as any).openImportModal = openImportModal; } catch (_) {}
try { (globalThis as any).openSlideEditor = openSlideEditor; } catch (_) {}
try { (globalThis as any).getCanvasSlideEditorDebugState = () => ({ hasEditor: !!canvasSlideEditor, editingScheduleContext, activeSlideElements: canvasSlideEditor ? canvasSlideEditor.getActiveSlide().elements.map((e: any) => ({ id: e.id, transform: e.transform, block: e.type === 'TextBlock' ? e.block : undefined })) : null }); } catch (_) {}
try { (globalThis as any).__debugApplyTextStyle = (update: any) => canvasSlideEditor?.applyTextStyle(update); } catch (_) {}
try {
  (globalThis as any).__debugInsertElement = (el: any) => {
    if (!canvasSlideEditor) return false;
    const cur = canvasSlideEditor.getActiveSlide();
    cur.elements.push(el);
    canvasSlideEditor.canvas.setSlide(cur);
    return true;
  };
} catch (_) {}
