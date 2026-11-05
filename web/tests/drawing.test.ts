import { describe, test, expect } from "bun:test";
import { scalePath } from "../src/editor/svg_path";

describe("path data in an element's 0..100 box", () => {
  test("points scale per axis; arc radii scale, arc flags don't", () => {
    expect(scalePath("M 0 0 L 100 50 C 10 20 30 40 50 60 Z", 200, 10)).toBe("M 0 0 L 200 5 C 20 2 60 4 100 6 Z");
    expect(scalePath("M 0 50 A 50 50 0 1 0 100 50", 300, 100)).toBe("M 0 50 A 150 50 0 1 0 300 50");
    // A moveto's extra pairs are linetos: a pie wedge goes out from its centre to the rim.
    expect(scalePath("M 50 50 100 50 A 50 50 0 0 1 50 100 Z", 200, 100)).toBe("M 100 50 L 200 50 A 100 50 0 0 1 100 100 Z");
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

describe("imported fills", () => {
  test("a CSS gradient fill becomes an SVG gradient over the shape's box; a colour stays as is", async () => {
    const { svgPaint } = await import("../src/editor/svg_path");
    const node = (tag: string): any => ({
      tag, attrs: {} as Record<string, string>, children: [] as any[],
      setAttribute(k: string, v: string) { this.attrs[k] = v; },
      appendChild(c: any) { this.children.push(c); return c; },
      insertBefore(c: any) { this.children.unshift(c); return c; },
      querySelector(sel: string) { return this.children.find((c: any) => c.tag === sel) ?? null; },
    });
    const original = (globalThis as any).document;
    (globalThis as any).document = { createElementNS: (_ns: string, tag: string) => node(tag) };
    try {
      const svg = node("svg");
      expect(svgPaint(svg, "#11223380")).toBe("#11223380");
      const paint = svgPaint(svg, "linear-gradient(180deg, #ff0000 0%, #0000ff80 100%)");
      const grad = svg.querySelector("defs").children[0];
      expect(paint).toBe(`url(#${grad.attrs.id})`);
      expect(grad.tag).toBe("linearGradient");
      // 180deg runs top to bottom.
      expect([grad.attrs.x1, grad.attrs.y1, grad.attrs.x2, grad.attrs.y2].map(Number).map((v: number) => Math.round(v * 100) / 100)).toEqual([0.5, 0, 0.5, 1]);
      expect(grad.children.map((s: any) => [s.attrs.offset, s.attrs["stop-color"]])).toEqual([["0%", "#ff0000"], ["100%", "#0000ff80"]]);
      expect(svg.querySelector("defs").children.length).toBe(1);
      svgPaint(svg, "radial-gradient(circle, #ffffff 0%, #000000 100%)");
      expect(svg.querySelector("defs").children[1].tag).toBe("radialGradient");
    } finally {
      (globalThis as any).document = original;
    }
  });
});

describe("paragraph spacing", () => {
  test("only a run of exactly a newline separates paragraphs; a line break inside a run doesn't", async () => {
    const { splitParagraphs } = await import("../src/editor/text_block");
    const r = (text: string) => ({ text, font_size_pt: 20 }) as any;
    const paras = splitParagraphs([r("One\n"), r("Two"), r("\n"), r("\n"), r("Three")]);
    expect(paras.map((p) => p.runs.map((x: any) => x.text))).toEqual([["One\n", "Two"], [], ["Three"]]);
    expect(paras[1]!.lineStyle?.text).toBe("\n");
  });
});
