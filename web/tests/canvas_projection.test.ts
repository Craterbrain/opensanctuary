import { describe, expect, test } from "bun:test";
import { computeCanvasProjection, projectRect } from "../src/core/canvas_projection";

describe("computeCanvasProjection", () => {
  test("16:9 authored onto a 16:9 target: no letterbox", () => {
    const proj = computeCanvasProjection(1920, 1080, { authoredAspect: 16 / 9 });
    expect(proj.offsetX).toBeCloseTo(0, 5);
    expect(proj.offsetY).toBeCloseTo(0, 5);
    expect(proj.contentW).toBeCloseTo(1920, 5);
    expect(proj.contentH).toBeCloseTo(1080, 5);
    expect(proj.scaleX).toBeCloseTo(1, 5);
  });

  test("16:9 authored onto a 4:3 target: letterboxed top/bottom", () => {
    const proj = computeCanvasProjection(1024, 768, { authoredAspect: 16 / 9 });
    // Content width should fill the target width; height is shorter, centered vertically.
    expect(proj.contentW).toBeCloseTo(1024, 5);
    expect(proj.contentH).toBeCloseTo(1024 / (16 / 9), 5);
    expect(proj.offsetX).toBeCloseTo(0, 5);
    expect(proj.offsetY).toBeGreaterThan(0);
    expect(proj.offsetY).toBeCloseTo((768 - proj.contentH) / 2, 5);
  });

  test("16:9 authored onto a 21:9 target: pillarboxed left/right", () => {
    const proj = computeCanvasProjection(2560, 1080, { authoredAspect: 16 / 9 });
    // Content height should fill the target height; width is narrower, centered horizontally.
    expect(proj.contentH).toBeCloseTo(1080, 5);
    expect(proj.contentW).toBeCloseTo(1080 * (16 / 9), 5);
    expect(proj.offsetY).toBeCloseTo(0, 5);
    expect(proj.offsetX).toBeGreaterThan(0);
    expect(proj.offsetX).toBeCloseTo((2560 - proj.contentW) / 2, 5);
  });

  test("'cover' fit crops instead of letterboxing", () => {
    const proj = computeCanvasProjection(1024, 768, { authoredAspect: 16 / 9, fit: 'cover' });
    // With cover, content must be >= target on both axes (it overflows one axis).
    expect(proj.contentW).toBeGreaterThanOrEqual(1024 - 0.01);
    expect(proj.contentH).toBeGreaterThanOrEqual(768 - 0.01);
  });

  test("degenerate zero-size target does not throw and returns a sane fallback", () => {
    const proj = computeCanvasProjection(0, 0);
    expect(proj.contentW).toBe(0);
    expect(proj.contentH).toBe(0);
  });
});

describe("projectRect", () => {
  test("maps a normalized rect through a no-letterbox projection", () => {
    const proj = computeCanvasProjection(960, 540, { authoredAspect: 16 / 9 });
    const rect = projectRect({ x: 0.1, y: 0.2, w: 0.5, h: 0.25 }, proj);
    expect(rect.left).toBeCloseTo(96, 5);
    expect(rect.top).toBeCloseTo(108, 5);
    expect(rect.width).toBeCloseTo(480, 5);
    expect(rect.height).toBeCloseTo(135, 5);
  });

  test("maps a normalized rect through a letterboxed projection, honoring offset", () => {
    const proj = computeCanvasProjection(1024, 768, { authoredAspect: 16 / 9 });
    const rect = projectRect({ x: 0, y: 0, w: 1, h: 1 }, proj);
    // The full-canvas rect should exactly equal the projection's content box.
    expect(rect.left).toBeCloseTo(proj.offsetX, 5);
    expect(rect.top).toBeCloseTo(proj.offsetY, 5);
    expect(rect.width).toBeCloseTo(proj.contentW, 5);
    expect(rect.height).toBeCloseTo(proj.contentH, 5);
  });

  test("element positions stay proportionally consistent across different target aspect ratios", () => {
    const elementRect = { x: 0.25, y: 0.25, w: 0.5, h: 0.5 }; // centered square-ish box
    const proj169 = computeCanvasProjection(1920, 1080, { authoredAspect: 16 / 9 });
    const proj43 = computeCanvasProjection(1024, 768, { authoredAspect: 16 / 9 });

    const r169 = projectRect(elementRect, proj169);
    const r43 = projectRect(elementRect, proj43);

    // Relative position within the CONTENT box (not the raw target box) must match
    // regardless of the target's own aspect ratio — this is the whole point of
    // letterboxing: the authored layout is preserved, never stretched or skewed.
    const relLeft169 = (r169.left - proj169.offsetX) / proj169.contentW;
    const relLeft43 = (r43.left - proj43.offsetX) / proj43.contentW;
    expect(relLeft169).toBeCloseTo(relLeft43, 5);
    expect(relLeft169).toBeCloseTo(0.25, 5);

    const relWidth169 = r169.width / proj169.contentW;
    const relWidth43 = r43.width / proj43.contentW;
    expect(relWidth169).toBeCloseTo(relWidth43, 5);
  });
});
