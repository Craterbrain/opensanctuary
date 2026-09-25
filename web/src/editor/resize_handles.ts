/**
 * OpenSanctuary / OS-Next Resize Handles Utility
 * Shared 8-handle generation, cursor mapping, layout calculation, and delta math
 * used across both canvas element frames and image crop overlays.
 */

export const RESIZE_HANDLE_POSITIONS = ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'] as const;
export type ResizeHandlePosition = typeof RESIZE_HANDLE_POSITIONS[number];

export const RESIZE_HANDLE_CURSORS: Record<ResizeHandlePosition, string> = {
  nw: 'nwse-resize',
  n: 'ns-resize',
  ne: 'nesw-resize',
  e: 'ew-resize',
  se: 'nwse-resize',
  s: 'ns-resize',
  sw: 'nesw-resize',
  w: 'ew-resize',
};

export interface ResizeHandleOptions {
  sizePx?: number;
  borderColor?: string;
  background?: string;
  borderRadius?: string;
  borderWidth?: string;
  classNamePrefix?: string;
}

/**
 * Creates 8 positioned resize handles as DOM elements with appropriate cursor styles.
 */
export function createResizeHandleElements(
  options: ResizeHandleOptions = {}
): Map<ResizeHandlePosition, HTMLElement> {
  const size = options.sizePx ?? 8;
  const borderColor = options.borderColor ?? '#2563eb';
  const background = options.background ?? '#ffffff';
  const borderRadius = options.borderRadius ?? '1px';
  const borderWidth = options.borderWidth ?? '1.5px';
  const prefix = options.classNamePrefix ?? 'editor-handle';

  const map = new Map<ResizeHandlePosition, HTMLElement>();

  if (typeof document === 'undefined') return map;

  for (const pos of RESIZE_HANDLE_POSITIONS) {
    const h = document.createElement('div');
    h.className = `${prefix} ${prefix}-${pos}`;
    h.style.position = 'absolute';
    h.style.width = `${size}px`;
    h.style.height = `${size}px`;
    h.style.background = background;
    h.style.border = `${borderWidth} solid ${borderColor}`;
    h.style.borderRadius = borderRadius;
    h.style.boxSizing = 'border-box';
    h.style.pointerEvents = 'auto';
    h.style.cursor = RESIZE_HANDLE_CURSORS[pos];
    h.dataset.handle = pos;
    map.set(pos, h);
  }

  return map;
}

export interface HandlePositionCoordinates {
  left: number;
  top: number;
}

/**
 * Computes exact pixel left/top offsets for a handle given rectangle dimensions and handle size.
 */
export function getHandleCoordinates(
  pos: ResizeHandlePosition,
  width: number,
  height: number,
  handleSize: number = 8
): HandlePositionCoordinates {
  const half = handleSize / 2;
  switch (pos) {
    case 'nw': return { left: -half, top: -half };
    case 'n': return { left: width / 2 - half, top: -half };
    case 'ne': return { left: width - half, top: -half };
    case 'e': return { left: width - half, top: height / 2 - half };
    case 'se': return { left: width - half, top: height - half };
    case 's': return { left: width / 2 - half, top: height - half };
    case 'sw': return { left: -half, top: height - half };
    case 'w': return { left: -half, top: height / 2 - half };
  }
}

/**
 * Updates DOM styles of all 8 handles around a target pixel width and height.
 */
export function layoutResizeHandles(
  handles: Map<ResizeHandlePosition, HTMLElement>,
  width: number,
  height: number,
  handleSize: number = 8
): void {
  for (const [pos, el] of handles.entries()) {
    const coords = getHandleCoordinates(pos, width, height, handleSize);
    el.style.left = `${coords.left}px`;
    el.style.top = `${coords.top}px`;
  }
}

export interface RectBounds {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ResizeDeltaOptions {
  minWidth?: number;
  minHeight?: number;
  maxWidth?: number;
  maxHeight?: number;
  maintainAspect?: boolean;
}

/**
 * Computes updated rectangle bounds given drag deltas with bounds clamping and optional aspect locking.
 */
export function computeResizeDelta(
  pos: ResizeHandlePosition,
  dx: number,
  dy: number,
  init: RectBounds,
  options: ResizeDeltaOptions = {}
): RectBounds {
  const minW = options.minWidth ?? 0.02;
  const minH = options.minHeight ?? 0.02;
  const maxW = options.maxWidth ?? 1.0;
  const maxH = options.maxHeight ?? 1.0;

  let x = init.x;
  let y = init.y;
  let w = init.w;
  let h = init.h;

  if (pos.includes('e')) {
    w = Math.max(minW, Math.min(maxW - init.x, init.w + dx));
  }
  if (pos.includes('s')) {
    h = Math.max(minH, Math.min(maxH - init.y, init.h + dy));
  }
  if (pos.includes('w')) {
    const maxAllowedDx = init.w - minW;
    const clampedDx = Math.max(-init.x, Math.min(dx, maxAllowedDx));
    x = init.x + clampedDx;
    w = init.w - clampedDx;
  }
  if (pos.includes('n')) {
    const maxAllowedDy = init.h - minH;
    const clampedDy = Math.max(-init.y, Math.min(dy, maxAllowedDy));
    y = init.y + clampedDy;
    h = init.h - clampedDy;
  }

  if (options.maintainAspect && init.w > 0 && init.h > 0) {
    const aspect = init.w / init.h;
    if (pos === 'nw' || pos === 'ne' || pos === 'se' || pos === 'sw') {
      h = w / aspect;
    }
  }

  return { x, y, w, h };
}
