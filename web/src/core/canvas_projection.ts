// Single source of truth for mapping the normalized 0..1 authored slide canvas
// (see ElementTransform in the backend Slide model) onto an arbitrary target
// pixel box, so the editor canvas, the in-app Live preview, and the projector/
// confidence-monitor output all agree on where an element lands regardless of
// their own aspect ratio or resolution.

export interface CanvasProjection {
  offsetX: number;
  offsetY: number;
  contentW: number;
  contentH: number;
  scaleX: number;
  scaleY: number;
}

export interface ProjectionOptions {
  authoredAspect?: number; // authored design aspect ratio (w/h), default 16/9
  fit?: 'contain' | 'cover';
}

export interface NormalizedRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface PixelRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export function computeCanvasProjection(
  targetW: number,
  targetH: number,
  opts: ProjectionOptions = {}
): CanvasProjection {
  const authoredAspect = opts.authoredAspect ?? (16 / 9);
  const fit = opts.fit ?? 'contain';

  if (!targetW || !targetH || authoredAspect <= 0) {
    return { offsetX: 0, offsetY: 0, contentW: targetW || 0, contentH: targetH || 0, scaleX: 1, scaleY: 1 };
  }

  // Authored canvas at an arbitrary reference size sharing the target's scale.
  const authoredW = targetH * authoredAspect;
  const authoredH = targetH;
  const scaleByHeight = targetH / authoredH;
  const scaleByWidth = targetW / authoredW;
  const scale = fit === 'cover'
    ? Math.max(scaleByWidth, scaleByHeight)
    : Math.min(scaleByWidth, scaleByHeight);

  const contentW = authoredW * scale;
  const contentH = authoredH * scale;
  const offsetX = (targetW - contentW) / 2;
  const offsetY = (targetH - contentH) / 2;

  return { offsetX, offsetY, contentW, contentH, scaleX: scale, scaleY: scale };
}

export function projectRect(rect: NormalizedRect, projection: CanvasProjection): PixelRect {
  return {
    left: projection.offsetX + rect.x * projection.contentW,
    top: projection.offsetY + rect.y * projection.contentH,
    width: rect.w * projection.contentW,
    height: rect.h * projection.contentH
  };
}
