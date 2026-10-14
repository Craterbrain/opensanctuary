import { test, expect, describe, beforeEach, afterAll } from "bun:test";

// A just-enough DOM: elements with children, attributes, classes and listeners.
class El {
  tag: string; children: El[] = []; attrs: Record<string, string> = {}; dataset: Record<string, string> = {};
  className = ""; hidden = false; disabled = false; type = ""; title = ""; src = ""; alt = ""; draggable = false;
  textContent = ""; listeners: Record<string, Array<(e: any) => void>> = {}; blurred = 0;
  constructor(tag: string) { this.tag = tag; }
  get classList() {
    const self = this;
    return { toggle(c: string, on: boolean) { const set = new Set(self.className.split(" ").filter(Boolean)); on ? set.add(c) : set.delete(c); self.className = [...set].join(" "); } };
  }
  setAttribute(k: string, v: string) { this.attrs[k] = v; }
  getAttribute(k: string) { return this.attrs[k]; }
  appendChild(c: El) { this.children.push(c); return c; }
  append(...cs: El[]) { cs.forEach((c) => this.children.push(c)); }
  replaceChildren() { this.children = []; }
  addEventListener(t: string, fn: (e: any) => void) { (this.listeners[t] ??= []).push(fn); }
  blur() { this.blurred++; }
  click() { (this.listeners.click ?? []).forEach((fn) => fn({ stopPropagation() {} })); }
  all(): El[] { return [this, ...this.children.flatMap((c) => c.all())]; }
}
const root = new El("div");
// The fakes below must not outlive this file: bun runs every test file in one process.
const saved = ["document", "window", "localStorage"].map((k) => [k, Object.prototype.hasOwnProperty.call(globalThis, k), (globalThis as any)[k]] as const);
afterAll(() => { for (const [k, had, v] of saved) { if (had) (globalThis as any)[k] = v; else delete (globalThis as any)[k]; } });
(globalThis as any).document = {
  getElementById: (id: string) => (id === "ribbon-plugins" ? root : null),
  createElement: (t: string) => new El(t),
  createTextNode: (t: string) => Object.assign(new El("#text"), { textContent: t }),
  addEventListener: () => {},
};
(globalThis as any).window = (globalThis as any).window ?? globalThis;
(globalThis as any).localStorage = (globalThis as any).localStorage ?? { getItem: () => null, setItem: () => {} };
const T = await import("../src/core/plugin_toolbar.ts");

const plugin = (id: string, n: number) => ({
  id, name: `Plugin ${id}`,
  toolbar: Array.from({ length: n }, (_, i) => ({ id: `b${i}`, label: `Btn ${i}`, icon: "refresh", tooltip: `Do ${i}` })),
});
const buttons = () => root.all().filter((e) => e.tag === "button" && e.dataset.pluginId);

describe("plugin ribbon buttons", () => {
  beforeEach(() => { T.resetToolbarForTests(); root.children = []; root.hidden = true; });

  test("shows a marked group with attributed buttons and hides when empty", () => {
    expect(root.hidden).toBe(true);
    T.addToolbarItems(plugin("a", 2), () => {});
    expect(root.hidden).toBe(false);
    const caption = root.children[0];
    expect(caption.textContent).toBe("Plugins");
    const [b0] = buttons();
    expect(b0.title).toBe("Plugin a (plugin): Do 0");
    expect(b0.attrs["aria-label"]).toBe("Plugin a (plugin): Do 0");
    expect(b0.all().some((e) => e.className === "plugin-badge")).toBe(true);
    expect(b0.all().find((e) => e.className === "label")!.textContent).toBe("Btn 0");
    // the icon is only ever a name resolved to one of the app's own files
    const img = b0.all().find((e) => e.tag === "img")!;
    expect(img.dataset.icon).toBe("refresh");
    expect(img.src).toMatch(/^icons\/[a-z0-9-]+\/refresh\.svg$/);
    T.removeToolbarItems("a");
    expect(root.hidden).toBe(true);
    expect(buttons()).toHaveLength(0);
  });

  test("text from a manifest is never markup", () => {
    T.addToolbarItems({ id: "x", name: "<img src=x onerror=1>", toolbar: [{ id: "b", label: "<b>hi</b>", icon: "refresh", tooltip: "<script>" }] }, () => {});
    const b = buttons()[0];
    expect(b.all().find((e) => e.className === "label")!.textContent).toBe("<b>hi</b>");
    expect(b.title).toBe("<img src=x onerror=1> (plugin): <script>");
    expect(b.all().every((e) => !("innerHTML" in e))).toBe(true);
  });

  test("more than six buttons collapse into a menu, in a stable order", () => {
    T.addToolbarItems(plugin("a", 2), () => {});
    T.addToolbarItems(plugin("b", 2), () => {});
    T.addToolbarItems(plugin("c", 2), () => {});
    T.addToolbarItems(plugin("d", 2), () => {});
    const visible = buttons().filter((b) => b.className.includes("ribbon-btn--plugin") && b.dataset.itemId);
    expect(visible).toHaveLength(T.MAX_VISIBLE_TOOLBAR_BUTTONS);
    expect(visible.map((b) => `${b.dataset.pluginId}${b.dataset.itemId}`)).toEqual(["ab0", "ab1", "bb0", "bb1", "cb0", "cb1"]);
    const more = root.all().find((e) => e.className.includes("ribbon-plugins-more"))!;
    expect(more.all().find((e) => e.className === "label")!.textContent).toBe("More ▾ (2)");
    more.click();
    const menuItems = root.all().filter((e) => e.className === "ribbon-plugins-menu-item");
    expect(menuItems.map((e) => e.all().find((x) => x.className === "label")!.textContent)).toEqual(["Plugin d: Btn 0", "Plugin d: Btn 1"]);
  });

  test("a click reaches the plugin once per debounce window and the button gives up focus", () => {
    const clicks: string[] = [];
    T.addToolbarItems(plugin("a", 1), (p, i) => clicks.push(`${p}/${i}`));
    const b = buttons()[0];
    b.click();
    expect(clicks).toEqual(["a/b0"]);
    expect(b.blurred).toBeGreaterThan(0);
    buttons()[0].click(); // immediately again: debounced
    expect(clicks).toEqual(["a/b0"]);
  });

  test("only on/off and pressed can be updated, only for the plugin's own buttons", () => {
    T.addToolbarItems(plugin("a", 1), () => {});
    expect(T.updateToolbarItem("a", "nope", { disabled: true })).toBe(false);
    expect(T.updateToolbarItem("zzz", "b0", { disabled: true })).toBe(false);
    expect(T.updateToolbarItem("a", "b0", { disabled: true, pressed: true, label: "Go Live" } as any)).toBe(true);
    let b = buttons()[0];
    expect(b.disabled).toBe(true);
    expect(b.attrs["aria-pressed"]).toBe("true");
    expect(b.all().find((e) => e.className === "label")!.textContent).toBe("Btn 0"); // the label can't change
    T.updateToolbarItem("a", "b0", { disabled: "yes" as any }); // not a boolean: ignored
    b = buttons()[0];
    expect(b.disabled).toBe(true);
  });

  test("re-adding a plugin replaces its buttons instead of duplicating them", () => {
    T.addToolbarItems(plugin("a", 2), () => {});
    T.addToolbarItems(plugin("a", 1), () => {});
    expect(buttons()).toHaveLength(1);
  });
});
