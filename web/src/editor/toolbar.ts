import { SlideElement, EditorSlide, SlideBackground, TextStyleUpdate, SOLID_PALETTE_PRESETS, GRADIENT_PRESETS, ANIMATED_PATTERN_PRESETS, FONT_FAMILY_GROUPS } from './types';
import { createDefaultTextBlock } from './text_block';
import { createDefaultShape } from './shape_library';
import { createDefaultLine } from './line_tool';
import { createDefaultTable } from './table';
import { applyResolvedBackground, backgroundPresetDataString } from '../core/presentation_helpers';

export type ArrangeAction = 'bringToFront' | 'sendToBack' | 'bringForward' | 'sendBackward';
export type AlignAction = 'left' | 'center' | 'right' | 'top' | 'middle' | 'bottom';

export interface ToolbarCallbacks {
  onInsertElement: (el: SlideElement) => void;
  onApplyArchetype: (archetype: string) => void;
  onUndo: () => void;
  onRedo: () => void;
  onArrange: (action: ArrangeAction) => void;
  onAlign: (action: AlignAction) => void;
  onGroup: () => void;
  onUngroup: () => void;
  onZoomChange: (zoom: number) => void;
  onZoomToFit: () => void;
  onAddSlide?: () => void;
  onDuplicateSlide?: () => void;
  onDeleteSlide?: () => void;
  onUpdateTextStyle?: (update: TextStyleUpdate) => void;
  onUpdateBackground?: (bg: SlideBackground | null) => void;
  onOpenBackgroundPicker?: () => void;
  onSaveAsTemplate?: () => void;
  onApplyTemplate?: (templateId: string) => void;
}

export interface SlideTemplateSummary {
  id: string;
  name: string;
  category?: string | null;
}

/**
 * The ribbon's framework — every group, button, select, and divider — is
 * static markup in index.html (#slide-editor-ribbon [ARCH:slide-editor-shell]).
 * This class binds
 * to that existing DOM by id and wires up behavior; it does not construct
 * the toolbar's structure. The two exceptions are content that's genuinely
 * data-driven rather than fixed layout: the font-family <option> list (from
 * FONT_FAMILY_GROUPS) and the Background popover (from
 * SOLID_PALETTE_PRESETS/GRADIENT_PRESETS) — both still get their contents
 * filled in by JS, onto elements/anchors that already exist in the HTML.
 */
export class EditorToolbar {
  private containerEl: HTMLElement;
  private callbacks: ToolbarCallbacks;
  private undoBtn!: HTMLButtonElement;
  private redoBtn!: HTMLButtonElement;
  private groupBtn!: HTMLButtonElement;
  private ungroupBtn!: HTMLButtonElement;
  private zoomLabel!: HTMLElement;

  // Background Picker Controls
  private bgBtn!: HTMLButtonElement;
  private bgAnchorEl!: HTMLElement;
  private bgPopoverEl: HTMLElement | null = null;
  private currentBgPreview!: HTMLElement;

  // Text Formatting Controls
  private textClusterEl!: HTMLElement;
  private fontFamilySelect!: HTMLSelectElement;
  private fontSizeInput!: HTMLInputElement;
  private boldBtn!: HTMLButtonElement;
  private italicBtn!: HTMLButtonElement;
  private underlineBtn!: HTMLButtonElement;
  private strikeBtn!: HTMLButtonElement;
  private subscriptBtn!: HTMLButtonElement;
  private superscriptBtn!: HTMLButtonElement;
  private bulletListBtn!: HTMLButtonElement;
  private numberListBtn!: HTMLButtonElement;
  private indentDecBtn!: HTMLButtonElement;
  private indentIncBtn!: HTMLButtonElement;
  private textColorInput!: HTMLInputElement;
  private textColorBar!: HTMLElement;
  private alignLeftBtn!: HTMLButtonElement;
  private alignCenterBtn!: HTMLButtonElement;
  private alignRightBtn!: HTMLButtonElement;
  private alignJustifyBtn!: HTMLButtonElement;
  private autofitBtn!: HTMLButtonElement;

  // Template Controls
  private templateSelect!: HTMLSelectElement;

  constructor(containerEl: HTMLElement, callbacks: ToolbarCallbacks) {
    this.containerEl = containerEl;
    this.callbacks = callbacks;
    this.bind();
  }

  public updateHistoryState(canUndo: boolean, canRedo: boolean): void {
    if (this.undoBtn) this.undoBtn.disabled = !canUndo;
    if (this.redoBtn) this.redoBtn.disabled = !canRedo;
  }

  public updateSelectionState(selectedCount: number, hasGroup: boolean): void {
    if (this.groupBtn) this.groupBtn.disabled = selectedCount < 2;
    if (this.ungroupBtn) this.ungroupBtn.disabled = !hasGroup;
  }

  public updateZoom(zoom: number): void {
    if (this.zoomLabel) {
      this.zoomLabel.textContent = `${Math.round(zoom * 100)}%`;
    }
  }

  public setTemplates(templates: SlideTemplateSummary[]): void {
    if (!this.templateSelect) return;
    this.templateSelect.innerHTML = '<option value="" disabled selected>Apply Template…</option>';
    const byCategory = new Map<string, SlideTemplateSummary[]>();
    for (const t of templates) {
      const cat = t.category || 'Templates';
      if (!byCategory.has(cat)) byCategory.set(cat, []);
      byCategory.get(cat)!.push(t);
    }
    for (const [cat, group] of byCategory) {
      const optgroup = document.createElement('optgroup');
      optgroup.label = cat;
      for (const t of group) {
        const opt = document.createElement('option');
        opt.value = t.id;
        opt.textContent = t.name;
        optgroup.appendChild(opt);
      }
      this.templateSelect.appendChild(optgroup);
    }
    this.templateSelect.disabled = templates.length === 0;
  }

  public updateTextFormattingState(state: {
    fontFamily?: string;
    fontSize?: number;
    bold?: boolean;
    italic?: boolean;
    underline?: boolean;
    strike?: boolean;
    baselineShift?: string;
    bulletKind?: string;
    indentLevel?: number;
    color?: string;
    align?: string;
    autofit?: boolean;
    hasTextSelection: boolean;
  }): void {
    if (!this.textClusterEl) return;
    const enabled = state.hasTextSelection;
    this.fontFamilySelect.disabled = !enabled;
    this.fontSizeInput.disabled = !enabled;
    this.boldBtn.disabled = !enabled;
    this.italicBtn.disabled = !enabled;
    this.underlineBtn.disabled = !enabled;
    this.strikeBtn.disabled = !enabled;
    this.subscriptBtn.disabled = !enabled;
    this.superscriptBtn.disabled = !enabled;
    this.bulletListBtn.disabled = !enabled;
    this.numberListBtn.disabled = !enabled;
    this.indentDecBtn.disabled = !enabled;
    this.indentIncBtn.disabled = !enabled;
    this.textColorInput.disabled = !enabled;
    this.alignLeftBtn.disabled = !enabled;
    this.alignCenterBtn.disabled = !enabled;
    this.alignRightBtn.disabled = !enabled;
    this.alignJustifyBtn.disabled = !enabled;
    this.autofitBtn.disabled = !enabled;

    if (enabled) {
      if (state.fontFamily) this.fontFamilySelect.value = state.fontFamily;
      if (state.fontSize) this.fontSizeInput.value = Math.round(state.fontSize).toString();
      this.boldBtn.classList.toggle('active', !!state.bold);
      this.italicBtn.classList.toggle('active', !!state.italic);
      this.underlineBtn.classList.toggle('active', !!state.underline);
      this.strikeBtn.classList.toggle('active', !!state.strike);
      this.subscriptBtn.classList.toggle('active', state.baselineShift === 'sub');
      this.superscriptBtn.classList.toggle('active', state.baselineShift === 'super');
      this.bulletListBtn.classList.toggle('active', state.bulletKind === 'disc');
      this.numberListBtn.classList.toggle('active', state.bulletKind === 'decimal');
      if (state.color) {
        const c = state.color.startsWith('#') ? state.color : '#ffffff';
        this.textColorInput.value = c;
        this.textColorBar.style.background = c;
      }
      this.alignLeftBtn.classList.toggle('active', state.align === 'left');
      this.alignCenterBtn.classList.toggle('active', state.align === 'center' || !state.align);
      this.alignRightBtn.classList.toggle('active', state.align === 'right');
      this.alignJustifyBtn.classList.toggle('active', state.align === 'justify');
      this.autofitBtn.classList.toggle('active', !!state.autofit);
    }
  }

  public updateBackgroundState(bg?: SlideBackground, legacyBg?: string): void {
    if (!this.currentBgPreview) return;
    if (bg) {
      if (bg.kind === 'Solid') {
        // Also covers the "pattern:<name>" animated marker (see
        // ANIMATED_PATTERN_PRESETS) — applyResolvedBackground renders the
        // real drifting background-image + animation here too, not just a
        // static swatch, so this ribbon indicator matches what's on the slide.
        applyResolvedBackground(this.currentBgPreview, bg.data);
      } else if (bg.kind === 'Gradient') {
        this.currentBgPreview.style.animation = '';
        this.currentBgPreview.style.backgroundImage = '';
        const stops = bg.data.stops.map(s => `${s.color} ${s.offset * 100}%`).join(', ');
        this.currentBgPreview.style.background = bg.data.kind === 'radial'
          ? `radial-gradient(circle, ${stops})`
          : `linear-gradient(${bg.data.angle_deg ?? 180}deg, ${stops})`;
      } else if (bg.kind === 'Image') {
        this.currentBgPreview.style.animation = '';
        this.currentBgPreview.style.backgroundImage = '';
        this.currentBgPreview.style.background = `url("${bg.data.file_path}") center/cover no-repeat`;
      } else {
        this.currentBgPreview.style.animation = '';
        this.currentBgPreview.style.backgroundImage = '';
        this.currentBgPreview.style.background = '#000000';
      }
    } else if (legacyBg) {
      applyResolvedBackground(this.currentBgPreview, legacyBg);
    } else {
      this.currentBgPreview.style.animation = '';
      this.currentBgPreview.style.backgroundImage = '';
      this.currentBgPreview.style.background = '#000000';
    }
  }

  private req<T extends HTMLElement>(id: string): T {
    const el = this.containerEl.querySelector(`#${id}`) as T | null;
    if (!el) throw new Error(`EditorToolbar: expected static element #${id} in index.html's ribbon markup, but it wasn't found`);
    return el;
  }

  private bind(): void {
    // --- Slide management ---
    if (this.callbacks.onAddSlide) {
      this.req<HTMLButtonElement>('tb-add-slide').addEventListener('click', () => this.callbacks.onAddSlide?.());
    }
    if (this.callbacks.onDuplicateSlide) {
      this.req<HTMLButtonElement>('tb-duplicate-slide').addEventListener('click', () => this.callbacks.onDuplicateSlide?.());
    }
    if (this.callbacks.onDeleteSlide) {
      this.req<HTMLButtonElement>('tb-delete-slide').addEventListener('click', () => this.callbacks.onDeleteSlide?.());
    }

    // --- History ---
    this.undoBtn = this.req('tb-undo');
    this.redoBtn = this.req('tb-redo');
    this.undoBtn.addEventListener('click', () => this.callbacks.onUndo());
    this.redoBtn.addEventListener('click', () => this.callbacks.onRedo());

    // --- Text formatting ---
    this.textClusterEl = this.req('tb-text-cluster');
    this.fontFamilySelect = this.req('tb-font-family');
    for (const group of FONT_FAMILY_GROUPS) {
      const optgroup = document.createElement('optgroup');
      optgroup.label = group.label;
      for (const font of group.fonts) {
        const opt = document.createElement('option');
        opt.value = font;
        opt.textContent = font;
        optgroup.appendChild(opt);
      }
      this.fontFamilySelect.appendChild(optgroup);
    }
    this.fontFamilySelect.addEventListener('change', () => {
      this.callbacks.onUpdateTextStyle?.({ font_family: this.fontFamilySelect.value });
    });

    this.fontSizeInput = this.req('tb-font-size');
    this.req<HTMLButtonElement>('tb-font-size-minus').addEventListener('click', () => {
      const current = parseFloat(this.fontSizeInput.value) || 36;
      const next = Math.max(8, current - 2);
      this.fontSizeInput.value = next.toString();
      this.callbacks.onUpdateTextStyle?.({ font_size_pt: next });
    });
    this.req<HTMLButtonElement>('tb-font-size-plus').addEventListener('click', () => {
      const current = parseFloat(this.fontSizeInput.value) || 36;
      const next = Math.min(150, current + 2);
      this.fontSizeInput.value = next.toString();
      this.callbacks.onUpdateTextStyle?.({ font_size_pt: next });
    });
    this.fontSizeInput.addEventListener('change', () => {
      const val = parseFloat(this.fontSizeInput.value) || 36;
      this.callbacks.onUpdateTextStyle?.({ font_size_pt: val });
    });

    this.boldBtn = this.req('tb-bold');
    this.boldBtn.addEventListener('click', () => {
      const active = this.boldBtn.classList.toggle('active');
      this.callbacks.onUpdateTextStyle?.({ bold: active });
    });
    this.italicBtn = this.req('tb-italic');
    this.italicBtn.addEventListener('click', () => {
      const active = this.italicBtn.classList.toggle('active');
      this.callbacks.onUpdateTextStyle?.({ italic: active });
    });
    this.underlineBtn = this.req('tb-underline');
    this.underlineBtn.addEventListener('click', () => {
      const active = this.underlineBtn.classList.toggle('active');
      this.callbacks.onUpdateTextStyle?.({ underline: active });
    });

    this.strikeBtn = this.req('tb-strike');
    this.strikeBtn.addEventListener('click', () => {
      const active = this.strikeBtn.classList.toggle('active');
      this.callbacks.onUpdateTextStyle?.({ strike: active });
    });

    this.subscriptBtn = this.req('tb-subscript');
    this.subscriptBtn.addEventListener('click', () => {
      const active = this.subscriptBtn.classList.toggle('active');
      if (active) this.superscriptBtn.classList.remove('active');
      this.callbacks.onUpdateTextStyle?.({ baseline_shift: active ? 'sub' : 'normal' });
    });

    this.superscriptBtn = this.req('tb-superscript');
    this.superscriptBtn.addEventListener('click', () => {
      const active = this.superscriptBtn.classList.toggle('active');
      if (active) this.subscriptBtn.classList.remove('active');
      this.callbacks.onUpdateTextStyle?.({ baseline_shift: active ? 'super' : 'normal' });
    });

    this.bulletListBtn = this.req('tb-list-bullet');
    this.bulletListBtn.addEventListener('click', () => {
      const active = this.bulletListBtn.classList.toggle('active');
      if (active) this.numberListBtn.classList.remove('active');
      this.callbacks.onUpdateTextStyle?.({ bullet_kind: active ? 'disc' : 'none' });
    });

    this.numberListBtn = this.req('tb-list-number');
    this.numberListBtn.addEventListener('click', () => {
      const active = this.numberListBtn.classList.toggle('active');
      if (active) this.bulletListBtn.classList.remove('active');
      this.callbacks.onUpdateTextStyle?.({ bullet_kind: active ? 'decimal' : 'none' });
    });

    this.indentDecBtn = this.req('tb-indent-dec');
    this.indentDecBtn.addEventListener('click', () => {
      this.callbacks.onUpdateTextStyle?.({ indent_delta: -1 });
    });

    this.indentIncBtn = this.req('tb-indent-inc');
    this.indentIncBtn.addEventListener('click', () => {
      this.callbacks.onUpdateTextStyle?.({ indent_delta: 1 });
    });

    this.textColorInput = this.req('tb-text-color');
    this.textColorBar = this.req('tb-text-color-bar');
    this.textColorInput.addEventListener('input', () => {
      this.textColorBar.style.background = this.textColorInput.value;
      this.callbacks.onUpdateTextStyle?.({ color: this.textColorInput.value });
    });

    this.alignLeftBtn = this.req('tb-align-left');
    this.alignCenterBtn = this.req('tb-align-center');
    this.alignRightBtn = this.req('tb-align-right');
    this.alignJustifyBtn = this.req('tb-align-justify');
    this.alignLeftBtn.addEventListener('click', () => {
      this.setTextAlignActive('left');
      this.callbacks.onUpdateTextStyle?.({ align: 'left' });
    });
    this.alignCenterBtn.addEventListener('click', () => {
      this.setTextAlignActive('center');
      this.callbacks.onUpdateTextStyle?.({ align: 'center' });
    });
    this.alignRightBtn.addEventListener('click', () => {
      this.setTextAlignActive('right');
      this.callbacks.onUpdateTextStyle?.({ align: 'right' });
    });
    this.alignJustifyBtn.addEventListener('click', () => {
      this.setTextAlignActive('justify');
      this.callbacks.onUpdateTextStyle?.({ align: 'justify' });
    });

    this.autofitBtn = this.req('tb-autofit');
    this.autofitBtn.addEventListener('click', () => {
      const active = this.autofitBtn.classList.toggle('active');
      this.callbacks.onUpdateTextStyle?.({ autofit: active });
    });

    // --- Insert ---
    this.req<HTMLButtonElement>('tb-insert-text').addEventListener('click', () => {
      const el = createDefaultTextBlock(
        { x: 0.1, y: 0.3, w: 0.8, h: 0.4, rotation_deg: 0, z_index: 10, locked: false, opacity: 1.0 },
        'Enter text here...'
      );
      this.callbacks.onInsertElement(el);
    });

    this.req<HTMLButtonElement>('tb-insert-image').addEventListener('click', () => {
      const url = prompt('Enter Image URL or Path:', 'https://images.unsplash.com/photo-1470225620780-dba8ba36b745?w=800');
      if (url) {
        const el: SlideElement = {
          type: 'Image',
          id: `img-${Date.now()}`,
          transform: { x: 0.15, y: 0.15, w: 0.7, h: 0.7, rotation_deg: 0, z_index: 10, locked: false, opacity: 1.0 },
          file_path: url,
          alt_text: 'Inserted Image'
        };
        this.callbacks.onInsertElement(el);
      }
    });

    this.req<HTMLButtonElement>('tb-insert-video').addEventListener('click', () => {
      const url = prompt('Enter Video File Path or Stream URL:');
      if (url) {
        const el: SlideElement = {
          type: 'Video',
          id: `vid-${Date.now()}`,
          transform: { x: 0.1, y: 0.1, w: 0.8, h: 0.8, rotation_deg: 0, z_index: 10, locked: false, opacity: 1.0 },
          file_path: url,
          loop_playback: true,
          is_muted: false,
          volume: 1.0
        };
        this.callbacks.onInsertElement(el);
      }
    });

    this.req<HTMLButtonElement>('tb-insert-line').addEventListener('click', () => {
      const line = createDefaultLine(
        'straight',
        { x: 0.2, y: 0.5, w: 0.6, h: 0.05, rotation_deg: 0, z_index: 5, locked: false, opacity: 1.0 },
        '#ffffff',
        3,
        false,
        true
      );
      this.callbacks.onInsertElement(line);
    });

    this.req<HTMLButtonElement>('tb-insert-table').addEventListener('click', () => {
      const tbl = createDefaultTable(
        3,
        3,
        { x: 0.15, y: 0.2, w: 0.7, h: 0.6, rotation_deg: 0, z_index: 5, locked: false, opacity: 1.0 }
      );
      this.callbacks.onInsertElement(tbl);
    });

    // Shapes: shown inline as a small icon grid (index.html's
    // #tb-shapes-grid) rather than behind a dropdown or popover — each
    // button inserts its shape directly on click.
    this.req<HTMLElement>('tb-shapes-grid').querySelectorAll<HTMLButtonElement>('.ribbon-shape-btn-sm').forEach(btn => {
      btn.addEventListener('click', () => {
        const shapeKind = btn.dataset.shape!;
        const shape = createDefaultShape(
          shapeKind,
          { x: 0.3, y: 0.25, w: 0.4, h: 0.5, rotation_deg: 0, z_index: 5, locked: false, opacity: 1.0 },
          '#3b82f6'
        );
        this.callbacks.onInsertElement(shape);
      });
    });

    // --- Layout ---
    const layoutSelect = this.req<HTMLSelectElement>('tb-layout-select');
    layoutSelect.addEventListener('change', () => {
      if (layoutSelect.value) {
        this.callbacks.onApplyArchetype(layoutSelect.value);
        layoutSelect.value = '';
      }
    });

    this.templateSelect = this.req('tb-template-select');
    this.templateSelect.addEventListener('change', () => {
      if (this.templateSelect.value && this.callbacks.onApplyTemplate) {
        this.callbacks.onApplyTemplate(this.templateSelect.value);
        this.templateSelect.value = '';
      }
    });

    this.req<HTMLButtonElement>('tb-save-template').addEventListener('click', () => this.callbacks.onSaveAsTemplate?.());

    this.bgBtn = this.req('tb-background');
    this.bgAnchorEl = this.bgBtn.parentElement as HTMLElement;
    this.currentBgPreview = this.req('tb-background-preview');
    this.bgBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      if (this.callbacks.onOpenBackgroundPicker) {
        this.callbacks.onOpenBackgroundPicker();
      } else {
        this.toggleBackgroundPopover();
      }
    });

    // --- Arrange ---
    this.req<HTMLButtonElement>('tb-bring-forward').addEventListener('click', () => this.callbacks.onArrange('bringForward'));
    this.req<HTMLButtonElement>('tb-send-backward').addEventListener('click', () => this.callbacks.onArrange('sendBackward'));
    this.groupBtn = this.req('tb-group');
    this.groupBtn.addEventListener('click', () => this.callbacks.onGroup());
    this.ungroupBtn = this.req('tb-ungroup');
    this.ungroupBtn.addEventListener('click', () => this.callbacks.onUngroup());

    this.req<HTMLButtonElement>('tb-align-el-left').addEventListener('click', () => this.callbacks.onAlign('left'));
    this.req<HTMLButtonElement>('tb-align-el-center').addEventListener('click', () => this.callbacks.onAlign('center'));
    this.req<HTMLButtonElement>('tb-align-el-right').addEventListener('click', () => this.callbacks.onAlign('right'));
    this.req<HTMLButtonElement>('tb-align-el-top').addEventListener('click', () => this.callbacks.onAlign('top'));
    this.req<HTMLButtonElement>('tb-align-el-middle').addEventListener('click', () => this.callbacks.onAlign('middle'));
    this.req<HTMLButtonElement>('tb-align-el-bottom').addEventListener('click', () => this.callbacks.onAlign('bottom'));

    // --- Zoom ---
    this.zoomLabel = this.req('tb-zoom-label');
    this.req<HTMLButtonElement>('tb-zoom-out').addEventListener('click', () => {
      const current = parseInt(this.zoomLabel.textContent || '100', 10) / 100;
      this.callbacks.onZoomChange(current - 0.1);
    });
    this.req<HTMLButtonElement>('tb-zoom-in').addEventListener('click', () => {
      const current = parseInt(this.zoomLabel.textContent || '100', 10) / 100;
      this.callbacks.onZoomChange(current + 0.1);
    });
    this.req<HTMLButtonElement>('tb-zoom-fit').addEventListener('click', () => this.callbacks.onZoomToFit());
  }

  private toggleBackgroundPopover(): void {
    if (this.bgPopoverEl) {
      this.closeBackgroundPopover();
      return;
    }

    // Genuinely data-driven (SOLID_PALETTE_PRESETS / GRADIENT_PRESETS), so
    // this popover's contents stay JS-built. It shares .ribbon-popover
    // (position: fixed — see style.css) with the static Shapes popover, so
    // its placement is computed from #tb-background's real screen position
    // the same way.
    const popover = document.createElement('div');
    popover.className = 'ribbon-popover w-72 text-xs text-white';
    const anchorRect = this.bgBtn.getBoundingClientRect();
    popover.style.left = `${anchorRect.left}px`;
    popover.style.top = `${anchorRect.bottom + 4}px`;

    // Header
    const head = document.createElement('div');
    head.className = 'flex items-center justify-between pb-2 mb-2 border-b border-zinc-700/70 font-semibold text-zinc-300';
    head.innerHTML = '<span>🎨 Slide Background</span>';
    const closeBtn = document.createElement('button');
    closeBtn.className = 'text-zinc-400 hover:text-white px-1 cursor-pointer';
    closeBtn.textContent = '✕';
    closeBtn.addEventListener('click', () => this.closeBackgroundPopover());
    head.appendChild(closeBtn);
    popover.appendChild(head);

    // Tab buttons: Solid | Gradient | Media
    const tabs = document.createElement('div');
    tabs.className = 'flex gap-1 mb-2.5 p-0.5 bg-zinc-900/80 rounded border border-zinc-800';

    const btnSolid = document.createElement('button');
    btnSolid.className = 'flex-1 py-1 text-center rounded text-[11px] font-medium bg-zinc-700 text-white cursor-pointer';
    btnSolid.textContent = 'Solid';

    const btnGradient = document.createElement('button');
    btnGradient.className = 'flex-1 py-1 text-center rounded text-[11px] font-medium text-zinc-400 hover:text-zinc-200 cursor-pointer';
    btnGradient.textContent = 'Gradient';

    const btnMedia = document.createElement('button');
    btnMedia.className = 'flex-1 py-1 text-center rounded text-[11px] font-medium text-zinc-400 hover:text-zinc-200 cursor-pointer';
    btnMedia.textContent = 'Media';

    const btnAnimated = document.createElement('button');
    btnAnimated.className = 'flex-1 py-1 text-center rounded text-[11px] font-medium text-zinc-400 hover:text-zinc-200 cursor-pointer';
    btnAnimated.textContent = 'Animated';

    tabs.appendChild(btnSolid);
    tabs.appendChild(btnGradient);
    tabs.appendChild(btnAnimated);
    tabs.appendChild(btnMedia);
    popover.appendChild(tabs);

    // Tab Contents Container
    const body = document.createElement('div');
    body.className = 'space-y-2';

    // 1. Solid Content
    const solidPane = document.createElement('div');
    solidPane.className = 'space-y-2';

    const swatchGrid = document.createElement('div');
    swatchGrid.className = 'grid grid-cols-5 gap-1.5';
    for (const p of SOLID_PALETTE_PRESETS) {
      const sw = document.createElement('button');
      sw.className = 'w-full h-6 rounded border border-zinc-700 hover:scale-105 transition-transform cursor-pointer';
      sw.style.background = p.color;
      sw.title = p.name;
      sw.addEventListener('click', () => {
        if (this.callbacks.onUpdateBackground) {
          this.callbacks.onUpdateBackground({ kind: 'Solid', data: p.color });
        }
        this.updateBackgroundState({ kind: 'Solid', data: p.color });
      });
      swatchGrid.appendChild(sw);
    }
    solidPane.appendChild(swatchGrid);

    const customRow = document.createElement('div');
    customRow.className = 'flex items-center gap-2 pt-1';
    customRow.innerHTML = '<span class="text-zinc-400 text-[11px]">Custom:</span>';
    const colorPicker = document.createElement('input');
    colorPicker.type = 'color';
    colorPicker.className = 'w-7 h-7 bg-transparent border-0 rounded cursor-pointer';
    colorPicker.value = '#000000';
    colorPicker.addEventListener('input', () => {
      if (this.callbacks.onUpdateBackground) {
        this.callbacks.onUpdateBackground({ kind: 'Solid', data: colorPicker.value });
      }
      this.updateBackgroundState({ kind: 'Solid', data: colorPicker.value });
    });
    customRow.appendChild(colorPicker);
    solidPane.appendChild(customRow);

    // 2. Gradient Content
    const gradPane = document.createElement('div');
    gradPane.className = 'space-y-1.5 hidden';
    for (const g of GRADIENT_PRESETS) {
      const gRow = document.createElement('button');
      gRow.className = 'w-full flex items-center gap-2 p-1 rounded hover:bg-zinc-800 border border-zinc-800 text-left cursor-pointer';
      const preview = document.createElement('div');
      preview.className = 'w-8 h-5 rounded border border-zinc-700 flex-none';
      preview.style.background = g.preview;
      const label = document.createElement('span');
      label.className = 'text-[11px] text-zinc-300 truncate';
      label.textContent = g.name;
      gRow.appendChild(preview);
      gRow.appendChild(label);
      gRow.addEventListener('click', () => {
        if (this.callbacks.onUpdateBackground) {
          this.callbacks.onUpdateBackground(g.background);
        }
        this.updateBackgroundState(g.background);
      });
      gradPane.appendChild(gRow);
    }

    // 3. Animated Content — real, running previews (not just a static swatch)
    // via applyResolvedBackground, so picking one is "what you see is what
    // you get" for the motion, not just the base color.
    const animatedPane = document.createElement('div');
    animatedPane.className = 'space-y-1.5 hidden';
    for (const a of ANIMATED_PATTERN_PRESETS) {
      const aRow = document.createElement('button');
      aRow.className = 'w-full flex items-center gap-2 p-1 rounded hover:bg-zinc-800 border border-zinc-800 text-left cursor-pointer';
      const preview = document.createElement('div');
      preview.className = 'w-8 h-5 rounded border border-zinc-700 flex-none overflow-hidden';
      applyResolvedBackground(preview, backgroundPresetDataString(a));
      const label = document.createElement('span');
      label.className = 'text-[11px] text-zinc-300 truncate';
      label.textContent = a.name;
      aRow.appendChild(preview);
      aRow.appendChild(label);
      aRow.addEventListener('click', () => {
        if (this.callbacks.onUpdateBackground) {
          this.callbacks.onUpdateBackground(a.background);
        }
        this.updateBackgroundState(a.background);
      });
      animatedPane.appendChild(aRow);
    }

    // 4. Media Content
    const mediaPane = document.createElement('div');
    mediaPane.className = 'space-y-2 hidden';
    mediaPane.innerHTML = `
      <div>
        <label class="block text-[10px] text-zinc-400 mb-1">Image or Video URL / File Path</label>
        <input type="text" id="pop-media-url" class="w-full bg-zinc-800 border border-zinc-700 rounded px-2 py-1 text-xs text-zinc-200" placeholder="https://... or /assets/...">
      </div>
      <div class="flex gap-2 pt-1">
        <button id="pop-apply-img" class="flex-1 py-1 bg-sky-600 hover:bg-sky-500 rounded font-medium text-[11px] cursor-pointer">Apply Image</button>
        <button id="pop-apply-vid" class="flex-1 py-1 bg-indigo-600 hover:bg-indigo-500 rounded font-medium text-[11px] cursor-pointer">Apply Video</button>
      </div>
    `;

    body.appendChild(solidPane);
    body.appendChild(gradPane);
    body.appendChild(animatedPane);
    body.appendChild(mediaPane);
    popover.appendChild(body);

    // Media handlers
    setTimeout(() => {
      const urlInput = mediaPane.querySelector('#pop-media-url') as HTMLInputElement;
      const applyImg = mediaPane.querySelector('#pop-apply-img') as HTMLButtonElement;
      const applyVid = mediaPane.querySelector('#pop-apply-vid') as HTMLButtonElement;
      applyImg?.addEventListener('click', () => {
        const val = urlInput.value.trim();
        if (val && this.callbacks.onUpdateBackground) {
          const bg: SlideBackground = { kind: 'Image', data: { file_path: val, opacity: 1.0 } };
          this.callbacks.onUpdateBackground(bg);
          this.updateBackgroundState(bg);
        }
      });
      applyVid?.addEventListener('click', () => {
        const val = urlInput.value.trim();
        if (val && this.callbacks.onUpdateBackground) {
          const bg: SlideBackground = { kind: 'Video', data: { file_path: val, loop_playback: true, is_muted: true } };
          this.callbacks.onUpdateBackground(bg);
          this.updateBackgroundState(bg);
        }
      });
    }, 0);

    // Switch tabs logic
    const setTab = (activeTab: 'solid' | 'gradient' | 'animated' | 'media') => {
      solidPane.classList.toggle('hidden', activeTab !== 'solid');
      gradPane.classList.toggle('hidden', activeTab !== 'gradient');
      animatedPane.classList.toggle('hidden', activeTab !== 'animated');
      mediaPane.classList.toggle('hidden', activeTab !== 'media');

      btnSolid.className = `flex-1 py-1 text-center rounded text-[11px] font-medium cursor-pointer ${activeTab === 'solid' ? 'bg-zinc-700 text-white' : 'text-zinc-400 hover:text-zinc-200'}`;
      btnGradient.className = `flex-1 py-1 text-center rounded text-[11px] font-medium cursor-pointer ${activeTab === 'gradient' ? 'bg-zinc-700 text-white' : 'text-zinc-400 hover:text-zinc-200'}`;
      btnAnimated.className = `flex-1 py-1 text-center rounded text-[11px] font-medium cursor-pointer ${activeTab === 'animated' ? 'bg-zinc-700 text-white' : 'text-zinc-400 hover:text-zinc-200'}`;
      btnMedia.className = `flex-1 py-1 text-center rounded text-[11px] font-medium cursor-pointer ${activeTab === 'media' ? 'bg-zinc-700 text-white' : 'text-zinc-400 hover:text-zinc-200'}`;
    };

    btnSolid.addEventListener('click', () => setTab('solid'));
    btnGradient.addEventListener('click', () => setTab('gradient'));
    btnAnimated.addEventListener('click', () => setTab('animated'));
    btnMedia.addEventListener('click', () => setTab('media'));

    // Footer: Reset
    const foot = document.createElement('div');
    foot.className = 'pt-2 mt-2 border-t border-zinc-700/60 flex justify-between';
    const resetBtn = document.createElement('button');
    resetBtn.className = 'text-[11px] text-rose-400 hover:text-rose-300 cursor-pointer';
    resetBtn.textContent = 'Clear / Reset';
    resetBtn.addEventListener('click', () => {
      if (this.callbacks.onUpdateBackground) {
        this.callbacks.onUpdateBackground(null);
      }
      this.updateBackgroundState(undefined);
    });
    foot.appendChild(resetBtn);
    popover.appendChild(foot);

    this.bgAnchorEl.appendChild(popover);
    this.bgPopoverEl = popover;

    // Click outside listener
    const onDocClick = (ev: MouseEvent) => {
      if (this.bgPopoverEl && !this.bgPopoverEl.contains(ev.target as Node) && ev.target !== this.bgBtn && !this.bgBtn.contains(ev.target as Node)) {
        this.closeBackgroundPopover();
        document.removeEventListener('click', onDocClick);
      }
    };
    setTimeout(() => document.addEventListener('click', onDocClick), 10);
  }

  private closeBackgroundPopover(): void {
    if (this.bgPopoverEl) {
      this.bgPopoverEl.remove();
      this.bgPopoverEl = null;
    }
  }

  private setTextAlignActive(align: string): void {
    this.alignLeftBtn.classList.toggle('active', align === 'left');
    this.alignCenterBtn.classList.toggle('active', align === 'center');
    this.alignRightBtn.classList.toggle('active', align === 'right');
    this.alignJustifyBtn.classList.toggle('active', align === 'justify');
  }
}

/**
 * Apply one of the 7 archetypes to a slide, returning an updated elements list.
 * Preserves any primary text from existing elements if available.
 */
export function applyArchetypeToSlide(archetype: string, currentSlide: EditorSlide): SlideElement[] {
  // Extract any existing text to preserve
  let existingText = '';
  for (const el of currentSlide.elements) {
    if (el.type === 'TextBlock') {
      const t = el.block.runs.map(r => r.text).join(' ').trim();
      if (t) {
        existingText = t;
        break;
      }
    }
  }
  if (!existingText && currentSlide.text) {
    existingText = currentSlide.text;
  }

  switch (archetype) {
    case 'lyric': {
      const text = existingText || 'Amazing Grace, how sweet the sound\nThat saved a wretch like me...';
      return [
        createDefaultTextBlock(
          { x: 0.05, y: 0.08, w: 0.90, h: 0.84, rotation_deg: 0, z_index: 1, locked: false, opacity: 1.0 },
          text,
          { align: 'center', line_height: 1.3 },
          true // autofit
        )
      ];
    }
    case 'title': {
      const title = existingText || 'Presentation Title';
      return [
        // 36pt (this function's general default) reads as genuinely small body
        // text against a real 1920x1080 canvas — fine for bullets/paragraphs,
        // but a title needs to be sized like one.
        createDefaultTextBlock(
          { x: 0.10, y: 0.28, w: 0.80, h: 0.25, rotation_deg: 0, z_index: 1, locked: false, opacity: 1.0 },
          title,
          { align: 'center' },
          false,
          80
        ),
        createDefaultTextBlock(
          { x: 0.15, y: 0.56, w: 0.70, h: 0.18, rotation_deg: 0, z_index: 2, locked: false, opacity: 0.85 },
          'Subtitle or Presenter Name',
          { align: 'center' },
          false,
          40
        )
      ];
    }
    case 'title_body': {
      return [
        createDefaultTextBlock(
          { x: 0.08, y: 0.08, w: 0.84, h: 0.16, rotation_deg: 0, z_index: 1, locked: false, opacity: 1.0 },
          'Section Heading',
          { align: 'left' },
          false,
          56
        ),
        createDefaultTextBlock(
          { x: 0.08, y: 0.26, w: 0.84, h: 0.66, rotation_deg: 0, z_index: 2, locked: false, opacity: 1.0 },
          existingText || '• Key announcement point\n• Details and schedule\n• Contact information',
          { align: 'left', bullet_kind: 'disc' }
        )
      ];
    }
    case 'two_column': {
      return [
        createDefaultTextBlock(
          { x: 0.06, y: 0.12, w: 0.42, h: 0.76, rotation_deg: 0, z_index: 1, locked: false, opacity: 1.0 },
          existingText || 'Left column text or bullet points...',
          { align: 'left' }
        ),
        createDefaultTextBlock(
          { x: 0.52, y: 0.12, w: 0.42, h: 0.76, rotation_deg: 0, z_index: 2, locked: false, opacity: 1.0 },
          'Right column text or comparison notes...',
          { align: 'left' }
        )
      ];
    }
    case 'scripture': {
      return [
        createDefaultTextBlock(
          { x: 0.08, y: 0.10, w: 0.84, h: 0.68, rotation_deg: 0, z_index: 1, locked: false, opacity: 1.0 },
          existingText || '"For God so loved the world, that He gave His only begotten Son, that whosoever believeth in Him should not perish, but have everlasting life."',
          { align: 'center', line_height: 1.4 },
          true
        ),
        createDefaultTextBlock(
          { x: 0.15, y: 0.82, w: 0.70, h: 0.12, rotation_deg: 0, z_index: 2, locked: false, opacity: 0.9 },
          '— John 3:16 (KJV)',
          { align: 'right' }
        )
      ];
    }
    case 'quote': {
      return [
        createDefaultTextBlock(
          { x: 0.12, y: 0.16, w: 0.76, h: 0.58, rotation_deg: 0, z_index: 1, locked: false, opacity: 1.0 },
          existingText || '“Faith is taking the first step even when you don\'t see the whole staircase.”',
          { align: 'center', line_height: 1.4 },
          true
        ),
        createDefaultTextBlock(
          { x: 0.12, y: 0.76, w: 0.76, h: 0.12, rotation_deg: 0, z_index: 2, locked: false, opacity: 0.85 },
          '— Martin Luther King Jr.',
          { align: 'right' }
        )
      ];
    }
    case 'blank':
    default:
      return [];
  }
}
