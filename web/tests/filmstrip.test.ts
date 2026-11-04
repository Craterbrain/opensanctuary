import { describe, test, expect, beforeAll } from 'bun:test';
import { FilmstripSidebar, applyTagBtnStyle, applyTimerBtnStyle } from '../src/editor/thumbnail';

function createMockEl(tag: string = 'div') {
  const children: any[] = [];
  const classes = new Set<string>();
  const el: any = {
    tagName: tag.toUpperCase(),
    style: {},
    dataset: {},
    className: '',
    children,
    classList: {
      add: (...cs: string[]) => cs.forEach(c => classes.add(c)),
      remove: (...cs: string[]) => cs.forEach(c => classes.delete(c)),
      toggle: (c: string, force?: boolean) => {
        const val = force !== undefined ? force : !classes.has(c);
        if (val) classes.add(c); else classes.delete(c);
        return val;
      },
      has: (c: string) => classes.has(c),
      contains: (c: string) => classes.has(c)
    },
    set innerHTML(val: string) {
      if (!val) children.length = 0;
    },
    get innerHTML() { return ''; },
    appendChild: (child: any) => { children.push(child); return child; },
    querySelectorAll: (sel: string) => {
      const results: any[] = [];
      const walk = (node: any) => {
        if (node.className && node.className.split(/\s+/).includes(sel.replace(/^\./, ''))) {
          results.push(node);
        }
        (node.children || []).forEach(walk);
      };
      (el.children || []).forEach(walk);
      return results;
    },
    addEventListener: () => {},
    removeEventListener: () => {}
  };
  return el;
}

beforeAll(() => {
  const doc = (globalThis as any).document || {};
  doc.createElement = createMockEl;
  if (!doc.getElementById) doc.getElementById = () => null;
  doc.body = createMockEl('body');
  if (!doc.addEventListener) doc.addEventListener = () => {};
  if (!doc.removeEventListener) doc.removeEventListener = () => {};
  (globalThis as any).document = doc;
});

describe('Filmstrip buttons and cards', () => {
  test('applyTagBtnStyle and applyTimerBtnStyle map classes accurately', () => {
    const mockEl = {
      classList: new Set<string>(),
      className: ''
    };
    const elShim = {
      classList: {
        add: (c: string) => mockEl.classList.add(c),
        remove: (...cs: string[]) => cs.forEach(c => mockEl.classList.delete(c)),
        has: (c: string) => mockEl.classList.has(c)
      }
    } as any;

    applyTagBtnStyle(elShim, 'V1');
    expect(elShim.classList.has('fs-tag-verse')).toBe(true);

    applyTagBtnStyle(elShim, 'C');
    expect(elShim.classList.has('fs-tag-chorus')).toBe(true);

    applyTagBtnStyle(elShim, 'B');
    expect(elShim.classList.has('fs-tag-bridge')).toBe(true);

    applyTagBtnStyle(elShim, 'E');
    expect(elShim.classList.has('fs-tag-ending')).toBe(true);

    applyTagBtnStyle(elShim, undefined);
    expect(elShim.classList.has('fs-tag-neutral')).toBe(true);

    applyTimerBtnStyle(elShim, 5);
    expect(elShim.classList.has('fs-timer-active')).toBe(true);

    applyTimerBtnStyle(elShim, null);
    expect(elShim.classList.has('fs-timer-inactive')).toBe(true);
  });

  test('hides slide tagger outside of song mode and keeps timer visible', () => {
    const container = document.createElement('div');
    document.body.appendChild(container);

    const slides: EditorSlide[] = [
      { id: 'slide-1', text: 'Slide 1', tag: 'V1', duration_seconds: 5, elements: [] },
      { id: 'slide-2', text: 'Slide 2', tag: 'C', duration_seconds: null, elements: [] }
    ];

    const filmstrip = new FilmstripSidebar(container, {}, 'song');
    filmstrip.setSlides(slides, 0);

    // Song mode: tag buttons are visible
    const tagBtnsSong = container.querySelectorAll<HTMLElement>('.fs-slide-tag-btn');
    expect(tagBtnsSong.length).toBe(2);
    tagBtnsSong.forEach(btn => {
      expect(btn.style.display).not.toBe('none');
    });

    // Timer buttons visible
    const timerBtnsSong = container.querySelectorAll<HTMLElement>('.fs-slide-timer-btn');
    expect(timerBtnsSong.length).toBe(2);
    timerBtnsSong.forEach(btn => {
      expect(btn.style.display).not.toBe('none');
    });

    // Switch to Presentation mode: tag buttons hidden (display: none)
    filmstrip.setMode('presentation');
    expect(filmstrip.isSongMode()).toBe(false);
    expect(container.classList.contains('mode-song')).toBe(false);
    const tagBtnsPres = container.querySelectorAll<HTMLElement>('.fs-slide-tag-btn');
    expect(tagBtnsPres.length).toBe(2);
    tagBtnsPres.forEach(btn => {
      expect(btn.style.display).toBe('none');
    });

    // Timer buttons remain visible in presentation mode
    const timerBtnsPres = container.querySelectorAll<HTMLElement>('.fs-slide-timer-btn');
    expect(timerBtnsPres.length).toBe(2);
    timerBtnsPres.forEach(btn => {
      expect(btn.style.display).not.toBe('none');
    });

    // Switch to Scripture mode: tag buttons hidden
    filmstrip.setMode('scripture');
    expect(filmstrip.isSongMode()).toBe(false);
    const tagBtnsScrip = container.querySelectorAll<HTMLElement>('.fs-slide-tag-btn');
    tagBtnsScrip.forEach(btn => {
      expect(btn.style.display).toBe('none');
    });

    // Switch back to Song mode: tag buttons visible
    filmstrip.setMode('song');
    expect(filmstrip.isSongMode()).toBe(true);
    expect(container.classList.contains('mode-song')).toBe(true);
    const tagBtnsBack = container.querySelectorAll<HTMLElement>('.fs-slide-tag-btn');
    tagBtnsBack.forEach(btn => {
      expect(btn.style.display).not.toBe('none');
    });
  });

  test('hide button shows the slide state and hidden cards are marked', () => {
    const container = document.createElement('div');
    document.body.appendChild(container);

    const slides: EditorSlide[] = [
      { id: 'slide-1', text: 'Slide 1', elements: [] },
      { id: 'slide-2', text: 'Slide 2', hidden: true, elements: [] }
    ];
    const filmstrip = new FilmstripSidebar(container, {}, 'presentation');
    filmstrip.setSlides(slides, 0);

    const hideBtns = container.querySelectorAll<HTMLElement>('.fs-slide-hide-btn');
    expect(hideBtns.length).toBe(2);
    const cards = container.querySelectorAll<HTMLElement>('.editor-filmstrip-card');
    expect(cards[0].classList.contains('slide-hidden')).toBe(false);
    expect(cards[1].classList.contains('slide-hidden')).toBe(true);
    expect(hideBtns[0].textContent).toBe('👁');
    expect(hideBtns[1].textContent).toBe('🚫');
    expect(hideBtns[1].title).toContain('Hidden slide');
  });
});
