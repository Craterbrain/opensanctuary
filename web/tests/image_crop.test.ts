import { describe, expect, test } from "bun:test";
import { computeCropBackgroundStyle } from "../src/editor/image_tools";

describe("computeCropBackgroundStyle", () => {
  test("full-image crop (x:0,y:0,w:1,h:1) sizes/positions as a plain cover", () => {
    const { size, position } = computeCropBackgroundStyle({ x: 0, y: 0, w: 1, h: 1 });
    expect(size).toBe("100% 100%");
    expect(position).toBe("0% 0%");
  });

  test("cropping the left half (w:0.5) doubles background-size width and anchors left", () => {
    const { size, position } = computeCropBackgroundStyle({ x: 0, y: 0, w: 0.5, h: 1 });
    expect(size).toBe("200% 100%");
    expect(position).toBe("0% 0%");
  });

  test("cropping the right half (x:0.5,w:0.5) anchors fully right (100%)", () => {
    const { size, position } = computeCropBackgroundStyle({ x: 0.5, y: 0, w: 0.5, h: 1 });
    expect(size).toBe("200% 100%");
    // x / (1 - w) = 0.5 / 0.5 = 1 -> 100%
    expect(position).toBe("100% 0%");
  });

  test("cropping a centered quarter window (x:0.25,y:0.25,w:0.5,h:0.5)", () => {
    const { size, position } = computeCropBackgroundStyle({ x: 0.25, y: 0.25, w: 0.5, h: 0.5 });
    expect(size).toBe("200% 200%");
    // 0.25 / (1-0.5) = 0.5 -> 50%
    expect(position).toBe("50% 50%");
  });

  test("clamps an out-of-range crop rect instead of producing negative/blown-up values", () => {
    const { size, position } = computeCropBackgroundStyle({ x: 0.9, y: 0.9, w: 0.5, h: 0.5 });
    // x is clamped to (1 - w) = 0.5 so the window stays within the image.
    expect(size).toBe("200% 200%");
    expect(position).toBe("100% 100%");
  });
});
