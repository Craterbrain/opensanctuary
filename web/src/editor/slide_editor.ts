import { EditorSlide, SlideElement, SlideEditOp, SlideBackground, TextStyleUpdate, TextRun, normalizeTransform } from './types';
import { EditorCanvas } from './canvas';
import { EditorToolbar, applyArchetypeToSlide, ArrangeAction, AlignAction } from './toolbar';
import { PropertiesPanel } from './properties_panel';
import { NotesPane } from './notes_pane';
import { FilmstripSidebar } from './thumbnail';
import { EditorContextMenu } from './context_menu';
import { EditorHistoryManager } from './history';
import { applyStyleToSelection, reconcileTextBlockFromDOM } from './text_selection';
import { formatBulletLines } from './text_block';
import { ImageCropOverlay } from './image_crop_overlay';
import { openMediaImagePicker } from '../ui/media_image_picker';
import { isVideoBackground } from '../core/presentation_helpers';

export interface SlideTemplate {
  id: string;
  name: string;
  category?: string | null;
  elements: SlideElement[];
  background?: SlideBackground | null;
  thumbnail_data_url?: string | null;
}

/** Regenerates every element's id (recursing into Group children) so applying
 * the same template twice — or applying it alongside the slide it was saved
 * from — never produces id collisions. */
function regenerateElementIds(elements: SlideElement[]): SlideElement[] {
  return elements.map(el => {
    const copy: any = { ...el, id: `${el.type.toLowerCase()}-${Date.now()}-${Math.random().toString(36).substr(2, 6)}` };
    if (el.type === 'Group') {
      copy.children = regenerateElementIds(el.children);
    }
    return copy as SlideElement;
  });
}

/** Sets both background_v2 (source of truth) and the derived legacy CSS
 * `background` string on a slide, keeping them consistent — the REST save
 * path still reads the legacy field as its background value. */
function applyBackgroundToSlide(cur: EditorSlide, bg: SlideBackground | null | undefined): void {
  if (bg) {
    cur.background_v2 = bg;
    if (bg.kind === 'Solid') {
      cur.background = bg.data;
    } else if (bg.kind === 'Gradient') {
      const stops = bg.data.stops.map(s => `${s.color} ${s.offset * 100}%`).join(', ');
      cur.background = bg.data.kind === 'radial'
        ? `radial-gradient(circle, ${stops})`
        : `linear-gradient(${bg.data.angle_deg ?? 180}deg, ${stops})`;
    } else if (bg.kind === 'Image') {
      cur.background = `url("${bg.data.file_path}")`;
    } else if (bg.kind === 'Video') {
      cur.background = bg.data.file_path;
    }
  } else {
    cur.background_v2 = undefined;
    cur.background = undefined;
  }
}

function applyPickerBackgroundToSlide(
  cur: EditorSlide,
  filePath: string,
  cssBg: string,
  isVideo?: boolean,
  isLooping?: boolean
): void {
  if (isVideo && filePath) {
    cur.background_v2 = {
      kind: 'Video',
      data: {
        file_path: filePath,
        loop_playback: isLooping !== false,
        is_muted: true
      }
    };
    cur.background = filePath;
    return;
  }
  if (filePath) {
    if (isVideoBackground(filePath)) {
      cur.background_v2 = {
        kind: 'Video',
        data: {
          file_path: filePath,
          loop_playback: isLooping !== false,
          is_muted: true
        }
      };
      cur.background = filePath;
    } else {
      cur.background_v2 = { kind: 'Image', data: { file_path: filePath, opacity: 1.0 } };
      cur.background = `url("${filePath}")`;
    }
    return;
  }
  if (!cssBg) {
    cur.background_v2 = undefined;
    cur.background = undefined;
    return;
  }
  const cleanBg = cssBg.trim();
  if (isVideoBackground(cleanBg)) {
    cur.background_v2 = {
      kind: 'Video',
      data: {
        file_path: cleanBg,
        loop_playback: isLooping !== false,
        is_muted: true
      }
    };
    cur.background = cleanBg;
    return;
  }
  if (cleanBg.startsWith('url(')) {
    const match = cleanBg.match(/url\(['"]?([^'"]+)['"]?\)/);
    const p = match ? match[1] : cleanBg;
    if (isVideoBackground(p)) {
      cur.background_v2 = {
        kind: 'Video',
        data: {
          file_path: p,
          loop_playback: isLooping !== false,
          is_muted: true
        }
      };
      cur.background = p;
    } else {
      cur.background_v2 = { kind: 'Image', data: { file_path: p, opacity: 1.0 } };
      cur.background = `url("${p}")`;
    }
  } else if (cleanBg.startsWith('#') || cleanBg.startsWith('rgb')) {
    cur.background_v2 = { kind: 'Solid', data: cleanBg };
    cur.background = cleanBg;
  } else if (cleanBg.startsWith('pattern:')) {
    cur.background_v2 = { kind: 'Solid', data: cleanBg };
    cur.background = cleanBg;
  } else {
    cur.background_v2 = undefined;
    cur.background = cleanBg;
  }
}

export function projectTextFromElements(elements: SlideElement[]): string {
  const chunks: string[] = [];
  for (const el of elements) {
    if (el.type === 'TextBlock') {
      const text = el.block.runs.map(r => r.text).join('');
      if (text.trim()) {
        chunks.push(text.trim());
      }
    } else if (el.type === 'Table') {
      const rows = el.cells.map(r => r.join(' | ')).join('\n');
      if (rows.trim()) chunks.push(rows.trim());
    } else if (el.type === 'Group') {
      const gText = projectTextFromElements(el.children);
      if (gText.trim()) chunks.push(gText.trim());
    }
  }
  return chunks.join('\n\n');
}

export interface SlideEditorOptions {
  container: HTMLElement;
  onApply?: (result: { slides: EditorSlide[]; batchOps: SlideEditOp[]; hasChanges: boolean; activeIndex: number }) => void;
  onCancel?: () => void;
}

export class SlideEditor {
  public container: HTMLElement;
  public canvas!: EditorCanvas;
  public toolbar!: EditorToolbar;
  public propertiesPanel!: PropertiesPanel;
  public notesPane!: NotesPane;
  public filmstrip!: FilmstripSidebar;
  public contextMenu!: EditorContextMenu;
  public history: EditorHistoryManager;

  private slides: EditorSlide[] = [];
  private activeIndex: number = 0;
  private clipboard: SlideElement[] = [];
  private activeCropOverlay: ImageCropOverlay | null = null;
  private templates: SlideTemplate[] = [];
  private onApplyCb?: (result: { slides: EditorSlide[]; batchOps: SlideEditOp[]; hasChanges: boolean; activeIndex: number }) => void;
  private onCancelCb?: () => void;
  private boundKeydownHandler: ((e: KeyboardEvent) => void) | null = null;
  private mode: string = 'song';

  constructor(options: SlideEditorOptions) {
    this.container = options.container;
    this.onApplyCb = options.onApply;
    this.onCancelCb = options.onCancel;
    this.history = new EditorHistoryManager(100);

    this.initLayout();
    this.bindKeyboardShortcuts();
  }

  public setMode(mode: string): void {
    this.mode = mode || 'song';
    this.container.dataset.mode = this.mode;
    this.container.classList.toggle('mode-song', this.isSongMode());
    this.filmstrip.setMode(this.mode);
  }

  public getMode(): string {
    return this.mode;
  }

  public isSongMode(): boolean {
    return (this.mode || 'song').toLowerCase() === 'song';
  }

  public open(slides: EditorSlide[], activeIndex: number = 0, mode: string = 'song'): void {
    this.mode = mode || 'song';
    this.container.dataset.mode = this.mode;
    this.container.classList.toggle('mode-song', this.isSongMode());

    // Deep clone incoming slides
    this.slides = JSON.parse(JSON.stringify(slides));
    if (this.slides.length === 0) {
      this.slides = [this.createBlankSlide(0)];
    }
    this.activeIndex = Math.max(0, Math.min(activeIndex, this.slides.length - 1));

    const cur = this.slides[this.activeIndex];
    this.history.init(cur);
    this.canvas.setSlide(cur);
    this.propertiesPanel.bind(cur, []);
    this.notesPane.bind(cur);
    this.filmstrip.setMode(this.mode);
    this.filmstrip.setSlides(this.slides, this.activeIndex);
    this.updateToolbarState();
    requestAnimationFrame(() => this.canvas.zoomToFit());
    setTimeout(() => this.canvas.zoomToFit(), 50);
    setTimeout(() => this.canvas.zoomToFit(), 150);
  }

  public getActiveSlide(): EditorSlide {
    return this.slides[this.activeIndex];
  }

  public getAllSlides(): EditorSlide[] {
    return this.slides;
  }

  public apply(): void {
    const cur = this.getActiveSlide();
    cur.text = projectTextFromElements(cur.elements);
    const batchOps = this.history.computeBatchOps(cur);
    const hasChanges = this.history.hasChanges(cur);

    if (this.onApplyCb) {
      this.onApplyCb({
        slides: this.slides,
        batchOps,
        hasChanges,
        activeIndex: this.activeIndex
      });
    }
  }

  public cancel(): void {
    if (this.onCancelCb) {
      this.onCancelCb();
    }
  }

  public destroy(): void {
    if (this.boundKeydownHandler) {
      window.removeEventListener('keydown', this.boundKeydownHandler);
      this.boundKeydownHandler = null;
    }
    this.contextMenu.hide();
    this.container.innerHTML = '';
  }

  private initLayout(): void {
    this.container.innerHTML = '';
    this.container.className = 'editor-main-workspace-shell flex flex-col h-full w-full bg-[#121214] text-white overflow-hidden select-none';
    this.container.style.display = 'flex';
    this.container.style.flexDirection = 'column';
    this.container.style.width = '100%';
    this.container.style.height = '100%';
    this.container.style.background = '#121214';
    this.container.style.color = '#ffffff';
    this.container.style.overflow = 'hidden';
    this.container.style.boxSizing = 'border-box';

    // 1. Ribbon toolbar: static markup in index.html (#slide-editor-ribbon,
    // a sibling of .studio-workspace-row so it spans the full width above
    // the filmstrip/canvas/sidebar row, not just above this shell) — see
    // docs/FRONTEND_ARCHITECTURE.md [ARCH:slide-editor-shell]. EditorToolbar
    // binds to it rather than building it.
    const ribbonEl = document.getElementById('slide-editor-ribbon');
    if (!ribbonEl) {
      throw new Error('SlideEditor: expected static #slide-editor-ribbon in index.html');
    }

    this.toolbar = new EditorToolbar(ribbonEl, {
      onAddSlide: () => this.addNewSlide(),
      onDuplicateSlide: () => this.duplicateSlide(this.activeIndex),
      onDeleteSlide: () => this.deleteSlide(this.activeIndex),
      onInsertElement: (el) => this.insertElement(el),
      onApplyArchetype: (archetype) => this.applyArchetype(archetype),
      onUndo: () => this.undo(),
      onRedo: () => this.redo(),
      onArrange: (action) => this.arrange(action),
      onAlign: (action) => this.align(action),
      onGroup: () => this.groupSelected(),
      onUngroup: () => this.ungroupSelected(),
      onZoomChange: (zoom) => {
        this.canvas.setZoom(zoom);
        this.toolbar.updateZoom(this.canvas.getZoom());
      },
      onZoomToFit: () => {
        this.canvas.zoomToFit();
        this.toolbar.updateZoom(this.canvas.getZoom());
      },
      onUpdateTextStyle: (update) => this.applyTextStyle(update),
      onUpdateBackground: (bg) => this.updateSlideBackground(bg),
      onOpenBackgroundPicker: () => this.openBackgroundGridPicker(),
      onSaveAsTemplate: () => this.saveActiveSlideAsTemplate(),
      onApplyTemplate: (templateId) => this.applyTemplate(templateId)
    });
    this.refreshTemplates();

    // 2. Middle Region (Filmstrip + Canvas Viewport + Properties Panel)
    const middleRegion = document.createElement('div');
    middleRegion.className = 'editor-middle-region flex flex-1 min-h-0 overflow-hidden relative';
    middleRegion.style.display = 'flex';
    middleRegion.style.flexDirection = 'row';
    middleRegion.style.flex = '1 1 0%';
    middleRegion.style.minHeight = '0';
    middleRegion.style.width = '100%';
    middleRegion.style.height = '100%';
    middleRegion.style.overflow = 'hidden';
    middleRegion.style.position = 'relative';

    // 2a. Left Filmstrip Sidebar
    const filmstripMount = document.createElement('div');
    filmstripMount.className = 'editor-filmstrip-mount flex-none h-full';
    filmstripMount.style.flex = '0 0 200px';
    filmstripMount.style.width = '200px';
    filmstripMount.style.minWidth = '200px';
    filmstripMount.style.maxWidth = '200px';
    filmstripMount.style.height = '100%';
    filmstripMount.style.borderRight = '1px solid #27272a';
    filmstripMount.style.background = '#18181b';
    filmstripMount.style.overflow = 'hidden';
    filmstripMount.style.display = 'flex';
    filmstripMount.style.flexDirection = 'column';
    filmstripMount.dataset.mode = this.mode;
    filmstripMount.classList.toggle('mode-song', this.isSongMode());
    middleRegion.appendChild(filmstripMount);

    this.filmstrip = new FilmstripSidebar(filmstripMount, {
      onSelectSlide: (idx) => this.switchSlide(idx),
      onReorderSlides: (from, to) => this.reorderSlides(from, to),
      onDuplicateSlide: (idx) => this.duplicateSlide(idx),
      onDeleteSlide: (idx) => this.deleteSlide(idx),
      onAddSlide: () => this.addNewSlide(),
      onUpdateSlideTag: (idx, tag) => this.setSlideTag(idx, tag),
      onUpdateSlideDuration: (idx, duration) => this.setSlideDuration(idx, duration)
    }, this.mode);

    // 2b. Center Canvas Viewport
    const canvasViewport = document.createElement('div');
    canvasViewport.className = 'editor-canvas-viewport flex-1 h-full relative overflow-hidden bg-[#0c0c0e]';
    canvasViewport.style.flex = '1 1 0%';
    canvasViewport.style.minWidth = '0';
    canvasViewport.style.height = '100%';
    canvasViewport.style.width = 'auto';
    canvasViewport.style.position = 'relative';
    canvasViewport.style.overflow = 'hidden';
    canvasViewport.style.background = '#0c0c0e';
    middleRegion.appendChild(canvasViewport);

    this.canvas = new EditorCanvas(canvasViewport);
    this.canvas.onSlideChange((updated) => {
      this.slides[this.activeIndex] = updated;
      this.history.pushState(updated);
      this.syncUIState();
    });

    // 2c. Right Sidebar: title/type/close header (relocated here from its old
    // spot as a full-width top bar — see #studio-sidebar-header in index.html)
    // stacked above the Properties Panel, both in one fixed-width column.
    const sidebarColumn = document.createElement('div');
    sidebarColumn.className = 'editor-sidebar-column flex-none w-[260px] h-full';
    sidebarColumn.style.flex = '0 0 260px';
    sidebarColumn.style.width = '260px';
    sidebarColumn.style.minWidth = '260px';
    sidebarColumn.style.maxWidth = '260px';
    sidebarColumn.style.height = '100%';
    sidebarColumn.style.display = 'flex';
    sidebarColumn.style.flexDirection = 'column';
    sidebarColumn.style.borderLeft = '1px solid #27272a';
    sidebarColumn.style.background = '#18181b';
    sidebarColumn.style.overflow = 'hidden';
    middleRegion.appendChild(sidebarColumn);

    // The header lives as static markup in index.html (its inputs/buttons are
    // wired up by app_ui.ts via getElementById, which keeps working no matter
    // where in the DOM the node ends up) — move the real node in rather than
    // cloning it, so those existing listeners stay attached.
    const sidebarHeader = document.getElementById('studio-sidebar-header');
    if (sidebarHeader) {
      sidebarHeader.style.flex = '0 0 auto';
      sidebarColumn.appendChild(sidebarHeader);
    }

    const propsMount = document.createElement('div');
    propsMount.className = 'editor-props-mount flex-1 min-h-0 overflow-y-auto bg-[#18181b]';
    propsMount.style.flex = '1 1 0%';
    propsMount.style.minHeight = '0';
    propsMount.style.width = '100%';
    propsMount.style.background = '#18181b';
    propsMount.style.overflowY = 'auto';
    propsMount.style.display = 'flex';
    propsMount.style.flexDirection = 'column';
    sidebarColumn.appendChild(propsMount);

    this.propertiesPanel = new PropertiesPanel(propsMount);
    this.propertiesPanel.onChange((updated) => {
      this.slides[this.activeIndex] = updated;
      this.history.pushState(updated);
      this.canvas.render();
      this.syncUIState();
    });
    this.propertiesPanel.onOpenBackgroundPicker((tab) => this.openBackgroundGridPicker(tab));

    this.container.appendChild(middleRegion);

    // 3. Bottom Notes Pane Mount
    const notesMount = document.createElement('div');
    notesMount.className = 'editor-notes-mount flex-none';
    notesMount.style.flex = '0 0 auto';
    notesMount.style.width = '100%';
    notesMount.style.zIndex = '10';
    this.container.appendChild(notesMount);

    this.notesPane = new NotesPane(notesMount);
    this.notesPane.onChange((updated) => {
      this.slides[this.activeIndex] = updated;
      this.history.pushState(updated);
      this.updateToolbarState();
    });

    // 4. Context Menu
    this.contextMenu = new EditorContextMenu({
      onCut: () => this.cutSelected(),
      onCopy: () => this.copySelected(),
      onPaste: () => this.pasteClipboard(),
      onDuplicate: () => this.duplicateSelected(),
      onDelete: () => this.deleteSelected(),
      onBringForward: () => this.arrange('bringForward'),
      onSendBackward: () => this.arrange('sendBackward'),
      onBringToFront: () => this.arrange('bringToFront'),
      onSendToBack: () => this.arrange('sendToBack'),
      onGroup: () => this.groupSelected(),
      onUngroup: () => this.ungroupSelected(),
      onSelectAll: () => this.selectAll(),
      onCropImage: () => this.startImageCrop()
    });

    // Canvas right click triggers context menu
    this.canvas.stage.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      const sel = this.getSelectedElements();
      this.contextMenu.show(e.clientX, e.clientY, {
        hasSelection: sel.length > 0,
        selectedCount: sel.length,
        hasClipboard: this.clipboard.length > 0,
        isGroup: sel.some(el => el.type === 'Group'),
        isSingleImage: sel.length === 1 && sel[0].type === 'Image'
      });
    });

    // Update selection listener on canvas
    this.canvas.selection.onChange(() => {
      this.syncUIState();
    });
  }

  private switchSlide(newIndex: number): void {
    if (newIndex === this.activeIndex || newIndex < 0 || newIndex >= this.slides.length) return;
    this.activeCropOverlay?.destroy();
    this.activeCropOverlay = null;
    this.activeIndex = newIndex;
    const cur = this.slides[this.activeIndex];
    this.history.init(cur);
    this.canvas.selection.deselectAll();
    this.canvas.setSlide(cur);
    this.propertiesPanel.bind(cur, []);
    this.notesPane.bind(cur);
    this.filmstrip.setActiveIndex(this.activeIndex);
    this.updateToolbarState();
  }

  private addNewSlide(templateId?: string): void {
    const newSlide = this.createBlankSlide(this.slides.length);
    if (templateId) {
      const template = this.templates.find(t => t.id === templateId);
      if (template) {
        newSlide.elements = regenerateElementIds(JSON.parse(JSON.stringify(template.elements)));
        applyBackgroundToSlide(newSlide, template.background ? JSON.parse(JSON.stringify(template.background)) : null);
      }
    }
    this.slides.splice(this.activeIndex + 1, 0, newSlide);
    this.filmstrip.setSlides(this.slides, this.activeIndex + 1);
    this.switchSlide(this.activeIndex + 1);
  }

  private duplicateSlide(idx: number): void {
    const target = this.slides[idx];
    if (!target) return;
    const copy: EditorSlide = JSON.parse(JSON.stringify(target));
    copy.id = `slide-${Date.now()}-${Math.random().toString(36).substr(2, 4)}`;
    this.slides.splice(idx + 1, 0, copy);
    this.filmstrip.setSlides(this.slides, idx + 1);
    this.switchSlide(idx + 1);
  }

  private deleteSlide(idx: number): void {
    if (this.slides.length <= 1) return; // Keep at least one slide
    this.slides.splice(idx, 1);
    const nextIdx = Math.min(this.activeIndex, this.slides.length - 1);
    this.filmstrip.setSlides(this.slides, nextIdx);
    this.switchSlide(nextIdx);
  }

  private reorderSlides(from: number, to: number): void {
    if (from === to) return;
    const [moved] = this.slides.splice(from, 1);
    this.slides.splice(to, 0, moved);
    this.activeIndex = to;
    this.filmstrip.setSlides(this.slides, this.activeIndex);
    this.switchSlide(to);
  }

  public setSlideTag(index: number, tag: string | null): void {
    if (index < 0 || index >= this.slides.length) return;
    const slide = this.slides[index];
    this.history.pushState(this.getActiveSlide());
    slide.tag = tag ? tag.trim() : undefined;
    if (slide.tag) {
      const tagUpper = slide.tag.toUpperCase();
      if (tagUpper.startsWith('V')) slide.label = `Verse ${slide.tag.slice(1) || '1'}`;
      else if (tagUpper === 'C' || tagUpper.startsWith('C')) slide.label = `Chorus ${slide.tag.slice(1)}`.trim();
      else if (tagUpper === 'B' || tagUpper.startsWith('B')) slide.label = `Bridge ${slide.tag.slice(1)}`.trim();
      else if (tagUpper === 'E') slide.label = 'Ending';
      else if (tagUpper === 'P') slide.label = 'Pre-Chorus';
      else if (tagUpper === 'I') slide.label = 'Intro';
      else if (tagUpper === 'O') slide.label = 'Outro';
      else slide.label = slide.tag;
    }
    this.filmstrip.setSlides(this.slides, this.activeIndex);
    this.syncUIState();
  }

  public setSlideDuration(index: number, durationSeconds: number | null): void {
    if (index < 0 || index >= this.slides.length) return;
    const slide = this.slides[index];
    this.history.pushState(this.getActiveSlide());
    slide.duration_seconds = durationSeconds && durationSeconds > 0 ? durationSeconds : undefined;
    this.filmstrip.setSlides(this.slides, this.activeIndex);
    this.syncUIState();
  }

  private insertElement(el: SlideElement): void {
    const cur = this.getActiveSlide();
    this.history.pushState(cur);
    // Assign unique z-index
    const maxZ = cur.elements.reduce((acc, item) => Math.max(acc, item.transform.z_index), 0);
    el.transform.z_index = maxZ + 1;
    cur.elements.push(el);
    this.canvas.selection.select(el.id);
    this.canvas.render();
    this.syncUIState();
  }

  private applyArchetype(archetype: string): void {
    const cur = this.getActiveSlide();
    this.history.pushState(cur);
    cur.elements = applyArchetypeToSlide(archetype, cur);
    this.canvas.selection.deselectAll();
    this.canvas.render();
    this.syncUIState();
  }

  public undo(): void {
    const cur = this.getActiveSlide();
    const prev = this.history.undo(cur);
    if (prev) {
      this.activeCropOverlay?.destroy();
      this.activeCropOverlay = null;
      this.slides[this.activeIndex] = prev;
      this.canvas.setSlide(prev);
      this.syncUIState();
    }
  }

  public redo(): void {
    const cur = this.getActiveSlide();
    const next = this.history.redo(cur);
    if (next) {
      this.activeCropOverlay?.destroy();
      this.activeCropOverlay = null;
      this.slides[this.activeIndex] = next;
      this.canvas.setSlide(next);
      this.syncUIState();
    }
  }

  private arrange(action: ArrangeAction): void {
    const cur = this.getActiveSlide();
    const selectedIds = this.canvas.selection.getSelectedIds();
    if (selectedIds.length === 0) return;

    this.history.pushState(cur);

    // Sort elements by z_index
    cur.elements.sort((a, b) => a.transform.z_index - b.transform.z_index);

    if (action === 'bringToFront') {
      const selectedEls = cur.elements.filter(e => selectedIds.includes(e.id));
      const unselectedEls = cur.elements.filter(e => !selectedIds.includes(e.id));
      cur.elements = [...unselectedEls, ...selectedEls];
    } else if (action === 'sendToBack') {
      const selectedEls = cur.elements.filter(e => selectedIds.includes(e.id));
      const unselectedEls = cur.elements.filter(e => !selectedIds.includes(e.id));
      cur.elements = [...selectedEls, ...unselectedEls];
    } else if (action === 'bringForward') {
      for (let i = cur.elements.length - 2; i >= 0; i--) {
        if (selectedIds.includes(cur.elements[i].id) && !selectedIds.includes(cur.elements[i + 1].id)) {
          const tmp = cur.elements[i];
          cur.elements[i] = cur.elements[i + 1];
          cur.elements[i + 1] = tmp;
        }
      }
    } else if (action === 'sendBackward') {
      for (let i = 1; i < cur.elements.length; i++) {
        if (selectedIds.includes(cur.elements[i].id) && !selectedIds.includes(cur.elements[i - 1].id)) {
          const tmp = cur.elements[i];
          cur.elements[i] = cur.elements[i - 1];
          cur.elements[i - 1] = tmp;
        }
      }
    }

    // Re-assign dense 1..N z-indices
    cur.elements.forEach((el, idx) => {
      el.transform.z_index = idx + 1;
    });

    this.canvas.render();
    this.syncUIState();
  }

  private align(action: AlignAction): void {
    const cur = this.getActiveSlide();
    const selected = this.getSelectedElements();
    if (selected.length === 0) return;

    this.history.pushState(cur);

    if (selected.length === 1) {
      // Align single element relative to canvas (0..1)
      const el = selected[0];
      switch (action) {
        case 'left': el.transform.x = 0.05; break;
        case 'center': el.transform.x = (1.0 - el.transform.w) / 2; break;
        case 'right': el.transform.x = 0.95 - el.transform.w; break;
        case 'top': el.transform.y = 0.05; break;
        case 'middle': el.transform.y = (1.0 - el.transform.h) / 2; break;
        case 'bottom': el.transform.y = 0.95 - el.transform.h; break;
      }
    } else {
      // Align multiple elements relative to their common bounding box
      let minX = 1.0, minY = 1.0, maxX = 0.0, maxY = 0.0;
      for (const el of selected) {
        minX = Math.min(minX, el.transform.x);
        minY = Math.min(minY, el.transform.y);
        maxX = Math.max(maxX, el.transform.x + el.transform.w);
        maxY = Math.max(maxY, el.transform.y + el.transform.h);
      }

      for (const el of selected) {
        switch (action) {
          case 'left': el.transform.x = minX; break;
          case 'center': el.transform.x = minX + (maxX - minX - el.transform.w) / 2; break;
          case 'right': el.transform.x = maxX - el.transform.w; break;
          case 'top': el.transform.y = minY; break;
          case 'middle': el.transform.y = minY + (maxY - minY - el.transform.h) / 2; break;
          case 'bottom': el.transform.y = maxY - el.transform.h; break;
        }
      }
    }

    selected.forEach(el => {
      el.transform = normalizeTransform(el.transform);
    });

    this.canvas.render();
    this.syncUIState();
  }

  private groupSelected(): void {
    const cur = this.getActiveSlide();
    const selected = this.getSelectedElements();
    if (selected.length < 2) return;

    this.history.pushState(cur);

    let minX = 1.0, minY = 1.0, maxX = 0.0, maxY = 0.0, maxZ = 0;
    const selectedIds = new Set(selected.map(e => e.id));

    cur.elements = cur.elements.filter(el => {
      if (selectedIds.has(el.id)) {
        minX = Math.min(minX, el.transform.x);
        minY = Math.min(minY, el.transform.y);
        maxX = Math.max(maxX, el.transform.x + el.transform.w);
        maxY = Math.max(maxY, el.transform.y + el.transform.h);
        maxZ = Math.max(maxZ, el.transform.z_index);
        return false;
      }
      return true;
    });

    const groupEl: SlideElement = {
      type: 'Group',
      id: `group-${Date.now()}`,
      transform: {
        x: Math.max(0, Math.min(1, minX)),
        y: Math.max(0, Math.min(1, minY)),
        w: Math.max(0.01, Math.min(1, maxX - minX)),
        h: Math.max(0.01, Math.min(1, maxY - minY)),
        rotation_deg: 0,
        z_index: maxZ,
        locked: false,
        opacity: 1.0
      },
      children: selected
    };

    cur.elements.push(groupEl);
    this.canvas.selection.select(groupEl.id);
    this.canvas.render();
    this.syncUIState();
  }

  private ungroupSelected(): void {
    const cur = this.getActiveSlide();
    const selected = this.getSelectedElements();
    const group = selected.find(e => e.type === 'Group');
    if (!group || group.type !== 'Group') return;

    this.history.pushState(cur);

    const pos = cur.elements.findIndex(e => e.id === group.id);
    if (pos !== -1) {
      cur.elements.splice(pos, 1, ...group.children);
    }

    this.canvas.selection.selectMultiple(group.children.map(c => c.id));
    this.canvas.render();
    this.syncUIState();
  }

  private copySelected(): void {
    const sel = this.getSelectedElements();
    if (sel.length > 0) {
      this.clipboard = JSON.parse(JSON.stringify(sel));
    }
  }

  private cutSelected(): void {
    this.copySelected();
    this.deleteSelected();
  }

  private pasteClipboard(): void {
    if (this.clipboard.length === 0) return;
    const cur = this.getActiveSlide();
    this.history.pushState(cur);

    const pastedIds: string[] = [];
    for (const item of this.clipboard) {
      const copy: SlideElement = JSON.parse(JSON.stringify(item));
      copy.id = `${copy.type.toLowerCase()}-${Date.now()}-${Math.random().toString(36).substr(2, 4)}`;
      copy.transform.x = Math.min(0.9, copy.transform.x + 0.03);
      copy.transform.y = Math.min(0.9, copy.transform.y + 0.03);
      const maxZ = cur.elements.reduce((acc, el) => Math.max(acc, el.transform.z_index), 0);
      copy.transform.z_index = maxZ + 1;
      cur.elements.push(copy);
      pastedIds.push(copy.id);
    }

    this.canvas.selection.selectMultiple(pastedIds);
    this.canvas.render();
    this.syncUIState();
  }

  private duplicateSelected(): void {
    this.copySelected();
    this.pasteClipboard();
  }

  private deleteSelected(): void {
    const cur = this.getActiveSlide();
    const selectedIds = new Set(this.canvas.selection.getSelectedIds());
    if (selectedIds.size === 0) return;

    this.history.pushState(cur);
    cur.elements = cur.elements.filter(e => !selectedIds.has(e.id));
    this.canvas.selection.deselectAll();
    this.canvas.render();
    this.syncUIState();
  }

  private selectAll(): void {
    const cur = this.getActiveSlide();
    this.canvas.selection.selectMultiple(cur.elements.map(e => e.id));
    this.canvas.render();
    this.syncUIState();
  }

  private nudgeSelected(dx: number, dy: number): void {
    const selected = this.getSelectedElements();
    if (selected.length === 0) return;
    const cur = this.getActiveSlide();

    this.history.pushState(cur);
    for (const el of selected) {
      el.transform.x += dx;
      el.transform.y += dy;
      el.transform = normalizeTransform(el.transform);
    }
    this.canvas.render();
    this.syncUIState();
  }

  private getSelectedElements(): SlideElement[] {
    const cur = this.getActiveSlide();
    const ids = this.canvas.selection.getSelectedIds();
    return cur.elements.filter(e => ids.includes(e.id));
  }

  private syncUIState(): void {
    const cur = this.getActiveSlide();
    const sel = this.getSelectedElements();
    this.propertiesPanel.bind(cur, sel);
    this.notesPane.bind(cur);
    this.filmstrip.setSlides(this.slides, this.activeIndex);
    this.updateToolbarState();
  }

  private updateToolbarState(): void {
    const cur = this.getActiveSlide();
    const sel = this.getSelectedElements();
    const hasGroup = sel.some(e => e.type === 'Group');
    this.toolbar.updateHistoryState(this.history.canUndo(), this.history.canRedo());
    this.toolbar.updateSelectionState(sel.length, hasGroup);
    this.toolbar.updateZoom(this.canvas.getZoom());
    this.toolbar.updateBackgroundState(cur?.background_v2, cur?.background);

    const textElements = sel.filter(e => e.type === 'TextBlock') as (SlideElement & { type: 'TextBlock' })[];
    if (textElements.length > 0) {
      const primary = textElements[0];
      const run0 = primary.block.runs[0] || { text: '' };
      this.toolbar.updateTextFormattingState({
        fontFamily: run0.font_family,
        fontSize: run0.font_size_pt,
        bold: run0.bold,
        italic: run0.italic,
        underline: run0.underline,
        strike: run0.strike,
        baselineShift: run0.baseline_shift,
        bulletKind: primary.block.paragraph_style?.bullet_kind,
        indentLevel: primary.block.paragraph_style?.indent_level,
        color: run0.color,
        align: primary.block.paragraph_style?.align,
        autofit: primary.block.autofit,
        hasTextSelection: true
      });
    } else {
      this.toolbar.updateTextFormattingState({
        hasTextSelection: false
      });
    }
  }

  public applyTextStyle(update: TextStyleUpdate): void {
    const cur = this.getActiveSlide();
    const sel = this.getSelectedElements();
    let textElements = sel.filter(e => e.type === 'TextBlock') as (SlideElement & { type: 'TextBlock' })[];

    if (textElements.length === 0) {
      const allText = cur.elements.filter(e => e.type === 'TextBlock') as (SlideElement & { type: 'TextBlock' })[];
      if (allText.length === 1) {
        textElements = allText;
      }
    }

    if (textElements.length === 0) return;

    const hasRunLevelUpdate = update.font_family !== undefined || update.font_size_pt !== undefined ||
      update.bold !== undefined || update.italic !== undefined || update.underline !== undefined ||
      update.strike !== undefined || update.baseline_shift !== undefined || update.color !== undefined;
    const hasParagraphUpdate = update.align !== undefined || update.line_height !== undefined || update.autofit !== undefined ||
      update.bullet_kind !== undefined || update.indent_level !== undefined || update.indent_delta !== undefined;

    this.history.pushState(cur);

    // If the user is actively editing one of these text boxes and has a live,
    // non-collapsed selection inside it, restyle just that selection (split
    // into its own run) instead of the whole box — this is what makes
    // "bold one word" work rather than bolding the entire text box. Run-level
    // properties only; paragraph-level ones (align/line-height/autofit) below
    // always apply to the whole box regardless of selection.
    let appliedToSelection = false;
    const editingId = this.canvas.getEditingElementId();
    if (hasRunLevelUpdate && editingId) {
      const editingEl = textElements.find(e => e.id === editingId);
      // Note: renderTextBlockDOM adds 'slide-element-text-content' directly
      // onto the same [data-element-id] node, not a child — no descendant
      // combinator here.
      const editingNode = this.canvas.stage.querySelector(
        `[data-element-id="${editingId}"].slide-element-text-content`
      ) as HTMLElement | null;
      const winSel = window.getSelection();
      if (editingEl && editingNode && winSel && !winSel.isCollapsed && editingNode.contains(winSel.anchorNode)) {
        const runStyle: Partial<TextRun> = {};
        if (update.font_family !== undefined) runStyle.font_family = update.font_family;
        if (update.font_size_pt !== undefined) runStyle.font_size_pt = update.font_size_pt;
        if (update.bold !== undefined) runStyle.bold = update.bold;
        if (update.italic !== undefined) runStyle.italic = update.italic;
        if (update.underline !== undefined) runStyle.underline = update.underline;
        if (update.strike !== undefined) runStyle.strike = update.strike;
        if (update.baseline_shift !== undefined) runStyle.baseline_shift = update.baseline_shift;
        if (update.color !== undefined) runStyle.color = update.color;

        if (applyStyleToSelection(editingNode, runStyle)) {
          editingEl.block.runs = reconcileTextBlockFromDOM(editingNode);
          appliedToSelection = true;
        }
      }
    }

    if (!appliedToSelection && hasRunLevelUpdate) {
      for (const el of textElements) {
        const tb = el.block;
        if (!tb.runs[0]) tb.runs.push({ text: '' });
        for (const r of tb.runs) {
          if (update.font_family !== undefined) r.font_family = update.font_family;
          if (update.font_size_pt !== undefined) r.font_size_pt = update.font_size_pt;
          if (update.bold !== undefined) r.bold = update.bold;
          if (update.italic !== undefined) r.italic = update.italic;
          if (update.underline !== undefined) r.underline = update.underline;
          if (update.strike !== undefined) r.strike = update.strike;
          if (update.baseline_shift !== undefined) r.baseline_shift = update.baseline_shift;
          if (update.color !== undefined) r.color = update.color;
        }
      }
    }

    if (hasParagraphUpdate) {
      for (const el of textElements) {
        const tb = el.block;
        if (!tb.paragraph_style) tb.paragraph_style = {};
        if (update.align !== undefined) {
          tb.paragraph_style.align = update.align;
        }
        if (update.line_height !== undefined) {
          tb.paragraph_style.line_height = update.line_height;
        }
        if (update.autofit !== undefined) {
          tb.autofit = update.autofit;
        }
        if (update.bullet_kind !== undefined) {
          tb.paragraph_style.bullet_kind = update.bullet_kind;
          if (update.bullet_kind === 'disc' || update.bullet_kind === 'decimal') {
            if (!tb.paragraph_style.align || tb.paragraph_style.align === 'center') {
              tb.paragraph_style.align = 'left';
            }
          }
          if (tb.runs.length > 0) {
            tb.runs[0].text = formatBulletLines(tb.runs[0].text, update.bullet_kind);
          }
        }
        if (update.indent_level !== undefined) {
          tb.paragraph_style.indent_level = Math.max(0, Math.min(8, update.indent_level));
        }
        if (update.indent_delta !== undefined) {
          const curIndent = tb.paragraph_style.indent_level || 0;
          tb.paragraph_style.indent_level = Math.max(0, Math.min(8, curIndent + update.indent_delta));
        }
      }
    }

    // Re-rendering the canvas while a contenteditable box is mid-edit would
    // tear down its DOM and lose focus/cursor position — skip it when we just
    // restyled a live selection in place; the DOM there is already correct.
    if (!appliedToSelection) {
      this.canvas.render();
    }
    this.syncUIState();
  }

  public updateSlideBackground(bg: SlideBackground | null): void {
    const cur = this.getActiveSlide();
    this.history.pushState(cur);
    applyBackgroundToSlide(cur, bg);
    this.canvas.render();
    this.syncUIState();
  }

  public openBackgroundGridPicker(initialTab?: 'images' | 'css' | 'videos'): void {
    const cur = this.getActiveSlide();
    let currentImg: string | null = null;
    let currentVid: string | null = null;
    if (cur.background_v2?.kind === 'Image') {
      currentImg = cur.background_v2.data.file_path;
    } else if (cur.background_v2?.kind === 'Video') {
      currentVid = cur.background_v2.data.file_path;
    } else if (cur.background && isVideoBackground(cur.background)) {
      currentVid = cur.background;
    } else if (cur.background && (cur.background.startsWith('url(') || cur.background.includes('/media/images/'))) {
      const match = cur.background.match(/url\(['"]?([^'"]+)['"]?\)/);
      currentImg = match ? match[1] : null;
    }

    const defaultTab = initialTab || (currentVid ? 'videos' : 'images');

    openMediaImagePicker({
      currentImage: currentImg,
      currentVideo: currentVid,
      initialTab: defaultTab,
      isStudio: true,
      onSelectSlide: (filePath, cssBg, _name, isVideo, isLooping) => {
        this.history.pushState(cur);
        applyPickerBackgroundToSlide(cur, filePath, cssBg, isVideo, isLooping);
        this.canvas.render();
        this.syncUIState();
      }
    });
  }

  private async refreshTemplates(): Promise<void> {
    try {
      const res = await fetch('/api/slide-templates');
      if (!res.ok) return;
      this.templates = await res.json();
      this.toolbar.setTemplates(this.templates.map(t => ({ id: t.id, name: t.name, category: t.category })));
    } catch (_) {
      // Templates are a convenience feature — silently skip if unreachable.
    }
  }

  private async saveActiveSlideAsTemplate(): Promise<void> {
    const name = prompt('Name this template:', 'My Template');
    if (!name || !name.trim()) return;
    const cur = this.getActiveSlide();

    const template: SlideTemplate = {
      id: `tmpl-${Date.now()}-${Math.random().toString(36).substr(2, 6)}`,
      name: name.trim(),
      category: null,
      elements: JSON.parse(JSON.stringify(cur.elements)),
      background: cur.background_v2 ? JSON.parse(JSON.stringify(cur.background_v2)) : null
    };

    try {
      const res = await fetch('/api/slide-templates', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(template)
      });
      if (res.ok) {
        await this.refreshTemplates();
      }
    } catch (_) {
      // Non-fatal — the slide itself is unaffected either way.
    }
  }

  /** Applying a template fully replaces the active slide's elements/background
   * (with fresh element ids) rather than merging — a positioned-layout merge
   * has no sensible default (overlap/z-order conflicts), so full-replace is
   * the primary action here. */
  private applyTemplate(templateId: string): void {
    const template = this.templates.find(t => t.id === templateId);
    if (!template) return;
    const cur = this.getActiveSlide();

    this.history.pushState(cur);
    this.canvas.selection.deselectAll();
    cur.elements = regenerateElementIds(JSON.parse(JSON.stringify(template.elements)));
    applyBackgroundToSlide(cur, template.background ? JSON.parse(JSON.stringify(template.background)) : null);

    this.canvas.render();
    this.syncUIState();
  }

  private startImageCrop(): void {
    const sel = this.getSelectedElements();
    if (sel.length !== 1 || sel[0].type !== 'Image') return;
    const el = sel[0] as SlideElement & { type: 'Image' };

    this.endImageCrop();

    const hostNode = this.canvas.stage.querySelector(`[data-element-id="${el.id}"]`) as HTMLElement | null;
    if (!hostNode) return;

    const initialCrop = el.crop ?? { x: 0, y: 0, w: 1, h: 1 };
    const overlay = new ImageCropOverlay(hostNode, el.file_path, initialCrop);
    overlay.onChange((crop, isEnd) => {
      if (isEnd) {
        this.history.pushState(this.getActiveSlide());
        el.crop = { ...crop };
        this.notifyCropChange();
      }
    });
    overlay.onDone(() => this.endImageCrop());
    this.activeCropOverlay = overlay;
  }

  private endImageCrop(): void {
    if (this.activeCropOverlay) {
      this.activeCropOverlay.destroy();
      this.activeCropOverlay = null;
      this.canvas.render();
      this.syncUIState();
    }
  }

  private notifyCropChange(): void {
    // Update the properties panel's numeric readout live without tearing
    // down the crop overlay itself (canvas.render() would remove it, since
    // it's mounted as a child of the image's own canvas node).
    this.syncUIState();
  }

  private bindKeyboardShortcuts(): void {
    this.boundKeydownHandler = (e: KeyboardEvent) => {
      // Don't intercept typing in inputs / textareas / contenteditables
      const activeEl = document.activeElement as HTMLElement;
      const isInput = activeEl && (activeEl.tagName === 'INPUT' || activeEl.tagName === 'TEXTAREA' || activeEl.isContentEditable);

      if (e.key === 'Escape') {
        if (isInput) activeEl.blur();
        this.canvas.selection.deselectAll();
        this.canvas.render();
        this.syncUIState();
        return;
      }

      if (isInput) return;

      const isCtrl = e.ctrlKey || e.metaKey;

      if (isCtrl && !e.shiftKey && e.key.toLowerCase() === 'z') {
        e.preventDefault();
        this.undo();
      } else if (isCtrl && (e.key.toLowerCase() === 'y' || (e.shiftKey && e.key.toLowerCase() === 'z'))) {
        e.preventDefault();
        this.redo();
      } else if (isCtrl && e.key.toLowerCase() === 'c') {
        e.preventDefault();
        this.copySelected();
      } else if (isCtrl && e.key.toLowerCase() === 'x') {
        e.preventDefault();
        this.cutSelected();
      } else if (isCtrl && e.key.toLowerCase() === 'v') {
        e.preventDefault();
        this.pasteClipboard();
      } else if (isCtrl && e.key.toLowerCase() === 'd') {
        e.preventDefault();
        this.duplicateSelected();
      } else if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        this.deleteSelected();
      } else if (isCtrl && e.key.toLowerCase() === 'a') {
        e.preventDefault();
        this.selectAll();
      } else if (isCtrl && !e.shiftKey && e.key.toLowerCase() === 'g') {
        e.preventDefault();
        this.groupSelected();
      } else if (isCtrl && e.shiftKey && e.key.toLowerCase() === 'g') {
        e.preventDefault();
        this.ungroupSelected();
      } else if (isCtrl && e.key === ']') {
        e.preventDefault();
        this.arrange(e.shiftKey ? 'bringToFront' : 'bringForward');
      } else if (isCtrl && e.key === '[') {
        e.preventDefault();
        this.arrange(e.shiftKey ? 'sendToBack' : 'sendBackward');
      } else if (e.key.startsWith('Arrow')) {
        e.preventDefault();
        const step = e.shiftKey ? 0.05 : 0.01;
        if (e.key === 'ArrowLeft') this.nudgeSelected(-step, 0);
        else if (e.key === 'ArrowRight') this.nudgeSelected(step, 0);
        else if (e.key === 'ArrowUp') this.nudgeSelected(0, -step);
        else if (e.key === 'ArrowDown') this.nudgeSelected(0, step);
      }
    };

    window.addEventListener('keydown', this.boundKeydownHandler);
  }

  private createBlankSlide(index: number): EditorSlide {
    return {
      id: `slide-${Date.now()}-${index}`,
      text: '',
      elements: [],
      speaker_notes: '',
      ccli_metadata: {},
      slide_document_version: 2
    };
  }
}
