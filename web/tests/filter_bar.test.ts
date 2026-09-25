import { describe, test, expect, beforeEach, afterEach } from "bun:test";
import {
  createSearchInput,
  createCategoryPills,
  renderEmptyState
} from "../src/ui/filter_bar.ts";

describe("Universal Filter Bar & Search: filter_bar.ts", () => {
  beforeEach(() => {
    const createMockElement = (tag: string) => {
      const listeners: Record<string, Function[]> = {};
      const children: any[] = [];
      const classes = new Set<string>();
      const attrs: Record<string, string> = {};

      const el: any = {
        tagName: tag.toUpperCase(),
        children,
        style: {},
        dataset: {},
        value: "",
        placeholder: "",
        title: "",
        type: "text",
        innerHTML: "",
        textContent: "",
        focus: () => {},
        classList: {
          add: (c: string) => classes.add(c),
          remove: (c: string) => classes.delete(c),
          contains: (c: string) => classes.has(c),
          toggle: (c: string, force?: boolean) => {
            if (force === undefined) {
              if (classes.has(c)) classes.delete(c);
              else classes.add(c);
            } else if (force) {
              classes.add(c);
            } else {
              classes.delete(c);
            }
          }
        },
        get className() {
          return Array.from(classes).join(' ');
        },
        set className(val: string) {
          classes.clear();
          (val || '').split(/\s+/).filter(Boolean).forEach(c => classes.add(c));
        },
        setAttribute: (name: string, val: string) => {
          attrs[name] = val;
        },
        getAttribute: (name: string) => {
          return attrs[name] ?? null;
        },
        appendChild: (child: any) => {
          children.push(child);
          return child;
        },
        contains: (child: any) => {
          const check = (node: any): boolean => {
            if (node === child) return true;
            for (const ch of node.children || []) {
              if (check(ch)) return true;
            }
            return false;
          };
          return check(el);
        },
        addEventListener: (event: string, fn: Function) => {
          if (!listeners[event]) listeners[event] = [];
          listeners[event].push(fn);
        },
        dispatchEvent: (event: any) => {
          const ev = typeof event === 'string' ? { type: event } : event;
          (listeners[ev.type] || []).forEach(fn => fn(ev));
        },
        click: () => {
          (listeners['click'] || []).forEach(fn => fn({ type: 'click', stopPropagation: () => {} }));
        },
        querySelector: (sel: string) => {
          const matches = (node: any): boolean => {
            if (sel.startsWith('.') && node.classList?.contains(sel.slice(1))) return true;
            if (sel.startsWith('#') && node.id === sel.slice(1)) return true;
            return false;
          };
          const findIn = (node: any): any => {
            for (const ch of node.children || []) {
              if (matches(ch)) return ch;
              const sub = findIn(ch);
              if (sub) return sub;
            }
            return null;
          };
          return findIn(el);
        },
        querySelectorAll: (sel: string) => {
          const results: any[] = [];
          const matches = (node: any): boolean => {
            if (sel.startsWith('.') && node.classList?.contains(sel.slice(1))) return true;
            return false;
          };
          const collect = (node: any) => {
            for (const ch of node.children || []) {
              if (matches(ch)) results.push(ch);
              collect(ch);
            }
          };
          collect(el);
          return results;
        }
      };
      return el;
    };

    (globalThis as any).document = {
      createElement: createMockElement
    };
  });

  afterEach(() => {
    delete (globalThis as any).document;
  });

  test("createSearchInput sets up input, clear button, and search dispatch", async () => {
    let lastQuery = "";
    const handle = createSearchInput({
      placeholder: "Filter songs...",
      debounceMs: 10,
      onSearch: (q) => {
        lastQuery = q;
      }
    });

    expect(handle.container).toBeDefined();
    expect(handle.input).toBeDefined();
    expect(handle.clearBtn).toBeDefined();
    expect(handle.input.placeholder).toBe("Filter songs...");
    expect(handle.clearBtn.classList.contains("visible")).toBe(false);

    // Simulate typing
    handle.input.value = "Amazing Grace";
    handle.input.dispatchEvent({ type: "input" });
    expect(handle.clearBtn.classList.contains("visible")).toBe(true);

    // Wait for debounced search
    await new Promise((r) => setTimeout(r, 25));
    expect(lastQuery).toBe("Amazing Grace");

    // Click clear
    handle.clearBtn.click();
    expect(handle.input.value).toBe("");
    expect(handle.clearBtn.classList.contains("visible")).toBe(false);
    expect(lastQuery).toBe("");
  });

  test("createSearchInput supports programmatic setValue and clear", () => {
    let lastQuery = "";
    const handle = createSearchInput({
      initialValue: "Initial",
      onSearch: (q) => {
        lastQuery = q;
      }
    });

    expect(handle.getValue()).toBe("Initial");
    expect(handle.clearBtn.classList.contains("visible")).toBe(true);

    handle.setValue("New Query", true);
    expect(handle.getValue()).toBe("New Query");
    expect(lastQuery).toBe("New Query");

    handle.clear();
    expect(handle.getValue()).toBe("");
    expect(lastQuery).toBe("");
  });

  test("createCategoryPills renders accessible pills and handles tab selection", () => {
    const categories = [
      { id: "all", label: "All Items", count: 12 },
      { id: "nature", label: "Nature", icon: "🌲" },
      { id: "worship", label: "Sanctuary", icon: "⛪" }
    ];

    let currentCat = "all";
    const handle = createCategoryPills({
      categories,
      activeId: "all",
      onChange: (catId) => {
        currentCat = catId;
      }
    });

    expect(handle.container).toBeDefined();
    expect(handle.container.getAttribute("role")).toBe("tablist");

    const buttons = handle.container.querySelectorAll(".import-tab-btn");
    expect(buttons.length).toBe(3);
    expect(buttons[0].textContent).toContain("All Items (12)");
    expect(buttons[0].classList.contains("active")).toBe(true);
    expect(buttons[0].getAttribute("aria-selected")).toBe("true");

    expect(buttons[1].textContent).toContain("🌲 Nature");
    expect(buttons[1].classList.contains("active")).toBe(false);

    // Click second pill
    buttons[1].click();
    expect(currentCat).toBe("nature");
    expect(handle.getActive()).toBe("nature");
    expect(buttons[1].classList.contains("active")).toBe(true);
    expect(buttons[0].classList.contains("active")).toBe(false);
  });

  test("renderEmptyState populates icon, title, description, and action button", () => {
    const container = (globalThis as any).document.createElement("div");
    let actionTriggered = false;

    const el = renderEmptyState(container, {
      icon: "🔍",
      title: "No Media Found",
      description: "No items match your query. Try a different term.",
      actionLabel: "Clear Filter",
      onAction: () => {
        actionTriggered = true;
      }
    });

    expect(el).toBeDefined();
    expect(container.contains(el)).toBe(true);

    const btn = el.querySelector(".btn-empty-action");
    expect(btn).toBeDefined();
    expect(btn?.textContent).toBe("Clear Filter");

    btn?.click();
    expect(actionTriggered).toBe(true);
  });
});
