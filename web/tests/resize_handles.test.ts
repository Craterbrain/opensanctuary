import { describe, test, expect } from 'bun:test';
import {
  RESIZE_HANDLE_POSITIONS,
  RESIZE_HANDLE_CURSORS,
  createResizeHandleElements,
  getHandleCoordinates,
  layoutResizeHandles,
  computeResizeDelta,
} from '../src/editor/resize_handles';

describe('Shared Resize Handles: resize_handles.ts', () => {
  test('defines exactly 8 distinct cardinal and diagonal handle positions', () => {
    expect(RESIZE_HANDLE_POSITIONS.length).toBe(8);
    expect(RESIZE_HANDLE_POSITIONS).toContain('nw');
    expect(RESIZE_HANDLE_POSITIONS).toContain('n');
    expect(RESIZE_HANDLE_POSITIONS).toContain('ne');
    expect(RESIZE_HANDLE_POSITIONS).toContain('e');
    expect(RESIZE_HANDLE_POSITIONS).toContain('se');
    expect(RESIZE_HANDLE_POSITIONS).toContain('s');
    expect(RESIZE_HANDLE_POSITIONS).toContain('sw');
    expect(RESIZE_HANDLE_POSITIONS).toContain('w');
  });

  test('maps positions to standard CSS resize cursor styles', () => {
    expect(RESIZE_HANDLE_CURSORS.nw).toBe('nwse-resize');
    expect(RESIZE_HANDLE_CURSORS.n).toBe('ns-resize');
    expect(RESIZE_HANDLE_CURSORS.ne).toBe('nesw-resize');
    expect(RESIZE_HANDLE_CURSORS.e).toBe('ew-resize');
    expect(RESIZE_HANDLE_CURSORS.se).toBe('nwse-resize');
    expect(RESIZE_HANDLE_CURSORS.s).toBe('ns-resize');
    expect(RESIZE_HANDLE_CURSORS.sw).toBe('nesw-resize');
    expect(RESIZE_HANDLE_CURSORS.w).toBe('ew-resize');
  });

  test('calculates accurate pixel offsets for all 8 handles', () => {
    const w = 200;
    const h = 100;
    const size = 8;
    const half = 4;

    const nw = getHandleCoordinates('nw', w, h, size);
    expect(nw).toEqual({ left: -4, top: -4 });

    const n = getHandleCoordinates('n', w, h, size);
    expect(n).toEqual({ left: 100 - half, top: -4 });

    const ne = getHandleCoordinates('ne', w, h, size);
    expect(ne).toEqual({ left: 200 - half, top: -4 });

    const e = getHandleCoordinates('e', w, h, size);
    expect(e).toEqual({ left: 200 - half, top: 50 - half });

    const se = getHandleCoordinates('se', w, h, size);
    expect(se).toEqual({ left: 200 - half, top: 100 - half });

    const s = getHandleCoordinates('s', w, h, size);
    expect(s).toEqual({ left: 100 - half, top: 100 - half });

    const sw = getHandleCoordinates('sw', w, h, size);
    expect(sw).toEqual({ left: -4, top: 100 - half });

    const wCoords = getHandleCoordinates('w', w, h, size);
    expect(wCoords).toEqual({ left: -4, top: 50 - half });
  });

  test('computeResizeDelta handles east and south expansions with clamping', () => {
    const init = { x: 0.1, y: 0.1, w: 0.4, h: 0.3 };
    // Drag 'se' by +0.1 in x and +0.05 in y
    const res = computeResizeDelta('se', 0.1, 0.05, init);
    expect(res.x).toBe(0.1);
    expect(res.y).toBe(0.1);
    expect(Math.abs(res.w - 0.5)).toBeLessThan(0.001);
    expect(Math.abs(res.h - 0.35)).toBeLessThan(0.001);
  });

  test('computeResizeDelta handles west and north resizing without moving opposite edge', () => {
    const init = { x: 0.2, y: 0.2, w: 0.4, h: 0.4 };
    // Drag 'w' handle left by 0.05 (dx = -0.05) -> x decreases, w increases
    const resW = computeResizeDelta('w', -0.05, 0, init);
    expect(Math.abs(resW.x - 0.15)).toBeLessThan(0.001);
    expect(Math.abs(resW.w - 0.45)).toBeLessThan(0.001);

    // Drag 'n' handle down by 0.05 (dy = 0.05) -> y increases, h decreases
    const resN = computeResizeDelta('n', 0, 0.05, init);
    expect(Math.abs(resN.y - 0.25)).toBeLessThan(0.001);
    expect(Math.abs(resN.h - 0.35)).toBeLessThan(0.001);
  });

  test('computeResizeDelta respects minimum width and height constraints', () => {
    const init = { x: 0.2, y: 0.2, w: 0.1, h: 0.1 };
    // Try to shrink 'se' past minSize of 0.05
    const res = computeResizeDelta('se', -0.09, -0.09, init, { minWidth: 0.05, minHeight: 0.05 });
    expect(res.w).toBe(0.05);
    expect(res.h).toBe(0.05);
  });
});
