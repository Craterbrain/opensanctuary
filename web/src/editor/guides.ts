import { SlideElement } from './types';

export interface GuideLine {
  orientation: 'v' | 'h';
  pos: number; // 0.0 .. 1.0 (normalized)
}

export interface SnapResult {
  x: number;
  y: number;
  activeGuides: GuideLine[];
}

export function snapElement(
  x: number,
  y: number,
  w: number,
  h: number,
  otherElements: SlideElement[],
  canvasWidth: number,
  canvasHeight: number,
  snapThresholdPx: number = 6
): SnapResult {
  const thresholdX = snapThresholdPx / Math.max(canvasWidth, 1);
  const thresholdY = snapThresholdPx / Math.max(canvasHeight, 1);

  // Targets to align with (normalized)
  const vTargets: number[] = [0.0, 0.5, 1.0]; // Canvas left, center, right
  const hTargets: number[] = [0.0, 0.5, 1.0]; // Canvas top, center, bottom

  for (const el of otherElements) {
    const t = el.transform;
    vTargets.push(t.x, t.x + t.w / 2, t.x + t.w);
    hTargets.push(t.y, t.y + t.h / 2, t.y + t.h);
  }

  let snappedX = x;
  let snappedY = y;
  const activeGuides: GuideLine[] = [];

  // X Snap Candidates: left edge, center, right edge
  const xPoints = [
    { pt: x, offset: 0 },
    { pt: x + w / 2, offset: w / 2 },
    { pt: x + w, offset: w }
  ];

  let minDeltaX = thresholdX + 1e-6;
  let bestTargetX: number | null = null;
  let bestOffsetX = 0;

  for (const xp of xPoints) {
    for (const target of vTargets) {
      const delta = Math.abs(xp.pt - target);
      if (delta <= thresholdX && delta < minDeltaX) {
        minDeltaX = delta;
        bestTargetX = target;
        bestOffsetX = xp.offset;
      }
    }
  }

  if (bestTargetX !== null) {
    snappedX = bestTargetX - bestOffsetX;
    activeGuides.push({ orientation: 'v', pos: bestTargetX });
  }

  // Y Snap Candidates: top edge, center, bottom edge
  const yPoints = [
    { pt: y, offset: 0 },
    { pt: y + h / 2, offset: h / 2 },
    { pt: y + h, offset: h }
  ];

  let minDeltaY = thresholdY + 1e-6;
  let bestTargetY: number | null = null;
  let bestOffsetY = 0;

  for (const yp of yPoints) {
    for (const target of hTargets) {
      const delta = Math.abs(yp.pt - target);
      if (delta <= thresholdY && delta < minDeltaY) {
        minDeltaY = delta;
        bestTargetY = target;
        bestOffsetY = yp.offset;
      }
    }
  }

  if (bestTargetY !== null) {
    snappedY = bestTargetY - bestOffsetY;
    activeGuides.push({ orientation: 'h', pos: bestTargetY });
  }

  return {
    x: Math.max(0, Math.min(1 - w, snappedX)),
    y: Math.max(0, Math.min(1 - h, snappedY)),
    activeGuides
  };
}

export function renderGuidesOverlay(
  containerEl: HTMLElement,
  guides: GuideLine[],
  canvasWidth: number,
  canvasHeight: number
): void {
  // Remove existing guide lines
  const existing = containerEl.querySelectorAll('.canvas-magnetic-guide');
  existing.forEach(e => e.remove());

  for (const g of guides) {
    const line = document.createElement('div');
    line.className = 'canvas-magnetic-guide';
    line.style.position = 'absolute';
    line.style.pointerEvents = 'none';
    line.style.zIndex = '9999';

    if (g.orientation === 'v') {
      line.style.left = `${g.pos * canvasWidth}px`;
      line.style.top = '0px';
      line.style.width = '1px';
      line.style.height = `${canvasHeight}px`;
      line.style.borderLeft = '1px dashed #3b82f6';
    } else {
      line.style.left = '0px';
      line.style.top = `${g.pos * canvasHeight}px`;
      line.style.width = `${canvasWidth}px`;
      line.style.height = '1px';
      line.style.borderTop = '1px dashed #3b82f6';
    }
    containerEl.appendChild(line);
  }
}
