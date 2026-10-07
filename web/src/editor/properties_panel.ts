import { EditorSlide, SlideElement, SOLID_PALETTE_PRESETS, GRADIENT_PRESETS, ANIMATED_PATTERN_PRESETS, FONT_FAMILY_GROUPS } from './types';
import { applyResolvedBackground, backgroundPresetDataString, escapeHtml } from '../core/presentation_helpers';

export class PropertiesPanel {
  private panelEl: HTMLElement;
  private currentSlide: EditorSlide | null = null;
  private selectedElements: SlideElement[] = [];
  private onChangeCb: ((slide: EditorSlide) => void) | null = null;
  private onOpenBackgroundPickerCb?: (tab?: 'images' | 'css' | 'videos') => void;

  public onOpenBackgroundPicker(cb: (tab?: 'images' | 'css' | 'videos') => void): void {
    this.onOpenBackgroundPickerCb = cb;
  }

  constructor(containerEl: HTMLElement) {
    // [ARCH:slide-editor-shell] className-overwrite trap: this.panelEl IS
    // containerEl, not a child of it.
    // containerEl (slide_editor.ts's propsMount) already sits inside
    // .editor-sidebar-column, which owns the fixed 260px width and the left
    // border — this panel just needs to fill that column's remaining height
    // below #studio-sidebar-header (see the shared .editor-props-mount,
    // .editor-properties-panel rule in style.css).
    this.panelEl = containerEl;
    this.panelEl.classList.add('editor-properties-panel');
    this.panelEl.style.width = '100%';
    this.panelEl.style.flex = '1 1 0%';
    this.panelEl.style.minHeight = '0';
    this.panelEl.style.display = 'flex';
    this.panelEl.style.flexDirection = 'column';
    this.panelEl.style.background = '#18181b';
    this.panelEl.style.overflowY = 'auto';
  }

  public bind(slide: EditorSlide, selected: SlideElement[]): void {
    this.currentSlide = slide;
    this.selectedElements = selected;
    this.render();
  }

  public onChange(cb: (slide: EditorSlide) => void): void {
    this.onChangeCb = cb;
  }

  public render(): void {
    this.panelEl.innerHTML = '';

    if (!this.currentSlide) {
      this.panelEl.innerHTML = '<div class="p-4 text-xs text-slate-500">No active slide selected</div>';
      return;
    }

    if (this.selectedElements.length === 0) {
      this.renderSlideProperties();
    } else if (this.selectedElements.length === 1) {
      this.renderElementProperties(this.selectedElements[0]);
    } else {
      this.renderMultiSelectProperties();
    }
  }

  private renderSlideProperties(): void {
    const header = document.createElement('div');
    header.className = 'editor-props-header text-xs font-bold uppercase tracking-wider text-slate-400 p-3 border-b border-slate-700/60';
    header.textContent = 'Slide Properties';
    this.panelEl.appendChild(header);

    const content = document.createElement('div');
    content.className = 'p-3 space-y-4 text-xs text-slate-300';

    // Slide Background Section
    const bgGroup = document.createElement('div');
    bgGroup.className = 'space-y-2';

    const bgHeader = document.createElement('div');
    bgHeader.className = 'font-semibold text-slate-400 flex items-center justify-between';
    bgHeader.innerHTML = '<span>Slide Background</span>';
    bgGroup.appendChild(bgHeader);

    const curBg = this.currentSlide?.background_v2;
    let activeMode: 'solid' | 'gradient' | 'animated' | 'image' | 'video' = 'solid';
    if (curBg) {
      if (curBg.kind === 'Solid') activeMode = curBg.data.startsWith('pattern:') ? 'animated' : 'solid';
      else if (curBg.kind === 'Gradient') activeMode = 'gradient';
      else if (curBg.kind === 'Image') activeMode = 'image';
      else if (curBg.kind === 'Video') activeMode = 'video';
    }

    const modeTabs = document.createElement('div');
    modeTabs.className = 'grid grid-cols-5 gap-1 p-0.5 bg-slate-900 rounded border border-slate-700/80 mb-2';

    const tabButtons: Record<string, HTMLButtonElement> = {};
    const modes: Array<'solid' | 'gradient' | 'animated' | 'image' | 'video'> = ['solid', 'gradient', 'animated', 'image', 'video'];
    for (const m of modes) {
      const btn = document.createElement('button');
      btn.className = `py-1 text-[10px] font-medium rounded capitalize cursor-pointer ${m === activeMode ? 'bg-slate-700 text-white' : 'text-slate-400 hover:text-slate-200'}`;
      btn.textContent = m;
      btn.addEventListener('click', () => {
        activeMode = m;
        modes.forEach(mode => {
          tabButtons[mode].className = `py-1 text-[10px] font-medium rounded capitalize cursor-pointer ${mode === activeMode ? 'bg-slate-700 text-white' : 'text-slate-400 hover:text-slate-200'}`;
        });
        solidContainer.classList.toggle('hidden', activeMode !== 'solid');
        gradContainer.classList.toggle('hidden', activeMode !== 'gradient');
        animContainer.classList.toggle('hidden', activeMode !== 'animated');
        imgContainer.classList.toggle('hidden', activeMode !== 'image');
        vidContainer.classList.toggle('hidden', activeMode !== 'video');
      });
      tabButtons[m] = btn;
      modeTabs.appendChild(btn);
    }
    bgGroup.appendChild(modeTabs);

    // 1. Solid Mode Container
    const solidContainer = document.createElement('div');
    solidContainer.className = `space-y-2 ${activeMode !== 'solid' ? 'hidden' : ''}`;

    const swatchesGrid = document.createElement('div');
    swatchesGrid.className = 'grid grid-cols-5 gap-1.5';
    for (const p of SOLID_PALETTE_PRESETS) {
      const sw = document.createElement('button');
      sw.className = 'w-full h-5 rounded border border-slate-600 hover:scale-105 transition-transform cursor-pointer';
      sw.style.background = p.color;
      sw.title = p.name;
      sw.addEventListener('click', () => {
        this.currentSlide!.background_v2 = { kind: 'Solid', data: p.color };
        this.currentSlide!.background = p.color;
        colorInput.value = p.color;
        this.notifyChange();
      });
      swatchesGrid.appendChild(sw);
    }
    solidContainer.appendChild(swatchesGrid);

    const solidCustomRow = document.createElement('div');
    solidCustomRow.className = 'flex items-center gap-2 pt-1';
    solidCustomRow.innerHTML = '<span class="text-[10px] text-slate-500">Color:</span>';
    const colorInput = document.createElement('input');
    colorInput.type = 'color';
    colorInput.id = 'prop-slide-bg-color';
    colorInput.className = 'w-7 h-7 bg-transparent cursor-pointer rounded';
    colorInput.value = curBg?.kind === 'Solid' ? curBg.data : (this.currentSlide?.background?.startsWith('#') ? this.currentSlide.background : '#000000');
    colorInput.addEventListener('input', () => {
      this.currentSlide!.background_v2 = { kind: 'Solid', data: colorInput.value };
      this.currentSlide!.background = colorInput.value;
      this.notifyChange();
    });
    solidCustomRow.appendChild(colorInput);
    solidContainer.appendChild(solidCustomRow);

    // 2. Gradient Mode Container
    const gradContainer = document.createElement('div');
    gradContainer.className = `space-y-2 ${activeMode !== 'gradient' ? 'hidden' : ''}`;

    const gradPresetGrid = document.createElement('div');
    gradPresetGrid.className = 'space-y-1';
    for (const g of GRADIENT_PRESETS) {
      const gRow = document.createElement('button');
      gRow.className = 'w-full flex items-center gap-2 p-1 rounded hover:bg-slate-800 border border-slate-700 text-left cursor-pointer';
      const prev = document.createElement('div');
      prev.className = 'w-6 h-4 rounded border border-slate-600 flex-none';
      prev.style.background = g.preview;
      const lbl = document.createElement('span');
      lbl.className = 'text-[10px] text-slate-300 truncate';
      lbl.textContent = g.name;
      gRow.appendChild(prev);
      gRow.appendChild(lbl);
      gRow.addEventListener('click', () => {
        this.currentSlide!.background_v2 = JSON.parse(JSON.stringify(g.background));
        this.currentSlide!.background = g.preview;
        this.notifyChange();
      });
      gradPresetGrid.appendChild(gRow);
    }
    gradContainer.appendChild(gradPresetGrid);

    // 3. Animated Mode Container — real, running previews via
    // applyResolvedBackground, not just a static swatch.
    const animContainer = document.createElement('div');
    animContainer.className = `space-y-2 ${activeMode !== 'animated' ? 'hidden' : ''}`;
    const animPresetGrid = document.createElement('div');
    animPresetGrid.className = 'space-y-1';
    for (const a of ANIMATED_PATTERN_PRESETS) {
      const aRow = document.createElement('button');
      aRow.className = 'w-full flex items-center gap-2 p-1 rounded hover:bg-slate-800 border border-slate-700 text-left cursor-pointer';
      const prev = document.createElement('div');
      prev.className = 'w-6 h-4 rounded border border-slate-600 flex-none overflow-hidden';
      applyResolvedBackground(prev, backgroundPresetDataString(a));
      const lbl = document.createElement('span');
      lbl.className = 'text-[10px] text-slate-300 truncate';
      lbl.textContent = a.name;
      aRow.appendChild(prev);
      aRow.appendChild(lbl);
      aRow.addEventListener('click', () => {
        this.currentSlide!.background_v2 = JSON.parse(JSON.stringify(a.background));
        this.currentSlide!.background = (a.background as any).data;
        this.notifyChange();
      });
      animPresetGrid.appendChild(aRow);
    }
    animContainer.appendChild(animPresetGrid);

    // 4. Image Mode Container
    const imgContainer = document.createElement('div');
    imgContainer.className = `space-y-2 ${activeMode !== 'image' ? 'hidden' : ''}`;
    const imgPath = curBg?.kind === 'Image' ? curBg.data.file_path : '';
    const imgOpac = curBg?.kind === 'Image' && curBg.data.opacity !== undefined ? curBg.data.opacity : 1.0;
    imgContainer.innerHTML = `
      <div>
        <label class="text-[10px] text-slate-500 block mb-0.5">Image URL / Path</label>
        <div class="flex gap-1.5">
          <input type="text" id="prop-bg-img-path" class="flex-1 min-w-0 bg-slate-800 border border-slate-700 rounded px-1.5 py-1 text-xs text-slate-200" value="${escapeHtml(imgPath)}" placeholder="https://... or /assets/...">
          <button type="button" id="prop-bg-browse-btn" class="px-2 py-1 bg-slate-700 hover:bg-slate-600 text-slate-200 rounded text-xs font-medium cursor-pointer flex-none" title="Open Background Grid Picker">Browse…</button>
        </div>
      </div>
      <div>
        <label class="text-[10px] text-slate-500 block mb-0.5">Opacity</label>
        <input type="range" id="prop-bg-img-opacity" min="0" max="1" step="0.05" class="w-full cursor-pointer" value="${escapeHtml(imgOpac)}">
      </div>
    `;
    const imgPathInput = imgContainer.querySelector('#prop-bg-img-path') as HTMLInputElement;
    if (imgPathInput) imgPathInput.value = imgPath;
    const imgOpacInput = imgContainer.querySelector('#prop-bg-img-opacity') as HTMLInputElement;
    const browseBtn = imgContainer.querySelector('#prop-bg-browse-btn') as HTMLButtonElement | null;
    if (browseBtn) {
      browseBtn.addEventListener('click', () => {
        this.onOpenBackgroundPickerCb?.();
      });
    }
    if (imgPathInput && imgOpacInput) {
      const onImgChange = () => {
        const url = imgPathInput.value.trim();
        const op = parseFloat(imgOpacInput.value);
        if (url) {
          this.currentSlide!.background_v2 = { kind: 'Image', data: { file_path: url, opacity: op } };
          this.currentSlide!.background = `url("${url}")`;
          this.notifyChange();
        }
      };
      imgPathInput.addEventListener('change', onImgChange);
      imgOpacInput.addEventListener('input', onImgChange);
    }

    // 5. Video Mode Container
    const vidContainer = document.createElement('div');
    vidContainer.className = `space-y-2 ${activeMode !== 'video' ? 'hidden' : ''}`;
    const vidPath = curBg?.kind === 'Video' ? curBg.data.file_path : '';
    vidContainer.innerHTML = `
      <div>
        <label class="text-[10px] text-slate-500 block mb-0.5">Video URL / Path</label>
        <div class="flex gap-1.5">
          <input type="text" id="prop-bg-vid-path" class="flex-1 bg-slate-800 border border-slate-700 rounded px-1.5 py-1 text-xs text-slate-200" value="${escapeHtml(vidPath)}" placeholder="/media/loop.mp4">
          <button type="button" id="prop-bg-vid-browse" class="px-2 py-1 bg-slate-700 hover:bg-slate-600 border border-slate-600 rounded text-[11px] font-medium text-slate-200">Browse…</button>
        </div>
      </div>
      <div class="flex items-center gap-2 pt-1">
        <input type="checkbox" id="prop-bg-vid-loop" checked>
        <label for="prop-bg-vid-loop" class="text-[10px] text-slate-400">Loop Playback</label>
      </div>
    `;
    const vidPathInput = vidContainer.querySelector('#prop-bg-vid-path') as HTMLInputElement;
    if (vidPathInput) vidPathInput.value = vidPath;
    const vidLoopCheck = vidContainer.querySelector('#prop-bg-vid-loop') as HTMLInputElement;
    const vidBrowseBtn = vidContainer.querySelector('#prop-bg-vid-browse') as HTMLButtonElement | null;
    if (vidBrowseBtn) {
      vidBrowseBtn.addEventListener('click', () => {
        if (this.onOpenBackgroundPickerCb) {
          this.onOpenBackgroundPickerCb('videos');
        }
      });
    }
    if (vidPathInput && vidLoopCheck) {
      const onVidChange = () => {
        const url = vidPathInput.value.trim();
        if (url) {
          this.currentSlide!.background_v2 = {
            kind: 'Video',
            data: { file_path: url, loop_playback: vidLoopCheck.checked, is_muted: true }
          };
          this.currentSlide!.background = url;
          this.notifyChange();
        }
      };
      vidPathInput.addEventListener('change', onVidChange);
      vidLoopCheck.addEventListener('change', onVidChange);
    }

    // Reset button
    const resetRow = document.createElement('div');
    resetRow.className = 'pt-1 flex justify-end';
    const clearBgBtn = document.createElement('button');
    clearBgBtn.className = 'text-[10px] text-rose-400 hover:text-rose-300 cursor-pointer';
    clearBgBtn.textContent = 'Clear / Reset Background';
    clearBgBtn.addEventListener('click', () => {
      this.currentSlide!.background_v2 = undefined;
      this.currentSlide!.background = undefined;
      this.notifyChange();
    });
    resetRow.appendChild(clearBgBtn);

    bgGroup.appendChild(solidContainer);
    bgGroup.appendChild(gradContainer);
    bgGroup.appendChild(animContainer);
    bgGroup.appendChild(imgContainer);
    bgGroup.appendChild(vidContainer);
    bgGroup.appendChild(resetRow);
    content.appendChild(bgGroup);

    // Transition
    const transGroup = document.createElement('div');
    transGroup.innerHTML = `
      <label class="block mb-1 text-slate-400">Slide Transition</label>
      <select id="prop-slide-trans-type" class="w-full bg-slate-800 border border-slate-600 rounded p-1 text-xs text-slate-200">
        <option value="Cut">Cut (Instant)</option>
        <option value="Dissolve">Dissolve (Cross-fade)</option>
        <option value="Wipe">Wipe</option>
        <option value="Push">Push</option>
        <option value="Fade">Fade through Black</option>
      </select>
    `;
    content.appendChild(transGroup);

    const transSelect = transGroup.querySelector('#prop-slide-trans-type') as HTMLSelectElement | null;
    if (transSelect) {
      if (this.currentSlide?.transition) {
        transSelect.value = this.currentSlide.transition.kind;
      }
      transSelect.addEventListener('change', () => {
        this.currentSlide!.transition = {
          kind: transSelect.value,
          duration_ms: 400
        };
        this.notifyChange();
      });
    }

    this.panelEl.appendChild(content);
  }

  private renderElementProperties(el: SlideElement): void {
    const header = document.createElement('div');
    header.className = 'editor-props-header text-xs font-bold uppercase tracking-wider text-slate-400 p-3 border-b border-slate-700/60';
    header.textContent = `${el.type} Properties`;
    this.panelEl.appendChild(header);

    const content = document.createElement('div');
    content.className = 'p-3 space-y-3 text-xs text-slate-300';

    // 1. Transform Section
    const t = el.transform;
    const tSection = document.createElement('div');
    tSection.innerHTML = `
      <div class="font-semibold text-slate-400 mb-1">Transform</div>
      <div class="grid grid-cols-2 gap-2">
        <div>
          <label class="text-[10px] text-slate-500">X (%)</label>
          <input type="number" id="prop-tx" step="1" min="0" max="100" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(t.x * 100)}">
        </div>
        <div>
          <label class="text-[10px] text-slate-500">Y (%)</label>
          <input type="number" id="prop-ty" step="1" min="0" max="100" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(t.y * 100)}">
        </div>
        <div>
          <label class="text-[10px] text-slate-500">Width (%)</label>
          <input type="number" id="prop-tw" step="1" min="1" max="100" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(t.w * 100)}">
        </div>
        <div>
          <label class="text-[10px] text-slate-500">Height (%)</label>
          <input type="number" id="prop-th" step="1" min="1" max="100" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(t.h * 100)}">
        </div>
      </div>
      <div class="mt-2 flex items-center justify-between">
        <label class="text-[10px] text-slate-500">Opacity</label>
        <input type="range" id="prop-opacity" min="0" max="1" step="0.05" class="w-32 cursor-pointer" value="${escapeHtml(t.opacity)}">
      </div>
      <div class="mt-2 flex items-center gap-2">
        <input type="checkbox" id="prop-locked" ${t.locked ? 'checked' : ''}>
        <label for="prop-locked" class="text-xs text-slate-400">Lock Position & Size</label>
      </div>
    `;
    content.appendChild(tSection);

    // Bind Transform Inputs
    const tx = tSection.querySelector('#prop-tx') as HTMLInputElement;
    const ty = tSection.querySelector('#prop-ty') as HTMLInputElement;
    const tw = tSection.querySelector('#prop-tw') as HTMLInputElement;
    const th = tSection.querySelector('#prop-th') as HTMLInputElement;
    const opac = tSection.querySelector('#prop-opacity') as HTMLInputElement;
    const lock = tSection.querySelector('#prop-locked') as HTMLInputElement;

    const onTChange = () => {
      el.transform.x = Math.max(0, Math.min(1, parseFloat(tx.value) / 100));
      el.transform.y = Math.max(0, Math.min(1, parseFloat(ty.value) / 100));
      el.transform.w = Math.max(0.01, Math.min(1, parseFloat(tw.value) / 100));
      el.transform.h = Math.max(0.01, Math.min(1, parseFloat(th.value) / 100));
      el.transform.opacity = parseFloat(opac.value);
      el.transform.locked = lock.checked;
      this.notifyChange();
    };

    if (tx && ty && tw && th && opac && lock) {
      tx.addEventListener('change', onTChange);
      ty.addEventListener('change', onTChange);
      tw.addEventListener('change', onTChange);
      th.addEventListener('change', onTChange);
      opac.addEventListener('input', onTChange);
      lock.addEventListener('change', onTChange);
    }

    // 2. Type-specific controls
    if (el.type === 'TextBlock') {
      const tb = el.block;
      const textSection = document.createElement('div');
      textSection.className = 'border-t border-slate-700/60 pt-3 space-y-2';

      const run0 = tb.runs[0] || { text: '' };
      const curAlign = tb.paragraph_style?.align || 'center';

      textSection.innerHTML = `
        <div class="font-semibold text-slate-400 mb-1">Typography & Styling</div>

        <!-- Font Family -->
        <div>
          <label class="text-[10px] text-slate-500 block mb-0.5">Font Family</label>
          <select id="prop-font-family" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-1 text-xs text-slate-200">
            ${FONT_FAMILY_GROUPS.map(g => `<optgroup label="${g.label}">${g.fonts.map(f => `<option value="${f}" ${run0.font_family === f ? 'selected' : ''}>${f}</option>`).join('')}</optgroup>`).join('')}
          </select>
        </div>

        <!-- Font Size & Color -->
        <div class="grid grid-cols-2 gap-2">
          <div>
            <label class="text-[10px] text-slate-500 block mb-0.5">Font Size (pt)</label>
            <input type="number" id="prop-font-size" min="8" max="150" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${escapeHtml(run0.font_size_pt || 36)}">
          </div>
          <div>
            <label class="text-[10px] text-slate-500 block mb-0.5">Text Color</label>
            <input type="color" id="prop-text-color" class="w-full h-7 bg-transparent rounded cursor-pointer" value="${escapeHtml(run0.color?.startsWith('#') ? run0.color : '#ffffff')}">
          </div>
        </div>

        <!-- Styles: Bold, Italic, Underline & Alignment -->
        <div>
          <label class="text-[10px] text-slate-500 block mb-1">Styles & Alignment</label>
          <div class="flex gap-1">
            <button id="prop-style-bold" class="flex-1 py-1 rounded border border-slate-700 font-bold text-xs cursor-pointer ${run0.bold ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}">B</button>
            <button id="prop-style-italic" class="flex-1 py-1 rounded border border-slate-700 italic font-serif text-xs cursor-pointer ${run0.italic ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}">I</button>
            <button id="prop-style-underline" class="flex-1 py-1 rounded border border-slate-700 underline text-xs cursor-pointer ${run0.underline ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}">U</button>
            <button id="prop-align-left" class="flex-1 py-1 rounded border border-slate-700 text-xs cursor-pointer ${curAlign === 'left' ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}">⇤</button>
            <button id="prop-align-center" class="flex-1 py-1 rounded border border-slate-700 text-xs cursor-pointer ${curAlign === 'center' ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}">≡</button>
            <button id="prop-align-right" class="flex-1 py-1 rounded border border-slate-700 text-xs cursor-pointer ${curAlign === 'right' ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}">⇥</button>
          </div>
        </div>

        <!-- Line Height & Autofit -->
        <div class="grid grid-cols-2 gap-2 items-center">
          <div>
            <label class="text-[10px] text-slate-500 block mb-0.5">Line Height</label>
            <input type="number" id="prop-line-height" min="0.8" max="2.5" step="0.05" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${escapeHtml(tb.paragraph_style?.line_height || 1.25)}">
          </div>
          <div class="flex items-center gap-1.5 pt-3">
            <input type="checkbox" id="prop-autofit" ${tb.autofit ? 'checked' : ''}>
            <label for="prop-autofit" class="text-[11px] text-slate-400 cursor-pointer">Autofit</label>
          </div>
        </div>

        <!-- Outline -->
        <div class="border-t border-slate-700/40 pt-2">
          <div class="font-medium text-slate-400 text-[11px] mb-1">Text Outline</div>
          <div class="grid grid-cols-2 gap-2">
            <div>
              <label class="text-[10px] text-slate-500 block mb-0.5">Width (px)</label>
              <input type="number" id="prop-outline-w" min="0" max="10" step="0.5" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${escapeHtml(tb.effects?.outline?.width || 0)}">
            </div>
            <div>
              <label class="text-[10px] text-slate-500 block mb-0.5">Outline Color</label>
              <input type="color" id="prop-outline-color" class="w-full h-7 bg-transparent rounded cursor-pointer" value="${escapeHtml(tb.effects?.outline?.color || '#000000')}">
            </div>
          </div>
        </div>

        <!-- Shadow -->
        <div class="border-t border-slate-700/40 pt-2">
          <div class="font-medium text-slate-400 text-[11px] mb-1">Text Shadow</div>
          <div class="grid grid-cols-2 gap-2">
            <div>
              <label class="text-[10px] text-slate-500 block mb-0.5">Blur (px)</label>
              <input type="number" id="prop-shadow-blur" min="0" max="30" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${escapeHtml(tb.effects?.shadow?.blur || 0)}">
            </div>
            <div>
              <label class="text-[10px] text-slate-500 block mb-0.5">Shadow Color</label>
              <input type="color" id="prop-shadow-color" class="w-full h-7 bg-transparent rounded cursor-pointer" value="${escapeHtml(tb.effects?.shadow?.color?.startsWith('#') ? tb.effects.shadow.color : '#000000')}">
            </div>
          </div>
        </div>
      `;
      content.appendChild(textSection);

      const ff = textSection.querySelector('#prop-font-family') as HTMLSelectElement;
      const fs = textSection.querySelector('#prop-font-size') as HTMLInputElement;
      const tc = textSection.querySelector('#prop-text-color') as HTMLInputElement;
      const bBold = textSection.querySelector('#prop-style-bold') as HTMLButtonElement;
      const bItalic = textSection.querySelector('#prop-style-italic') as HTMLButtonElement;
      const bUnderline = textSection.querySelector('#prop-style-underline') as HTMLButtonElement;
      const bAlignL = textSection.querySelector('#prop-align-left') as HTMLButtonElement;
      const bAlignC = textSection.querySelector('#prop-align-center') as HTMLButtonElement;
      const bAlignR = textSection.querySelector('#prop-align-right') as HTMLButtonElement;
      const lh = textSection.querySelector('#prop-line-height') as HTMLInputElement;
      const af = textSection.querySelector('#prop-autofit') as HTMLInputElement;
      const ow = textSection.querySelector('#prop-outline-w') as HTMLInputElement;
      const oc = textSection.querySelector('#prop-outline-color') as HTMLInputElement;
      const sb = textSection.querySelector('#prop-shadow-blur') as HTMLInputElement;
      const sc = textSection.querySelector('#prop-shadow-color') as HTMLInputElement;

      const ensureRun = () => {
        if (!tb.runs[0]) tb.runs.push({ text: '' });
        return tb.runs[0];
      };

      if (ff) {
        ff.addEventListener('change', () => {
          const r = ensureRun();
          r.font_family = ff.value;
          this.notifyChange();
        });
      }

      if (fs) {
        fs.addEventListener('change', () => {
          const r = ensureRun();
          r.font_size_pt = parseFloat(fs.value);
          this.notifyChange();
        });
        // Scroll the wheel while hovering the font-size box to nudge it up/down
        // (a common numeric-input convention) instead of only editing via the
        // keyboard or the tiny native spinner arrows.
        fs.addEventListener('wheel', (e: WheelEvent) => {
          e.preventDefault();
          const min = fs.min ? parseFloat(fs.min) : -Infinity;
          const max = fs.max ? parseFloat(fs.max) : Infinity;
          const current = parseFloat(fs.value) || 0;
          const step = e.shiftKey ? 10 : 1;
          const next = Math.min(max, Math.max(min, current + (e.deltaY < 0 ? step : -step)));
          fs.value = String(next);
          const r = ensureRun();
          r.font_size_pt = next;
          this.notifyChange();
        }, { passive: false });
      }

      if (tc) {
        tc.addEventListener('input', () => {
          const r = ensureRun();
          r.color = tc.value;
          this.notifyChange();
        });
      }

      if (bBold) {
        bBold.addEventListener('click', () => {
          const r = ensureRun();
          r.bold = !r.bold;
          bBold.className = `flex-1 py-1 rounded border border-slate-700 font-bold text-xs cursor-pointer ${r.bold ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}`;
          this.notifyChange();
        });
      }

      if (bItalic) {
        bItalic.addEventListener('click', () => {
          const r = ensureRun();
          r.italic = !r.italic;
          bItalic.className = `flex-1 py-1 rounded border border-slate-700 italic font-serif text-xs cursor-pointer ${r.italic ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}`;
          this.notifyChange();
        });
      }

      if (bUnderline) {
        bUnderline.addEventListener('click', () => {
          const r = ensureRun();
          r.underline = !r.underline;
          bUnderline.className = `flex-1 py-1 rounded border border-slate-700 underline text-xs cursor-pointer ${r.underline ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}`;
          this.notifyChange();
        });
      }

      if (bAlignL && bAlignC && bAlignR) {
        const setAlign = (align: 'left' | 'center' | 'right') => {
          if (!tb.paragraph_style) tb.paragraph_style = {};
          tb.paragraph_style.align = align;
          bAlignL.className = `flex-1 py-1 rounded border border-slate-700 text-xs cursor-pointer ${align === 'left' ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}`;
          bAlignC.className = `flex-1 py-1 rounded border border-slate-700 text-xs cursor-pointer ${align === 'center' ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}`;
          bAlignR.className = `flex-1 py-1 rounded border border-slate-700 text-xs cursor-pointer ${align === 'right' ? 'bg-sky-700 text-white' : 'bg-slate-800 text-slate-400'}`;
          this.notifyChange();
        };

        bAlignL.addEventListener('click', () => setAlign('left'));
        bAlignC.addEventListener('click', () => setAlign('center'));
        bAlignR.addEventListener('click', () => setAlign('right'));
      }

      if (lh) {
        lh.addEventListener('change', () => {
          if (!tb.paragraph_style) tb.paragraph_style = {};
          tb.paragraph_style.line_height = parseFloat(lh.value);
          this.notifyChange();
        });
      }

      if (af) {
        af.addEventListener('change', () => {
          tb.autofit = af.checked;
          this.notifyChange();
        });
      }

      if (ow && oc) {
        const updateOutline = () => {
          const w = parseFloat(ow.value);
          if (!tb.effects) tb.effects = {};
          tb.effects.outline = w > 0 ? { width: w, color: oc.value } : undefined;
          this.notifyChange();
        };
        ow.addEventListener('change', updateOutline);
        oc.addEventListener('input', updateOutline);
      }

      if (sb && sc) {
        const updateShadow = () => {
          const b = parseFloat(sb.value);
          if (!tb.effects) tb.effects = {};
          tb.effects.shadow = b > 0 ? { dx: 2, dy: 2, blur: b, color: sc.value } : undefined;
          this.notifyChange();
        };
        sb.addEventListener('change', updateShadow);
        sc.addEventListener('input', updateShadow);
      }
    } else if (el.type === 'Shape') {
      const shapeSection = document.createElement('div');
      shapeSection.className = 'border-t border-slate-700/60 pt-3 space-y-2';
      shapeSection.innerHTML = `
        <div class="font-semibold text-slate-400 mb-1">Shape Attributes</div>
        <div>
          <label class="text-[10px] text-slate-500">Fill Color</label>
          <input type="color" id="prop-shape-fill" class="w-full h-7 bg-transparent rounded cursor-pointer" value="${escapeHtml(el.fill_color)}">
        </div>
      `;
      content.appendChild(shapeSection);
      const sf = shapeSection.querySelector('#prop-shape-fill') as HTMLInputElement | null;
      if (sf) {
        sf.addEventListener('input', () => {
          el.fill_color = sf.value;
          this.notifyChange();
        });
      }
    } else if (el.type === 'Image') {
      const crop = el.crop ?? { x: 0, y: 0, w: 1, h: 1 };
      const imgSection = document.createElement('div');
      imgSection.className = 'border-t border-slate-700/60 pt-3 space-y-2';
      imgSection.innerHTML = `
        <div class="font-semibold text-slate-400 mb-1">Image Crop</div>
        <div class="text-[10px] text-slate-500 mb-1">Right-click the image on canvas for the interactive crop tool, or fine-tune numerically here (% of the source image).</div>
        <div class="grid grid-cols-2 gap-2">
          <div>
            <label class="text-[10px] text-slate-500">Crop X (%)</label>
            <input type="number" id="prop-crop-x" step="1" min="0" max="99" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(crop.x * 100)}">
          </div>
          <div>
            <label class="text-[10px] text-slate-500">Crop Y (%)</label>
            <input type="number" id="prop-crop-y" step="1" min="0" max="99" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(crop.y * 100)}">
          </div>
          <div>
            <label class="text-[10px] text-slate-500">Crop Width (%)</label>
            <input type="number" id="prop-crop-w" step="1" min="1" max="100" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(crop.w * 100)}">
          </div>
          <div>
            <label class="text-[10px] text-slate-500">Crop Height (%)</label>
            <input type="number" id="prop-crop-h" step="1" min="1" max="100" class="w-full bg-slate-800 border border-slate-700 rounded px-1.5 py-0.5" value="${Math.round(crop.h * 100)}">
          </div>
        </div>
        <button id="prop-crop-reset" class="w-full mt-1 text-[10px] bg-slate-800 border border-slate-700 rounded py-1 hover:bg-slate-700">Reset Crop</button>
      `;
      content.appendChild(imgSection);

      const cx = imgSection.querySelector('#prop-crop-x') as HTMLInputElement;
      const cy = imgSection.querySelector('#prop-crop-y') as HTMLInputElement;
      const cw = imgSection.querySelector('#prop-crop-w') as HTMLInputElement;
      const ch = imgSection.querySelector('#prop-crop-h') as HTMLInputElement;
      const reset = imgSection.querySelector('#prop-crop-reset') as HTMLButtonElement;

      const onCropChange = () => {
        const w = Math.max(0.01, Math.min(1, parseFloat(cw.value) / 100));
        const h = Math.max(0.01, Math.min(1, parseFloat(ch.value) / 100));
        const x = Math.max(0, Math.min(1 - w, parseFloat(cx.value) / 100));
        const y = Math.max(0, Math.min(1 - h, parseFloat(cy.value) / 100));
        el.crop = { x, y, w, h };
        this.notifyChange();
      };
      if (cx && cy && cw && ch) {
        cx.addEventListener('change', onCropChange);
        cy.addEventListener('change', onCropChange);
        cw.addEventListener('change', onCropChange);
        ch.addEventListener('change', onCropChange);
      }
      if (reset) {
        reset.addEventListener('click', () => {
          el.crop = { x: 0, y: 0, w: 1, h: 1 };
          this.notifyChange();
        });
      }
    }

    this.panelEl.appendChild(content);
  }

  private renderMultiSelectProperties(): void {
    const header = document.createElement('div');
    header.className = 'editor-props-header text-xs font-bold uppercase tracking-wider text-slate-400 p-3 border-b border-slate-700/60';
    header.textContent = `Multiple Selection (${this.selectedElements.length})`;
    this.panelEl.appendChild(header);

    const content = document.createElement('div');
    content.className = 'p-3 space-y-3 text-xs text-slate-400';
    content.innerHTML = `
      <p>${this.selectedElements.length} elements selected.</p>
      <p class="text-[11px] text-slate-500">Use toolbar alignment buttons or right-click to group / arrange.</p>
    `;
    this.panelEl.appendChild(content);
  }

  private notifyChange(): void {
    if (this.currentSlide && this.onChangeCb) {
      this.onChangeCb(this.currentSlide);
    }
  }
}
