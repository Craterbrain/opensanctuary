import { EditorSlide, SlideElement, SlideBackground, normalizeTransform } from './types';
import { SelectionManager } from './selection';
import { ElementFrame } from './element_frame';
import { renderTextBlockDOM } from './text_block';
import { renderImageDOM } from './image_tools';
import { renderVideoDOM } from './video_tools';
import { renderShapeDOM } from './shape_library';
import { renderLineDOM } from './line_tool';
import { renderTableDOM } from './table';
import { snapElement, renderGuidesOverlay } from './guides';
import { applyResolvedBackground, isVideoBackground } from '../core/presentation_helpers';
import { autoFitLyrics } from '../core/autofit';
import { computeCanvasProjection, projectRect, CanvasProjection } from '../core/canvas_projection';

export class EditorCanvas {
  public container: HTMLElement;
  public stage: HTMLElement;
  public rulerTop: HTMLElement;
  public rulerLeft: HTMLElement;
  public selection: SelectionManager;
  public frame: ElementFrame;

  private currentSlide: EditorSlide | null = null;
  private zoomLevel: number = 1.0;
  // Authors at real 1080p (matches the live output's typical real resolution,
  // not an arbitrary scaled-down proxy) so absolute-unit properties (font
  // size in pt, outline/shadow width in px) preview at the same effective
  // size here as they'll actually render live — the stage is displayed at
  // whatever size fits the panel via `transform: scale(zoomLevel)` below,
  // this is the virtual/authored resolution, not the on-screen pixel size.
  private baseWidth: number = 1920;
  private baseHeight: number = 1080;
  // The editor always authors at a fixed 16:9 canvas, so this projection has no
  // letterbox offset today (contentW/H === baseWidth/baseHeight) — but every
  // element-rect calculation below goes through it so the same math the Live/
  // Stage renderer uses (computeCanvasProjection/projectRect) stays the single
  // source of truth for normalized-to-pixel mapping.
  private projection: CanvasProjection = computeCanvasProjection(1920, 1080, { authoredAspect: 1920 / 1080 });
  private onSlideChangeCb: ((slide: EditorSlide) => void) | null = null;
  private isMarqueeDragging: boolean = false;
  private marqueeBox: HTMLElement | null = null;
  private editingElementId: string | null = null;

  constructor(viewportEl: HTMLElement) {
    this.container = viewportEl;
    this.container.style.position = 'relative';
    this.container.style.overflow = 'hidden';
    this.selection = new SelectionManager();

    // Rulers
    this.rulerTop = document.createElement('div');
    this.rulerTop.className = 'editor-ruler-top';
    this.rulerTop.style.position = 'absolute';
    this.rulerTop.style.top = '0';
    this.rulerTop.style.left = '24px';
    this.rulerTop.style.right = '0';
    this.rulerTop.style.height = '20px';
    this.rulerTop.style.background = '#1e1e24';
    this.rulerTop.style.borderBottom = '1px solid #333';
    this.rulerTop.style.zIndex = '50';
    this.rulerTop.style.overflow = 'hidden';

    this.rulerLeft = document.createElement('div');
    this.rulerLeft.className = 'editor-ruler-left';
    this.rulerLeft.style.position = 'absolute';
    this.rulerLeft.style.top = '20px';
    this.rulerLeft.style.left = '0';
    this.rulerLeft.style.bottom = '0';
    this.rulerLeft.style.width = '24px';
    this.rulerLeft.style.background = '#1e1e24';
    this.rulerLeft.style.borderRight = '1px solid #333';
    this.rulerLeft.style.zIndex = '50';
    this.rulerLeft.style.overflow = 'hidden';

    // Canvas Stage (16:9)
    this.stage = document.createElement('div');
    this.stage.className = 'editor-slide-canvas-stage';
    this.stage.style.position = 'absolute';
    this.stage.style.left = '50%';
    this.stage.style.top = '50%';
    this.stage.style.transform = `translate(-50%, -50%) scale(${this.zoomLevel})`;
    this.stage.style.transformOrigin = 'center center';
    this.stage.style.width = `${this.baseWidth}px`;
    this.stage.style.height = `${this.baseHeight}px`;
    this.stage.style.aspectRatio = '16 / 9';
    this.stage.style.boxShadow = '0 10px 30px rgba(0,0,0,0.5)';
    this.stage.style.overflow = 'hidden';
    this.stage.style.background = '#000000';
    this.stage.style.userSelect = 'none';

    this.container.appendChild(this.rulerTop);
    this.container.appendChild(this.rulerLeft);
    this.container.appendChild(this.stage);

    this.frame = new ElementFrame(this.stage);
    this.frame.setDimensions(this.baseWidth, this.baseHeight);

    this.bindEvents();

    if (typeof ResizeObserver !== 'undefined') {
      try {
        const ro = new ResizeObserver(() => {
          if (this.container.clientWidth > 0 && this.container.clientHeight > 0) {
            this.zoomToFit();
          }
        });
        ro.observe(this.container);
      } catch (_) {}
    }
  }

  public setSlide(slide: EditorSlide): void {
    this.currentSlide = slide;
    this.render();
  }

  public getSlide(): EditorSlide | null {
    return this.currentSlide;
  }

  public setZoom(zoom: number): void {
    if (isNaN(zoom) || zoom <= 0) return;
    this.zoomLevel = Math.max(0.1, Math.min(3.0, zoom));
    this.stage.style.transform = `translate(-50%, -50%) scale(${this.zoomLevel})`;
    this.renderRulers();
  }

  public getZoom(): number {
    return this.zoomLevel;
  }

  public zoomToFit(): void {
    const pad = 40;
    const availW = this.container.clientWidth - pad;
    const availH = this.container.clientHeight - pad - 20;
    if (availW <= 0 || availH <= 0) return;
    const scale = Math.min(availW / this.baseWidth, availH / this.baseHeight);
    this.setZoom(scale);
  }

  public onSlideChange(cb: (slide: EditorSlide) => void): void {
    this.onSlideChangeCb = cb;
  }

  public getEditingElementId(): string | null {
    return this.editingElementId;
  }

  /** Updates selection highlighting and the transform frame in place, without
   * rebuilding element DOM nodes (see the note in bindElementInteraction's
   * mousedown handler for why that matters). */
  private updateSelectionVisuals(): void {
    if (!this.currentSlide) return;
    const selectedIds = this.selection.getSelectedIds();
    this.stage.querySelectorAll('.slide-canvas-element').forEach((node) => {
      const id = (node as HTMLElement).dataset.elementId;
      (node as HTMLElement).classList.toggle('selected', !!id && this.selection.isSelected(id));
    });
    if (selectedIds.length === 1) {
      const target = this.currentSlide.elements.find(e => e.id === selectedIds[0]);
      this.frame.setElement(target || null, this.currentSlide.elements);
    } else {
      this.frame.setElement(null);
    }
  }

  public render(): void {
    if (!this.currentSlide) return;

    // 1. Render Background
    this.renderBackground(this.currentSlide.background_v2, this.currentSlide.background);

    // 2. Clear old element nodes (except frame, guides, and background video)
    const oldNodes = Array.from(this.stage.children).filter(
      c => !c.classList.contains('editor-element-frame') &&
           !c.classList.contains('canvas-magnetic-guide') &&
           !c.classList.contains('slide-canvas-bg-video')
    );
    oldNodes.forEach(n => n.remove());

    // 3. Sort elements by z_index
    const sorted = [...this.currentSlide.elements].sort((a, b) => a.transform.z_index - b.transform.z_index);

    // 4. Render element DOM nodes
    for (const el of sorted) {
      const elNode = this.createElementNode(el);
      this.stage.appendChild(elNode);
    }

    // 4b. Perform autofit text measurement now that elements are mounted in the DOM
    for (const el of sorted) {
      if (el.type === 'TextBlock' && el.block.autofit) {
        const elNode = this.stage.querySelector(`[data-element-id="${el.id}"]`) as HTMLElement;
        if (elNode) {
          const textInner = elNode.querySelector('.slide-text-inner') as HTMLElement;
          if (textInner) {
            autoFitLyrics(elNode, textInner, { minFontSize: 14, maxFontSize: 100 });
          }
        }
      }
    }

    // 5. Update Frame
    const selectedIds = this.selection.getSelectedIds();
    if (selectedIds.length === 1) {
      const target = this.currentSlide.elements.find(e => e.id === selectedIds[0]);
      this.frame.setElement(target || null, this.currentSlide.elements);
    } else {
      this.frame.setElement(null);
    }

    this.renderRulers();
  }

  private renderBackground(bgV2?: SlideBackground, legacyBg?: string): void {
    const isVid = bgV2?.kind === 'Video' || (legacyBg && isVideoBackground(legacyBg));
    let bgVideo = this.stage.querySelector('.slide-canvas-bg-video') as HTMLVideoElement | null;

    if (isVid) {
      this.stage.style.animation = '';
      this.stage.style.backgroundImage = '';
      this.stage.style.background = '#000000';

      const vidPath = bgV2?.kind === 'Video' ? bgV2.data.file_path : legacyBg!;
      const isLoop = bgV2?.kind === 'Video' ? (bgV2.data.loop_playback !== false) : true;

      if (!bgVideo) {
        bgVideo = document.createElement('video');
        bgVideo.className = 'slide-canvas-bg-video';
        bgVideo.style.position = 'absolute';
        bgVideo.style.inset = '0';
        bgVideo.style.width = '100%';
        bgVideo.style.height = '100%';
        bgVideo.style.objectFit = 'contain';
        bgVideo.style.pointerEvents = 'none';
        bgVideo.style.zIndex = '0';
        bgVideo.muted = true;
        bgVideo.autoplay = true;
        bgVideo.playsInline = true;
        this.stage.insertBefore(bgVideo, this.stage.firstChild);
      }
      bgVideo.loop = isLoop;
      const currentSrc = bgVideo.src || '';
      if (currentSrc !== vidPath && !currentSrc.endsWith(vidPath)) {
        bgVideo.src = vidPath;
        if (typeof bgVideo.load === 'function') {
          try { bgVideo.load(); } catch {}
        }
        if (typeof bgVideo.play === 'function') {
          try {
            const p = bgVideo.play();
            if (p && typeof p.catch === 'function') {
              p.catch(() => {});
            }
          } catch {}
        }
      }
      return;
    }

    if (bgVideo) {
      bgVideo.remove();
    }

    if (bgV2) {
      switch (bgV2.kind) {
        case 'Solid':
          // Also covers the "pattern:<name>" animated marker (see
          // ANIMATED_PATTERN_PRESETS) — a plain string assignment would
          // silently fail to render it (it isn't valid CSS on its own).
          applyResolvedBackground(this.stage, bgV2.data);
          break;
        case 'Gradient': {
          this.stage.style.animation = '';
          this.stage.style.backgroundImage = '';
          const { kind, stops, angle_deg } = bgV2.data;
          const stopStr = stops.map(s => `${s.color} ${s.offset * 100}%`).join(', ');
          this.stage.style.background = kind === 'radial'
            ? `radial-gradient(circle, ${stopStr})`
            : `linear-gradient(${angle_deg ?? 180}deg, ${stopStr})`;
          break;
        }
        case 'Image':
          this.stage.style.animation = '';
          this.stage.style.backgroundImage = '';
          this.stage.style.background = `url("${bgV2.data.file_path}") center/cover no-repeat`;
          break;
        default:
          applyResolvedBackground(this.stage, null);
          break;
      }
    } else if (legacyBg) {
      applyResolvedBackground(this.stage, legacyBg);
    } else {
      applyResolvedBackground(this.stage, null);
    }
  }

  private createElementNode(el: SlideElement): HTMLElement {
    const node = document.createElement('div');
    node.className = `slide-canvas-element slide-canvas-element-${el.type.toLowerCase()}`;
    node.dataset.elementId = el.id;
    node.style.position = 'absolute';
    node.style.boxSizing = 'border-box';
    const rect = projectRect(el.transform, this.projection);
    node.style.left = `${rect.left}px`;
    node.style.top = `${rect.top}px`;
    node.style.width = `${rect.width}px`;
    node.style.height = `${rect.height}px`;
    node.style.transform = `rotate(${el.transform.rotation_deg}deg)`;
    node.style.opacity = `${el.transform.opacity}`;
    node.style.zIndex = `${el.transform.z_index}`;
    node.style.cursor = 'move';

    const isSelected = this.selection.isSelected(el.id);
    if (isSelected) {
      node.classList.add('selected');
    }

    const isEditingThis = this.editingElementId === el.id;

    // Render content based on element type
    switch (el.type) {
      case 'TextBlock':
        renderTextBlockDOM(node, el.block, isEditingThis, (updatedBlock) => {
          el.block = updatedBlock;
          this.notifyChange();
        });
        break;
      case 'Image':
        renderImageDOM(node, {
          file_path: el.file_path,
          crop: el.crop,
          mask_shape: el.mask_shape,
          alt_text: el.alt_text,
          opacity: el.transform.opacity
        });
        break;
      case 'Video':
        renderVideoDOM(node, {
          file_path: el.file_path,
          in_point_s: el.in_point_s,
          out_point_s: el.out_point_s,
          loop_playback: el.loop_playback,
          is_muted: el.is_muted,
          volume: el.volume,
          opacity: el.transform.opacity
        });
        break;
      case 'Shape':
        renderShapeDOM(node, {
          shape_kind: el.shape_kind,
          fill_color: el.fill_color,
          stroke_color: el.stroke_color,
          stroke_width: el.stroke_width,
          opacity: el.transform.opacity
        });
        break;
      case 'Line':
        renderLineDOM(node, {
          line_kind: el.line_kind,
          color: el.color,
          stroke_width: el.stroke_width,
          start_arrow: el.start_arrow,
          end_arrow: el.end_arrow,
          opacity: el.transform.opacity
        });
        break;
      case 'Table':
        renderTableDOM(node, {
          rows: el.rows,
          cols: el.cols,
          cells: el.cells,
          opacity: el.transform.opacity
        }, isEditingThis, (updatedCells) => {
          el.cells = updatedCells;
          this.notifyChange();
        });
        break;
      case 'Group':
        for (const child of el.children) {
          node.appendChild(this.createElementNode(child));
        }
        break;
    }

    // Bind element-level drag and double-click
    this.bindElementInteraction(node, el);

    return node;
  }

  private bindElementInteraction(node: HTMLElement, el: SlideElement): void {
    node.addEventListener('mousedown', (e) => {
      if (this.editingElementId === el.id) return; // Allow contenteditable text selection
      e.stopPropagation();

      const isMulti = e.shiftKey || e.ctrlKey || e.metaKey;
      this.selection.select(el.id, isMulti);
      // Update selection highlighting/frame only — a full render() would
      // tear down and recreate this very node, which breaks the browser's
      // native double-click detection (Chromium requires both clicks of a
      // dblclick to target the SAME node instance): the first click here
      // would replace the node the second click needs to land on, so
      // dblclick (used to enter text-edit mode) would never fire.
      this.updateSelectionVisuals();

      // Drag to move
      const startClientX = e.clientX;
      const startClientY = e.clientY;
      const initX = el.transform.x;
      const initY = el.transform.y;
      const otherElements = this.currentSlide!.elements.filter(other => other.id !== el.id);

      const onMouseMove = (moveEv: MouseEvent) => {
        const dx = (moveEv.clientX - startClientX) / (this.projection.contentW * this.zoomLevel);
        const dy = (moveEv.clientY - startClientY) / (this.projection.contentH * this.zoomLevel);

        const targetX = initX + dx;
        const targetY = initY + dy;

        const snapped = snapElement(targetX, targetY, el.transform.w, el.transform.h, otherElements, this.projection.contentW, this.projection.contentH);
        el.transform.x = snapped.x;
        el.transform.y = snapped.y;

        const rect = projectRect(el.transform, this.projection);
        node.style.left = `${rect.left}px`;
        node.style.top = `${rect.top}px`;

        renderGuidesOverlay(this.stage, snapped.activeGuides, this.projection.contentW, this.projection.contentH);
        this.frame.setElement(el, this.currentSlide!.elements);
      };

      const onMouseUp = () => {
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        renderGuidesOverlay(this.stage, [], this.projection.contentW, this.projection.contentH);
        el.transform = normalizeTransform(el.transform);
        this.notifyChange();
      };

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });

    // Double click to edit TextBlock or Table
    node.addEventListener('dblclick', (e) => {
      e.stopPropagation();
      if (el.type === 'TextBlock' || el.type === 'Table') {
        this.editingElementId = el.id;
        this.render();
      }
    });
  }

  private bindEvents(): void {
    // Stage background click: deselect or start marquee
    this.stage.addEventListener('mousedown', (e) => {
      if ((e.target as HTMLElement).closest('.slide-canvas-element') ||
          (e.target as HTMLElement).closest('.editor-element-frame')) {
        return;
      }

      this.editingElementId = null;
      if (!e.shiftKey) {
        this.selection.deselectAll();
        this.render();
      }

      // Marquee selection
      this.isMarqueeDragging = true;
      const rect = this.stage.getBoundingClientRect();
      const startX = (e.clientX - rect.left) / (this.projection.contentW * this.zoomLevel);
      const startY = (e.clientY - rect.top) / (this.projection.contentH * this.zoomLevel);

      this.marqueeBox = document.createElement('div');
      this.marqueeBox.className = 'editor-marquee-box';
      this.marqueeBox.style.position = 'absolute';
      this.marqueeBox.style.border = '1px dashed #3b82f6';
      this.marqueeBox.style.background = 'rgba(59, 130, 246, 0.15)';
      this.marqueeBox.style.pointerEvents = 'none';
      this.marqueeBox.style.zIndex = '9990';
      this.stage.appendChild(this.marqueeBox);

      const onMouseMove = (moveEv: MouseEvent) => {
        if (!this.isMarqueeDragging || !this.marqueeBox) return;
        const curX = (moveEv.clientX - rect.left) / (this.projection.contentW * this.zoomLevel);
        const curY = (moveEv.clientY - rect.top) / (this.projection.contentH * this.zoomLevel);

        const x = Math.min(startX, curX);
        const y = Math.min(startY, curY);
        const w = Math.abs(curX - startX);
        const h = Math.abs(curY - startY);

        const box = projectRect({ x, y, w, h }, this.projection);
        this.marqueeBox.style.left = `${box.left}px`;
        this.marqueeBox.style.top = `${box.top}px`;
        this.marqueeBox.style.width = `${box.width}px`;
        this.marqueeBox.style.height = `${box.height}px`;

        if (this.currentSlide) {
          this.selection.selectByMarquee(x, y, w, h, this.currentSlide.elements);
        }
      };

      const onMouseUp = () => {
        this.isMarqueeDragging = false;
        if (this.marqueeBox) {
          this.marqueeBox.remove();
          this.marqueeBox = null;
        }
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        this.render();
      };

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });

    // Wire frame transform change
    this.frame.onTransform((ev) => {
      if (!this.currentSlide) return;
      const el = this.currentSlide.elements.find(e => e.id === ev.elementId);
      if (el) {
        el.transform = ev.transform;
        const domNode = this.stage.querySelector(`[data-element-id="${el.id}"]`) as HTMLElement;
        if (domNode) {
          const rect = projectRect(el.transform, this.projection);
          domNode.style.left = `${rect.left}px`;
          domNode.style.top = `${rect.top}px`;
          domNode.style.width = `${rect.width}px`;
          domNode.style.height = `${rect.height}px`;
          domNode.style.transform = `rotate(${el.transform.rotation_deg}deg)`;
        }
        if (ev.isEnd) {
          this.notifyChange();
        }
      }
    });

    window.addEventListener('resize', () => {
      this.renderRulers();
    });
  }

  private renderRulers(): void {
    // Render top horizontal ruler markings
    this.rulerTop.innerHTML = '';
    const step = 50 * this.zoomLevel;
    const countTop = Math.floor(this.container.clientWidth / Math.max(step, 10));
    for (let i = 0; i < countTop; i++) {
      const mark = document.createElement('div');
      mark.style.position = 'absolute';
      mark.style.left = `${i * step}px`;
      mark.style.top = '10px';
      mark.style.width = '1px';
      mark.style.height = '10px';
      mark.style.background = '#666';
      this.rulerTop.appendChild(mark);

      if (i % 2 === 0) {
        const lbl = document.createElement('span');
        lbl.style.position = 'absolute';
        lbl.style.left = `${i * step + 2}px`;
        lbl.style.top = '1px';
        lbl.style.fontSize = '9px';
        lbl.style.color = '#888';
        lbl.textContent = `${i * 50}`;
        this.rulerTop.appendChild(lbl);
      }
    }

    // Render left vertical ruler markings
    this.rulerLeft.innerHTML = '';
    const countLeft = Math.floor(this.container.clientHeight / Math.max(step, 10));
    for (let i = 0; i < countLeft; i++) {
      const mark = document.createElement('div');
      mark.style.position = 'absolute';
      mark.style.top = `${i * step}px`;
      mark.style.right = '0';
      mark.style.height = '1px';
      mark.style.width = '10px';
      mark.style.background = '#666';
      this.rulerLeft.appendChild(mark);
    }
  }

  private notifyChange(): void {
    if (this.currentSlide && this.onSlideChangeCb) {
      this.onSlideChangeCb(this.currentSlide);
    }
  }
}
