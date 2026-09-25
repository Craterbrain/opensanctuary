import { ElementTransform, SlideElement, normalizeTransform } from './types';
import { snapElement, renderGuidesOverlay } from './guides';
import {
  createResizeHandleElements,
  layoutResizeHandles,
  type ResizeHandlePosition,
} from './resize_handles';

export interface TransformChangeEvent {
  elementId: string;
  transform: ElementTransform;
  isEnd: boolean;
}

export class ElementFrame {
  private frameEl: HTMLElement;
  private handles: Map<ResizeHandlePosition, HTMLElement> = new Map();
  private rotationStem: HTMLElement;
  private rotationHandle: HTMLElement;
  private activeElement: SlideElement | null = null;
  private otherElements: SlideElement[] = [];
  private canvasWidth: number = 1920;
  private canvasHeight: number = 1080;
  private onTransformCb: ((ev: TransformChangeEvent) => void) | null = null;

  constructor(canvasContainer: HTMLElement) {
    this.frameEl = document.createElement('div');
    this.frameEl.className = 'editor-element-frame';
    this.frameEl.style.position = 'absolute';
    this.frameEl.style.boxSizing = 'border-box';
    this.frameEl.style.border = '1.5px solid #2563eb';
    this.frameEl.style.pointerEvents = 'none';
    this.frameEl.style.zIndex = '9000';
    this.frameEl.style.display = 'none';

    // 8 Resize handles
    this.handles = createResizeHandleElements({ sizePx: 8, borderColor: '#2563eb' });
    for (const [pos, h] of this.handles.entries()) {
      this.frameEl.appendChild(h);
      this.bindHandleDrag(h, pos);
    }

    // Rotation stem & handle
    this.rotationStem = document.createElement('div');
    this.rotationStem.style.position = 'absolute';
    this.rotationStem.style.width = '1px';
    this.rotationStem.style.height = '16px';
    this.rotationStem.style.background = '#2563eb';
    this.rotationStem.style.left = '50%';
    this.rotationStem.style.top = '-16px';
    this.rotationStem.style.transform = 'translateX(-50%)';
    this.frameEl.appendChild(this.rotationStem);

    this.rotationHandle = document.createElement('div');
    this.rotationHandle.className = 'editor-rotation-handle';
    this.rotationHandle.style.position = 'absolute';
    this.rotationHandle.style.width = '10px';
    this.rotationHandle.style.height = '10px';
    this.rotationHandle.style.borderRadius = '50%';
    this.rotationHandle.style.background = '#ffffff';
    this.rotationHandle.style.border = '1.5px solid #2563eb';
    this.rotationHandle.style.left = '50%';
    this.rotationHandle.style.top = '-22px';
    this.rotationHandle.style.transform = 'translateX(-50%)';
    this.rotationHandle.style.pointerEvents = 'auto';
    this.rotationHandle.style.cursor = 'grab';
    this.frameEl.appendChild(this.rotationHandle);
    this.bindRotationDrag(this.rotationHandle);

    canvasContainer.appendChild(this.frameEl);
  }

  public setDimensions(w: number, h: number): void {
    this.canvasWidth = w;
    this.canvasHeight = h;
    this.update();
  }

  public setElement(el: SlideElement | null, allElements: SlideElement[] = []): void {
    this.activeElement = el ? JSON.parse(JSON.stringify(el)) : null;
    this.otherElements = el ? allElements.filter(e => e.id !== el.id) : [];
    if (!this.activeElement) {
      this.frameEl.style.display = 'none';
      return;
    }
    this.frameEl.style.display = 'block';
    this.update();
  }

  public onTransform(cb: (ev: TransformChangeEvent) => void): void {
    this.onTransformCb = cb;
  }

  public update(): void {
    if (!this.activeElement) return;
    const t = this.activeElement.transform;
    const pxX = t.x * this.canvasWidth;
    const pxY = t.y * this.canvasHeight;
    const pxW = t.w * this.canvasWidth;
    const pxH = t.h * this.canvasHeight;

    this.frameEl.style.left = `${pxX}px`;
    this.frameEl.style.top = `${pxY}px`;
    this.frameEl.style.width = `${pxW}px`;
    this.frameEl.style.height = `${pxH}px`;
    this.frameEl.style.transform = `rotate(${t.rotation_deg}deg)`;

    // Position handles
    layoutResizeHandles(this.handles, pxW, pxH, 8);
  }

  private bindHandleDrag(handleEl: HTMLElement, pos: string): void {
    handleEl.addEventListener('mousedown', (e) => {
      e.stopPropagation();
      e.preventDefault();
      if (!this.activeElement) return;

      const startX = e.clientX;
      const startY = e.clientY;
      const initT = { ...this.activeElement.transform };
      const initAspect = initT.w / Math.max(initT.h, 0.001);

      const onMouseMove = (moveEv: MouseEvent) => {
        const dx = (moveEv.clientX - startX) / this.canvasWidth;
        const dy = (moveEv.clientY - startY) / this.canvasHeight;
        let newX = initT.x;
        let newY = initT.y;
        let newW = initT.w;
        let newH = initT.h;

        // Apply delta based on handle direction
        if (pos.includes('e')) newW = Math.max(0.02, initT.w + dx);
        if (pos.includes('s')) newH = Math.max(0.02, initT.h + dy);
        if (pos.includes('w')) {
          const maxDx = initT.w - 0.02;
          const appliedDx = Math.min(dx, maxDx);
          newX = initT.x + appliedDx;
          newW = initT.w - appliedDx;
        }
        if (pos.includes('n')) {
          const maxDy = initT.h - 0.02;
          const appliedDy = Math.min(dy, maxDy);
          newY = initT.y + appliedDy;
          newH = initT.h - appliedDy;
        }

        // Maintain aspect ratio if Shift is pressed
        if (moveEv.shiftKey && (pos === 'nw' || pos === 'ne' || pos === 'se' || pos === 'sw')) {
          newH = newW / initAspect;
        }

        const snapped = snapElement(newX, newY, newW, newH, this.otherElements, this.canvasWidth, this.canvasHeight);
        const nextTransform = normalizeTransform({
          ...initT,
          x: snapped.x,
          y: snapped.y,
          w: newW,
          h: newH
        });

        if (this.frameEl.parentElement) {
          renderGuidesOverlay(this.frameEl.parentElement, snapped.activeGuides, this.canvasWidth, this.canvasHeight);
        }

        this.activeElement!.transform = nextTransform;
        this.update();
        if (this.onTransformCb) {
          this.onTransformCb({ elementId: this.activeElement!.id, transform: nextTransform, isEnd: false });
        }
      };

      const onMouseUp = () => {
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        if (this.frameEl.parentElement) {
          renderGuidesOverlay(this.frameEl.parentElement, [], this.canvasWidth, this.canvasHeight);
        }
        if (this.onTransformCb && this.activeElement) {
          this.onTransformCb({ elementId: this.activeElement.id, transform: this.activeElement.transform, isEnd: true });
        }
      };

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }

  private bindRotationDrag(handleEl: HTMLElement): void {
    handleEl.addEventListener('mousedown', (e) => {
      e.stopPropagation();
      e.preventDefault();
      if (!this.activeElement) return;

      const t = this.activeElement.transform;
      const centerPxX = (t.x + t.w / 2) * this.canvasWidth;
      const centerPxY = (t.y + t.h / 2) * this.canvasHeight;
      const rect = this.frameEl.parentElement?.getBoundingClientRect() || { left: 0, top: 0 };
      const absCenterX = rect.left + centerPxX;
      const absCenterY = rect.top + centerPxY;

      const onMouseMove = (moveEv: MouseEvent) => {
        const dx = moveEv.clientX - absCenterX;
        const dy = moveEv.clientY - absCenterY;
        let angleDeg = (Math.atan2(dy, dx) * 180) / Math.PI + 90;
        if (angleDeg < 0) angleDeg += 360;

        // Snap to 0, 45, 90, 135, 180, 225, 270, 315 when Shift is held
        if (moveEv.shiftKey) {
          angleDeg = Math.round(angleDeg / 15) * 15;
        }

        const nextT = { ...this.activeElement!.transform, rotation_deg: angleDeg };
        this.activeElement!.transform = nextT;
        this.update();
        if (this.onTransformCb) {
          this.onTransformCb({ elementId: this.activeElement!.id, transform: nextT, isEnd: false });
        }
      };

      const onMouseUp = () => {
        window.removeEventListener('mousemove', onMouseMove);
        window.removeEventListener('mouseup', onMouseUp);
        if (this.onTransformCb && this.activeElement) {
          this.onTransformCb({ elementId: this.activeElement.id, transform: this.activeElement.transform, isEnd: true });
        }
      };

      window.addEventListener('mousemove', onMouseMove);
      window.addEventListener('mouseup', onMouseUp);
    });
  }
}
