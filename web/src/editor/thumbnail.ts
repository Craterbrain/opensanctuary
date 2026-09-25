import { EditorSlide, SlideBackground } from './types';
import { applyResolvedBackground, isVideoBackground } from '../core/presentation_helpers';

export interface ThumbnailCallbacks {
  onSelectSlide: (index: number) => void;
  onReorderSlides: (fromIndex: number, toIndex: number) => void;
  onDuplicateSlide: (index: number) => void;
  onDeleteSlide: (index: number) => void;
  onAddSlide: () => void;
  onUpdateSlideTag?: (index: number, tag: string | null) => void;
  onUpdateSlideDuration?: (index: number, durationSeconds: number | null) => void;
}

export class FilmstripSidebar {
  private containerEl: HTMLElement;
  private callbacks: ThumbnailCallbacks;
  private slides: EditorSlide[] = [];
  private activeIndex: number = 0;
  private draggedIndex: number | null = null;
  private activePopover: HTMLElement | null = null;
  private mode: string = 'song';

  constructor(containerEl: HTMLElement, callbacks: ThumbnailCallbacks, mode: string = 'song') {
    this.containerEl = containerEl;
    this.callbacks = callbacks;
    this.mode = mode || 'song';
  }

  public setMode(mode: string): void {
    this.mode = mode || 'song';
    this.closeActivePopover();
    this.containerEl.dataset.mode = this.mode;
    this.containerEl.classList.toggle('mode-song', this.isSongMode());
    this.render();
  }

  public getMode(): string {
    return this.mode;
  }

  public isSongMode(): boolean {
    return (this.mode || 'song').toLowerCase() === 'song';
  }

  public setSlides(slides: EditorSlide[], activeIndex: number = 0, mode?: string): void {
    if (mode !== undefined) {
      this.mode = mode;
    }
    this.slides = slides;
    this.activeIndex = activeIndex;
    this.render();
  }

  public setActiveIndex(index: number): void {
    this.activeIndex = index;
    const cards = this.containerEl.querySelectorAll('.editor-filmstrip-card');
    cards.forEach((card, idx) => {
      card.classList.toggle('active', idx === index);
    });
  }

  public render(): void {
    this.containerEl.innerHTML = '';
    this.containerEl.classList.add('editor-filmstrip-pane', 'studio-filmstrip-pane');
    this.containerEl.dataset.mode = this.mode;
    this.containerEl.classList.toggle('mode-song', this.isSongMode());
    this.containerEl.style.width = '200px';
    this.containerEl.style.minWidth = '200px';
    this.containerEl.style.maxWidth = '200px';
    this.containerEl.style.flex = '0 0 200px';
    this.containerEl.style.display = 'flex';
    this.containerEl.style.flexDirection = 'column';
    this.containerEl.style.background = '#18181b';
    this.containerEl.style.borderRight = '1px solid #27272a';
    this.containerEl.style.height = '100%';
    this.containerEl.style.userSelect = 'none';

    // Header
    const header = document.createElement('div');
    header.className = 'studio-filmstrip-header';
    header.style.display = 'flex';
    header.style.justifyContent = 'space-between';
    header.style.alignItems = 'center';
    header.style.padding = '8px 10px';
    header.style.borderBottom = '1px solid #27272a';

    const titleSpan = document.createElement('span');
    titleSpan.className = 'text-xs font-semibold text-slate-400';
    titleSpan.textContent = `SLIDES (${this.slides.length})`;

    const addBtn = document.createElement('button');
    addBtn.className = 'btn btn-tag text-xs font-semibold text-sky-400 hover:text-sky-300';
    addBtn.textContent = '➕ Slide';
    addBtn.title = 'Add New Slide';
    addBtn.addEventListener('click', () => this.callbacks.onAddSlide());

    header.appendChild(titleSpan);
    header.appendChild(addBtn);
    this.containerEl.appendChild(header);

    // List container
    const listEl = document.createElement('div');
    listEl.className = 'studio-filmstrip-list';
    listEl.style.flex = '1';
    listEl.style.overflowY = 'auto';
    listEl.style.padding = '10px';
    listEl.style.display = 'flex';
    listEl.style.flexDirection = 'column';
    listEl.style.gap = '10px';

    this.slides.forEach((slide, idx) => {
      const card = this.createSlideCard(slide, idx);
      listEl.appendChild(card);
    });

    this.containerEl.appendChild(listEl);
  }

  private closeActivePopover(): void {
    if (this.activePopover) {
      this.activePopover.remove();
      this.activePopover = null;
    }
  }

  private showTagMenu(anchorEl: HTMLElement, currentTag: string | undefined, onSelect: (newTag: string | null) => void): void {
    this.closeActivePopover();

    const menu = document.createElement('div');
    menu.className = 'fs-popover-menu';
    const rect = anchorEl.getBoundingClientRect();
    menu.style.position = 'fixed';
    menu.style.left = `${rect.left}px`;
    menu.style.top = `${rect.bottom + 4}px`;
    menu.style.zIndex = '9999';

    const tags = [
      { id: 'V1', label: 'Verse 1 (V1)', color: '#93c5fd' },
      { id: 'V2', label: 'Verse 2 (V2)', color: '#93c5fd' },
      { id: 'V3', label: 'Verse 3 (V3)', color: '#93c5fd' },
      { id: 'V4', label: 'Verse 4 (V4)', color: '#93c5fd' },
      { id: 'C', label: 'Chorus (C)', color: '#fcd34d' },
      { id: 'C2', label: 'Chorus 2 (C2)', color: '#fcd34d' },
      { id: 'B', label: 'Bridge (B)', color: '#d8b4fe' },
      { id: 'P', label: 'Pre-Chorus (P)', color: '#5eead4' },
      { id: 'I', label: 'Intro (I)', color: '#a5b4fc' },
      { id: 'E', label: 'Ending (E)', color: '#fda4af' },
      { id: 'CUSTOM', label: '✏️ Custom Tag...', color: '#a1a1aa' },
      { id: 'CLEAR', label: '✕ Clear Tag', color: '#ef4444' }
    ];

    tags.forEach(t => {
      const item = document.createElement('button');
      item.className = 'fs-popover-item';
      item.style.color = t.color;
      item.textContent = t.label;
      if (currentTag && currentTag.toUpperCase() === t.id) {
        item.classList.add('selected');
      }
      item.addEventListener('click', (e) => {
        e.stopPropagation();
        this.closeActivePopover();
        if (t.id === 'CLEAR') {
          onSelect(null);
        } else if (t.id === 'CUSTOM') {
          const val = prompt('Enter custom slide section tag (e.g. Chorus 3, Interlude):', currentTag || '');
          if (val !== null) onSelect(val.trim() || null);
        } else {
          onSelect(t.id);
        }
      });
      menu.appendChild(item);
    });

    const onDocClick = (e: MouseEvent) => {
      if (!menu.contains(e.target as Node)) {
        this.closeActivePopover();
        document.removeEventListener('click', onDocClick, true);
      }
    };
    setTimeout(() => {
      document.addEventListener('click', onDocClick, true);
    }, 10);

    document.body.appendChild(menu);
    this.activePopover = menu;
  }

  private showTimerMenu(anchorEl: HTMLElement, currentDuration: number | undefined | null, onSelect: (seconds: number | null) => void): void {
    this.closeActivePopover();

    const menu = document.createElement('div');
    menu.className = 'fs-popover-menu';
    const rect = anchorEl.getBoundingClientRect();
    menu.style.position = 'fixed';
    menu.style.left = `${rect.left}px`;
    menu.style.top = `${rect.bottom + 4}px`;
    menu.style.zIndex = '9999';

    const options = [
      { sec: null, label: '⏱ Manual (Off)' },
      { sec: 3, label: '⏱ 3 seconds' },
      { sec: 5, label: '⏱ 5 seconds' },
      { sec: 7, label: '⏱ 7 seconds' },
      { sec: 10, label: '⏱ 10 seconds' },
      { sec: 15, label: '⏱ 15 seconds' },
      { sec: 20, label: '⏱ 20 seconds' },
      { sec: 30, label: '⏱ 30 seconds' },
      { sec: -1, label: '⏱ Custom seconds...' }
    ];

    options.forEach(opt => {
      const item = document.createElement('button');
      item.className = 'fs-popover-item';
      item.textContent = opt.label;
      if ((opt.sec === null && !currentDuration) || (opt.sec !== null && opt.sec === currentDuration)) {
        item.classList.add('selected');
      }
      item.addEventListener('click', (e) => {
        e.stopPropagation();
        this.closeActivePopover();
        if (opt.sec === -1) {
          const val = prompt('Enter auto-advance duration in seconds (e.g. 8):', String(currentDuration || 5));
          if (val !== null) {
            const num = parseFloat(val);
            if (!isNaN(num) && num > 0) onSelect(num);
            else if (num === 0) onSelect(null);
          }
        } else {
          onSelect(opt.sec);
        }
      });
      menu.appendChild(item);
    });

    const onDocClick = (e: MouseEvent) => {
      if (!menu.contains(e.target as Node)) {
        this.closeActivePopover();
        document.removeEventListener('click', onDocClick, true);
      }
    };
    setTimeout(() => {
      document.addEventListener('click', onDocClick, true);
    }, 10);

    document.body.appendChild(menu);
    this.activePopover = menu;
  }

  private createSlideCard(slide: EditorSlide, idx: number): HTMLElement {
    const card = document.createElement('div');
    card.className = `editor-filmstrip-card studio-filmstrip-card ${idx === this.activeIndex ? 'active' : ''}`;
    card.dataset.index = `${idx}`;
    card.draggable = true;

    card.style.display = 'flex';
    card.style.flexDirection = 'column';
    card.style.borderRadius = '6px';
    card.style.border = idx === this.activeIndex ? '2px solid #3b82f6' : '1px solid #27272a';
    card.style.background = '#202024';
    card.style.overflow = 'hidden';
    card.style.cursor = 'pointer';
    card.style.transition = 'all 0.15s ease';

    // Slide number & action header
    const cardHeader = document.createElement('div');
    cardHeader.className = 'editor-filmstrip-card-header';
    cardHeader.style.display = 'flex';
    cardHeader.style.justifyContent = 'space-between';
    cardHeader.style.alignItems = 'center';
    cardHeader.style.padding = '3px 6px';
    cardHeader.style.fontSize = '10px';
    cardHeader.style.color = '#a1a1aa';
    cardHeader.style.background = 'rgba(0,0,0,0.2)';
    cardHeader.style.gap = '4px';

    const leftWrap = document.createElement('div');
    leftWrap.style.display = 'flex';
    leftWrap.style.alignItems = 'center';
    leftWrap.style.gap = '4px';
    leftWrap.style.minWidth = '0';
    leftWrap.style.flex = '1';

    const numSpan = document.createElement('span');
    numSpan.textContent = `${idx + 1}`;
    numSpan.style.fontWeight = '600';
    numSpan.style.flexShrink = '0';
    leftWrap.appendChild(numSpan);

    // 1. Section Tag Button (shown in song mode, hidden outside song mode)
    const tagBtn = document.createElement('button');
    tagBtn.className = 'fs-header-badge-btn fs-slide-tag-btn';
    const tagLabel = slide.tag ? slide.tag : 'Tag';
    tagBtn.textContent = `${tagLabel} ▾`;
    tagBtn.title = slide.label ? `${slide.label} (${slide.tag || ''})` : 'Set Section Tag';
    applyTagBtnStyle(tagBtn, slide.tag);
    if (!this.isSongMode()) {
      tagBtn.style.display = 'none';
    }
    tagBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      this.showTagMenu(tagBtn, slide.tag, (newTag) => {
        this.callbacks.onUpdateSlideTag?.(idx, newTag);
      });
    });
    leftWrap.appendChild(tagBtn);

    // 2. Auto-Advance Timer Button
    const timerBtn = document.createElement('button');
    timerBtn.className = 'fs-header-badge-btn fs-slide-timer-btn';
    const hasTimer = slide.duration_seconds && slide.duration_seconds > 0;
    timerBtn.textContent = hasTimer ? `⏱ ${slide.duration_seconds}s` : '⏱ --';
    timerBtn.title = hasTimer ? `Auto-advance after ${slide.duration_seconds}s` : 'Set Slide Auto-Advance Timer';
    applyTimerBtnStyle(timerBtn, slide.duration_seconds);
    timerBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      this.showTimerMenu(timerBtn, slide.duration_seconds, (seconds) => {
        this.callbacks.onUpdateSlideDuration?.(idx, seconds);
      });
    });
    leftWrap.appendChild(timerBtn);

    cardHeader.appendChild(leftWrap);

    const actionsWrap = document.createElement('div');
    actionsWrap.style.display = 'flex';
    actionsWrap.style.alignItems = 'center';
    actionsWrap.style.gap = '4px';
    actionsWrap.style.flexShrink = '0';

    const dupBtn = document.createElement('button');
    dupBtn.className = 'studio-btn-mini';
    dupBtn.title = 'Duplicate Slide';
    dupBtn.textContent = '⧉';
    dupBtn.style.fontSize = '10px';
    dupBtn.style.padding = '1px 3px';
    dupBtn.style.background = 'transparent';
    dupBtn.style.border = 'none';
    dupBtn.style.color = '#a1a1aa';
    dupBtn.style.cursor = 'pointer';
    dupBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      this.callbacks.onDuplicateSlide(idx);
    });

    const delBtn = document.createElement('button');
    delBtn.className = 'studio-btn-mini';
    delBtn.title = 'Delete Slide';
    delBtn.textContent = '🗑';
    delBtn.style.fontSize = '10px';
    delBtn.style.padding = '1px 3px';
    delBtn.style.background = 'transparent';
    delBtn.style.border = 'none';
    delBtn.style.color = '#ef4444';
    delBtn.style.cursor = 'pointer';
    delBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      this.callbacks.onDeleteSlide(idx);
    });

    actionsWrap.appendChild(dupBtn);
    actionsWrap.appendChild(delBtn);
    cardHeader.appendChild(actionsWrap);
    card.appendChild(cardHeader);

    // Mini 16:9 thumbnail preview
    const thumbView = document.createElement('div');
    thumbView.style.width = '100%';
    thumbView.style.aspectRatio = '16 / 9';
    thumbView.style.position = 'relative';
    thumbView.style.overflow = 'hidden';
    thumbView.style.display = 'flex';
    thumbView.style.alignItems = 'center';
    thumbView.style.justifyContent = 'center';
    thumbView.style.padding = '4px';
    thumbView.style.boxSizing = 'border-box';

    this.renderThumbnailBackground(thumbView, slide.background_v2, slide.background);

    // Mini content preview text
    const textPreview = document.createElement('div');
    textPreview.style.fontSize = '8px';
    textPreview.style.lineHeight = '1.2';
    textPreview.style.color = '#ffffff';
    textPreview.style.textAlign = 'center';
    textPreview.style.wordBreak = 'break-word';
    textPreview.style.display = '-webkit-box';
    textPreview.style.webkitLineClamp = '3';
    textPreview.style.webkitBoxOrient = 'vertical';
    textPreview.style.overflow = 'hidden';
    textPreview.style.textShadow = '1px 1px 2px rgba(0,0,0,0.8)';
    textPreview.style.pointerEvents = 'none';

    // Get text from elements or text field
    let previewContent = slide.text || '';
    if (!previewContent && slide.elements) {
      const textEls = slide.elements.filter(e => e.type === 'TextBlock');
      if (textEls.length > 0) {
        previewContent = (textEls[0] as any).block.runs.map((r: any) => r.text).join(' ');
      }
    }
    textPreview.textContent = previewContent || '(Empty Slide)';
    thumbView.appendChild(textPreview);

    card.appendChild(thumbView);

    // Click to select
    card.addEventListener('click', () => {
      this.callbacks.onSelectSlide(idx);
    });

    // Drag & Drop reorder
    card.addEventListener('dragstart', (e) => {
      this.draggedIndex = idx;
      if (e.dataTransfer) {
        e.dataTransfer.effectAllowed = 'move';
        e.dataTransfer.setData('text/plain', `${idx}`);
      }
      card.style.opacity = '0.5';
    });

    card.addEventListener('dragend', () => {
      card.style.opacity = '1.0';
      this.draggedIndex = null;
    });

    card.addEventListener('dragover', (e) => {
      e.preventDefault();
      if (e.dataTransfer) {
        e.dataTransfer.dropEffect = 'move';
      }
      card.style.borderColor = '#10b981';
    });

    card.addEventListener('dragleave', () => {
      card.style.borderColor = idx === this.activeIndex ? '#3b82f6' : '#27272a';
    });

    card.addEventListener('drop', (e) => {
      e.preventDefault();
      card.style.borderColor = idx === this.activeIndex ? '#3b82f6' : '#27272a';
      if (this.draggedIndex !== null && this.draggedIndex !== idx) {
        this.callbacks.onReorderSlides(this.draggedIndex, idx);
      }
    });

    return card;
  }

  private renderThumbnailBackground(el: HTMLElement, bgV2?: SlideBackground, legacyBg?: string): void {
    if (bgV2) {
      switch (bgV2.kind) {
        case 'Solid':
          // Also covers the "pattern:<name>" animated marker — the
          // filmstrip thumbnail gets the real (tiny) drifting animation too.
          applyResolvedBackground(el, bgV2.data);
          break;
        case 'Gradient': {
          el.style.animation = '';
          el.style.backgroundImage = '';
          const { kind, stops, angle_deg } = bgV2.data;
          const stopStr = stops.map(s => `${s.color} ${s.offset * 100}%`).join(', ');
          el.style.background = kind === 'radial'
            ? `radial-gradient(circle, ${stopStr})`
            : `linear-gradient(${angle_deg ?? 180}deg, ${stopStr})`;
          break;
        }
        case 'Image':
          el.style.animation = '';
          el.style.backgroundImage = '';
          el.style.background = `url("${bgV2.data.file_path}") center/cover no-repeat`;
          break;
        case 'Video':
          el.style.animation = '';
          el.style.backgroundImage = '';
          el.style.background = '#0d1117';
          break;
        default:
          el.style.animation = '';
          el.style.backgroundImage = '';
          el.style.background = '#0a0a0c';
          break;
      }
    } else if (legacyBg) {
      if (isVideoBackground(legacyBg)) {
        el.style.animation = '';
        el.style.backgroundImage = '';
        el.style.background = '#0d1117';
      } else {
        applyResolvedBackground(el, legacyBg);
      }
    } else {
      applyResolvedBackground(el, null);
    }
  }
}

export function applyTagBtnStyle(btn: HTMLElement, tag?: string): void {
  btn.classList.remove('fs-tag-verse', 'fs-tag-chorus', 'fs-tag-bridge', 'fs-tag-ending', 'fs-tag-prechorus', 'fs-tag-intro', 'fs-tag-neutral');
  if (!tag) {
    btn.classList.add('fs-tag-neutral');
    return;
  }
  const u = tag.toUpperCase();
  if (u.startsWith('V')) btn.classList.add('fs-tag-verse');
  else if (u.startsWith('C')) btn.classList.add('fs-tag-chorus');
  else if (u.startsWith('B')) btn.classList.add('fs-tag-bridge');
  else if (u === 'E' || u.includes('END') || u.includes('OUTRO')) btn.classList.add('fs-tag-ending');
  else if (u.startsWith('P')) btn.classList.add('fs-tag-prechorus');
  else if (u.startsWith('I')) btn.classList.add('fs-tag-intro');
  else btn.classList.add('fs-tag-neutral');
}

export function applyTimerBtnStyle(btn: HTMLElement, duration?: number | null): void {
  btn.classList.remove('fs-timer-active', 'fs-timer-inactive');
  if (duration && duration > 0) {
    btn.classList.add('fs-timer-active');
  } else {
    btn.classList.add('fs-timer-inactive');
  }
}
