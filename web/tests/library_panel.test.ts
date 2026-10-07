import { describe, test, expect, beforeAll, afterAll } from 'bun:test';

/**
 * library_panel.ts resolves several DOM elements (catalogTableBody,
 * categoryTreeContainer, etc.) at MODULE TOP LEVEL via document.getElementById
 * -- unlike its sibling ui/*.ts modules (grid_card.ts, dialog_manager.ts),
 * which only touch `document` lazily inside the functions they export. That
 * makes most of this file's rendering functions impractical to unit-test
 * without a real DOM (they also call the real @atlaskit/pragmatic-drag-and-drop
 * library against the elements they create, which expects a real DOM element
 * API this file's fake `document` doesn't attempt to fully replicate).
 *
 * `highlightCatalogRow` is the one exported function this file has that
 * doesn't need either of those things -- it only reads `catalogTableBody`
 * (set up here as a fake element before the module's first import) and calls
 * plain classList/focus methods on whatever rows are already its children,
 * which this test adds directly rather than going through the real render
 * pipeline. This is a regression test for a real bug fixed in this file:
 * `renderCatalogTable` prepends a decorative ".catalog-fetch-prompt-row"
 * (the "Search Genius for Christian Lyrics" prompt) ahead of the real item
 * rows whenever the Songs tab's search box has 2+ characters, which used to
 * shift `highlightCatalogRow(idx)`'s raw positional DOM lookup off by one --
 * highlighting/focusing the prompt row instead of the item actually clicked.
 */

function makeFakeElement(tag: string) {
  const classes = new Set<string>();
  const el: any = {
    tagName: tag.toUpperCase(),
    children: [] as any[],
    dataset: {},
    focused: false,
    classList: {
      add: (c: string) => classes.add(c),
      remove: (c: string) => classes.delete(c),
      contains: (c: string) => classes.has(c),
      toggle: (c: string, force?: boolean) => {
        const shouldHave = force !== undefined ? force : !classes.has(c);
        if (shouldHave) classes.add(c); else classes.delete(c);
      },
    },
    appendChild: (child: any) => { el.children.push(child); return child; },
    focus: () => { el.focused = true; },
    addEventListener: () => {},
    removeEventListener: () => {},
    // Just enough to support the two selectors this file's functions use:
    // "tr" and "tr:not(.some-class)".
    querySelectorAll: (sel: string) => {
      const m = /^tr(?:\s*:not\(\.([\w-]+)\))?$/.exec(sel.trim());
      if (!m) return [];
      const excludeClass = m[1];
      return el.children.filter((c: any) => c.tagName === 'TR' && (!excludeClass || !c.classList.contains(excludeClass)));
    },
  };
  return el;
}

function makeFakeDocument() {
  const registry: Record<string, any> = {};
  return {
    getElementById: (id: string) => {
      if (!registry[id]) registry[id] = makeFakeElement('div');
      return registry[id];
    },
    createElement: (tag: string) => makeFakeElement(tag),
    querySelector: (_sel: string) => null,
    querySelectorAll: (_sel: string) => [],
    addEventListener: () => {},
  };
}

let fakeDocument: ReturnType<typeof makeFakeDocument>;
let highlightCatalogRow: (idx: number) => void;

describe('library_panel.ts: highlightCatalogRow', () => {
  beforeAll(async () => {
    // Only needs to exist for this one, first import of the module --
    // library_panel.ts reads `document.getElementById('catalog-table-body')`
    // etc. at module top level, so this has to be in place before that
    // line runs, not just before a test body. A dynamic `import()` is what
    // makes that possible from a bun:test file (its top-level code runs
    // synchronously once, the first time this specifier is imported in
    // this process -- every later `import()` of it, including bun's own
    // module cache across other test files, just returns the same
    // already-evaluated module without touching `document` again).
    const ORIGINAL_DOCUMENT = (globalThis as any).document;
    fakeDocument = makeFakeDocument();
    (globalThis as any).document = fakeDocument;
    try {
      ({ highlightCatalogRow } = await import('../src/ui/library_panel.ts'));
    } finally {
      // Restore immediately -- library_panel.ts's module-level code has
      // already run synchronously by the time the import above resolves,
      // so leaving a fake `document` installed globally past this point
      // would only leak into whichever unrelated test file bun happens to
      // run next (see pairing.test.ts's comment on this exact class of bug).
      (globalThis as any).document = ORIGINAL_DOCUMENT;
    }
  });

  test('skips the leading Genius-search prompt row so item index N highlights the row actually at index N', () => {
    const tableBody = fakeDocument.getElementById('catalog-table-body');
    tableBody.children.length = 0;

    const promptRow = fakeDocument.createElement('tr');
    promptRow.classList.add('catalog-fetch-prompt-row');
    const itemRow0 = fakeDocument.createElement('tr');
    const itemRow1 = fakeDocument.createElement('tr');
    tableBody.appendChild(promptRow);
    tableBody.appendChild(itemRow0);
    tableBody.appendChild(itemRow1);

    highlightCatalogRow(0);
    expect(promptRow.classList.contains('selected')).toBe(false);
    expect(itemRow0.classList.contains('selected')).toBe(true);
    expect(itemRow0.focused).toBe(true);
    expect(itemRow1.classList.contains('selected')).toBe(false);

    highlightCatalogRow(1);
    expect(itemRow0.classList.contains('selected')).toBe(false);
    expect(itemRow1.classList.contains('selected')).toBe(true);
    expect(itemRow1.focused).toBe(true);
  });

  test('behaves the same with no prompt row present (empty/short search query)', () => {
    const tableBody = fakeDocument.getElementById('catalog-table-body');
    tableBody.children.length = 0;

    const itemRow0 = fakeDocument.createElement('tr');
    const itemRow1 = fakeDocument.createElement('tr');
    tableBody.appendChild(itemRow0);
    tableBody.appendChild(itemRow1);

    highlightCatalogRow(1);
    expect(itemRow0.classList.contains('selected')).toBe(false);
    expect(itemRow1.classList.contains('selected')).toBe(true);
  });
});
