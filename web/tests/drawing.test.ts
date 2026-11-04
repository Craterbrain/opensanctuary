import { describe, test, expect } from "bun:test";
import { scalePath } from "../src/editor/svg_path";

describe("path data in an element's 0..100 box", () => {
  test("points scale per axis; arc radii scale, arc flags don't", () => {
    expect(scalePath("M 0 0 L 100 50 C 10 20 30 40 50 60 Z", 200, 10)).toBe("M 0 0 L 200 5 C 20 2 60 4 100 6 Z");
    expect(scalePath("M 0 50 A 50 50 0 1 0 100 50", 300, 100)).toBe("M 0 50 A 150 50 0 1 0 300 50");
  });
  test("repeated coordinates after one command are all scaled", () => {
    expect(scalePath("M 0 0 L 50 0 50 100 100 100", 10, 10)).toBe("M 0 0 L 5 0 L 5 10 L 10 10");
  });
});

describe("group children", () => {
  test("a child placed in slide coordinates is moved into its group's box", async () => {
    const { placeInGroup } = await import("../src/editor/group_layout");
    const child = { style: { left: "300px", top: "250px" } } as unknown as HTMLElement;
    placeInGroup(child, { left: 200, top: 50 });
    expect([child.style.left, child.style.top]).toEqual(["100px", "200px"]);
  });
});

describe("pictures keep the position their renderer gave them", () => {
  test("an absolutely placed image or video box stays absolute (relative stacked them down the slide)", async () => {
    const { renderImageDOM } = await import("../src/editor/image_tools");
    const { renderVideoDOM } = await import("../src/editor/video_tools");
    const fakeEl = (): any => ({ style: {} as Record<string, string>, dataset: {}, children: [] as any[], appendChild(c: any) { this.children.push(c); return c; }, setAttribute() {}, addEventListener() {}, set innerHTML(_v: string) {} });
    const original = (globalThis as any).document;
    (globalThis as any).document = { createElement: () => fakeEl() };
    try {
      for (const render of [() => { const el = fakeEl(); el.style.position = "absolute"; renderImageDOM(el, { file_path: "/media/images/a.png" } as any); return el; },
                            () => { const el = fakeEl(); el.style.position = "absolute"; renderVideoDOM(el, { file_path: "/media/videos/a.mp4" } as any); return el; }]) {
        expect(render().style.position).toBe("absolute");
      }
      const loose = fakeEl();
      renderImageDOM(loose, { file_path: "/media/images/a.png" } as any);
      expect(loose.style.position).toBe("relative");
    } finally {
      (globalThis as any).document = original;
    }
  });
});
