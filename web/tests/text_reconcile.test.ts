import { describe, test, expect } from "bun:test";
import { reconcileTextBlockFromDOM } from "../src/editor/text_selection";

// The paragraph layout text_block.ts draws for imported text (a box per paragraph, a hanging lead,
// a stand-in character for an empty paragraph), built as plain objects -- these tests run without a
// browser DOM -- and read back as an edit reads it.
(globalThis as any).Node ??= { TEXT_NODE: 3 };
const text = (t: string): any => ({ nodeType: 3, nodeName: "#text", textContent: t, childNodes: [] });
const span = (kids: any[], opts: { lead?: boolean; placeholder?: boolean; style?: Record<string, string> } = {}): any => ({
  nodeType: 1,
  nodeName: "SPAN",
  style: opts.style ?? {},
  dataset: { ...(opts.lead ? { lead: "1" } : {}), ...(opts.placeholder ? { placeholder: "1" } : {}) },
  childNodes: kids,
  get textContent() { return kids.map((k: any) => k.textContent).join(""); },
});
const div = (kids: any[]): any => ({ nodeType: 1, nodeName: "DIV", style: {}, dataset: {}, childNodes: kids });

describe("reading imported paragraphs back after an edit", () => {
  test("an empty paragraph's stand-in isn't saved, an empty lead is kept, breaks keep the look", () => {
    const container: any = { childNodes: [
      div([span([text("• ")], { lead: true }), span([span([text("First")], { style: { fontWeight: "bold" } })])]),
      div([span([text("​")], { placeholder: true, style: { fontSize: "20pt" } })]),
      div([span([], { lead: true }), span([span([text("No bullet")])])]),
    ] };
    const runs = reconcileTextBlockFromDOM(container);
    expect(runs.map((r) => r.text)).toEqual(["• ", "First", "\n", "\n", "", "No bullet"]);
    expect(runs.some((r) => r.text.includes("​"))).toBe(false);
    // The break after "First" carries its look (bold), not nothing.
    expect(runs[2]!.bold).toBe(true);
  });
});
