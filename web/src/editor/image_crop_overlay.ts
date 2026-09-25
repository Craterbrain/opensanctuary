import { CropRect } from './types';
import {
  createResizeHandleElements,
  layoutResizeHandles,
  type ResizeHandlePosition,
} from './resize_handles';

// Interactive crop tool for an Image element: shows the FULL source image
// (letterboxed to fit the element's box via object-fit: contain) with a
// draggable/resizable window over it representing the current CropRect —
// the standard "crop to a sub-rect" UX. Modeled on element_frame.ts's
// 8-handle drag pattern, but scoped to one image's own box rather than the
// whole canvas.
export class ImageCropOverlay {
  public readonly hostEl: HTMLElement;
  private img: HTMLImageElement;
  private maskTop: HTMLElement;
  private maskBottom: HTMLElement;
  private maskLeft: HTMLElement;
  private maskRight: HTMLElement;
  private windowEl: HTMLElement;
  private handles: Map<ResizeHandlePosition, HTMLElement> = new Map();
  private crop: CropRect;
  private doneBtn: HTMLElement;
  private onChangeCb: ((crop: CropRect, isEnd: boolean) => void) | null = null;
  private onDoneCb: (() => void) | null = null;

  constructor(parentEl: HTMLElement, filePath: string, initialCrop: CropRect) {
    this.crop = { ...initialCrop };

    this.hostEl = document.createElement('div');
    this.hostEl.className = 'editor-image-crop-overlay';
    this.hostEl.style.position = 'absolute';
    this.hostEl.style.inset = '0';
    this.hostEl.style.zIndex = '9500';
    this.hostEl.style.background = 'rgba(0,0,0,0.55)';
    this.hostEl.style.userSelect = 'none';

    this.img = document.createElement('img');
    this.img.src = filePath.startsWith('/media/') ? filePath : `/media/${encodeURIComponent(filePath)}`;
    this.img.draggable = false;
    this.img.style.position = 'absolute';
    this.img.style.inset = '0';
    this.img.style.width = '100%';
    this.img.style.height = '100%';
    this.img.style.objectFit = 'contain';
    this.img.style.pointerEvents = 'none';
    this.hostEl.appendChild(this.img);

    const mkMask = () => {
      const m = document.createElement('div');
      m.style.position = 'absolute';
      m.style.background = 'rgba(0,0,0,0.55)';
      m.style.pointerEvents = 'none';
      this.hostEl.appendChild(m);
      return m;
    };
    this.maskTop = mkMask();
    this.maskBottom = mkMask();
    this.maskLeft = mkMask();
    this.maskRight = mkMask();

    this.windowEl = document.createElement('div');
    this.windowEl.style.position = 'absolute';
    this.windowEl.style.border = '1.5px solid #f59e0b';
    this.windowEl.style.boxSizing = 'border-box';
    this.windowEl.style.cursor = 'move';
    this.hostEl.appendChild(this.windowEl);
    this.bindWindowDrag();

    this.handles = createResizeHandleElements({ sizePx: 9, borderColor: '#f59e0b' });
    for (const [pos, h] of this.handles.entries()) {
      this.windowEl.appendChild(h);
      this.bindHandleDrag(h, pos);
    }

    this.doneBtn = document.createElement('button');
    this.doneBtn.textContent = '✓ Done Cropping';
    this.doneBtn.style.position = 'absolute';
    // Positioned inside the overlay's own bounds (not above/outside it) since
    // the image element this overlay is mounted inside has overflow: hidden
    // (see renderImageDOM) — anything placed outside its box is clipped and
    // unclickable, not just visually cut off.
    this.doneBtn.style.top = '4px';
    this.doneBtn.style.right = '4px';
    this.doneBtn.style.padding = '3px 10px';
    this.doneBtn.style.fontSize = '11px';
    this.doneBtn.style.fontWeight = '600';
    this.doneBtn.style.background = '#f59e0b';
    this.doneBtn.style.color = '#1e1e24';
    this.doneBtn.style.border = 'none';
    this.doneBtn.style.borderRadius = '4px';
    this.doneBtn.style.cursor = 'pointer';
    this.doneBtn.addEventListener('mousedown', (e) => e.stopPropagation());
    this.doneBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      this.onDoneCb?.();
    });
    this.hostEl.appendChild(this.doneBtn);

    parentEl.appendChild(this.hostEl);

    // The image needs a layout pass before its rendered (letterboxed) box can
    // be measured — reposition once it's loaded, and on any resize.
    if (this.img.complete) {
      requestAnimationFrame(() => this.updateVisual());
    } else {
      this.img.addEventListener('load', () => this.updateVisual(), { once: true });
    }
    this.resizeObserver = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(() => this.updateVisual()) : null;
    this.resizeObserver?.observe(this.hostEl);
  }

  private resizeObserver: ResizeObserver | null;

  public onChange(cb: (crop: CropRect, isEnd: boolean) => void): void {
    this.onChangeCb = cb;
  }

  public onDone(cb: () => void): void {
    this.onDoneCb = cb;
  }

  public destroy(): void {
    this.resizeObserver?.disconnect();
    this.hostEl.remove();
  }

  /** The image's own rendered (letterboxed) box, in px relative to hostEl. */
  private getImageBox(): { left: number; top: number; width: number; height: number } {
    const hostRect = this.hostEl.getBoundingClientRect();
    const imgRect = this.img.getBoundingClientRect();
    return {
      left: imgRect.left - hostRect.left,
      top: imgRect.top - hostRect.top,
      width: imgRect.width,
      height: imgRect.height
    };
  }

  private updateVisual(): void {
    const box = this.getImageBox();
    if (box.width <= 0 || box.height <= 0) return;

    const winLeft = box.left + this.crop.x * box.width;
    const winTop = box.top + this.crop.y * box.height;
    const winW = this.crop.w * box.width;
    const winH = this.crop.h * box.height;

    this.windowEl.style.left = `${winLeft}px`;
    this.windowEl.style.top = `${winTop}px`;
    this.windowEl.style.width = `${winW}px`;
    this.windowEl.style.height = `${winH}px`;

    // Darken everything outside the window, within the image's own box.
    this.maskTop.style.left = `${box.left}px`;
    this.maskTop.style.top = `${box.top}px`;
    this.maskTop.style.width = `${box.width}px`;
    this.maskTop.style.height = `${Math.max(0, winTop - box.top)}px`;

    this.maskBottom.style.left = `${box.left}px`;
    this.maskBottom.style.top = `${winTop + winH}px`;
    this.maskBottom.style.width = `${box.width}px`;
    this.maskBottom.style.height = `${Math.max(0, box.top + box.height - (winTop + winH))}px`;

    this.maskLeft.style.left = `${box.left}px`;
    this.maskLeft.style.top = `${winTop}px`;
    this.maskLeft.style.width = `${Math.max(0, winLeft - box.left)}px`;
    this.maskLeft.style.height = `${winH}px`;

    this.maskRight.style.left = `${winLeft + winW}px`;
    this.maskRight.style.top = `${winTop}px`;
    this.maskRight.style.width = `${Math.max(0, box.left + box.width - (winLeft + winW))}px`;
    this.maskRight.style.height = `${winH}px`;

    layoutResizeHandles(this.handles, winW, winH, 9);
  }

  private bindWindowDrag(): void {
    this.windowEl.addEventListener('mousedown', (e) => {
      if ((e.target as HTMLElement) !== this.windowEl) return; // let handles own their own drags
      e.stopPropagation();
      e.preventDefault();
      const box = this.getImageBox();
      const startX = e.clientX;
      const startY = e.clientY;
      const init = { ...this.crop };

      const onMouseMove = (moveEv: MouseEvent) => {
        const dx = (moveEv.clientX - startX) / box.width;
        const dy = (moveEv.clientY - startY) / box.height;
        this.crop.x = Math.max(0, Math.min(1 - init.w, init.x + dx));
        this.crop.y = Math.max(0, Math.min(1 - init.h, init.y + dy));
        this.updateVisual();
        this.onChangeCb?.(this.crop, false);
      };
      const onMouseUp = () => {
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        this.onChangeCb?.(this.crop, true);
      };
      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }

  private bindHandleDrag(handleEl: HTMLElement, pos: string): void {
    handleEl.addEventListener('mousedown', (e) => {
      e.stopPropagation();
      e.preventDefault();
      const box = this.getImageBox();
      const startX = e.clientX;
      const startY = e.clientY;
      const init = { ...this.crop };
      const minSize = 0.03;

      const onMouseMove = (moveEv: MouseEvent) => {
        const dx = (moveEv.clientX - startX) / box.width;
        const dy = (moveEv.clientY - startY) / box.height;
        let { x, y, w, h } = init;

        if (pos.includes('e')) w = Math.max(minSize, Math.min(1 - init.x, init.w + dx));
        if (pos.includes('s')) h = Math.max(minSize, Math.min(1 - init.y, init.h + dy));
        if (pos.includes('w')) {
          const maxDx = init.w - minSize;
          const appliedDx = Math.max(-init.x, Math.min(dx, maxDx));
          x = init.x + appliedDx;
          w = init.w - appliedDx;
        }
        if (pos.includes('n')) {
          const maxDy = init.h - minSize;
          const appliedDy = Math.max(-init.y, Math.min(dy, maxDy));
          y = init.y + appliedDy;
          h = init.h - appliedDy;
        }

        this.crop = { x, y, w, h };
        this.updateVisual();
        this.onChangeCb?.(this.crop, false);
      };
      const onMouseUp = () => {
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        this.onChangeCb?.(this.crop, true);
      };
      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }
}
