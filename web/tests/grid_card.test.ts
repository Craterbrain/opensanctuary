import { describe, test, expect, beforeEach, afterEach } from 'bun:test';
import { buildGridCard, GridSelectionController } from '../src/ui/grid_card';

describe('Universal Grid Card: grid_card.ts', () => {
  beforeEach(() => {
    (globalThis as any).document = {
      createElement: (tag: string) => {
        const listeners: Record<string, Function[]> = {};
        const children: any[] = [];
        const classes = new Set<string>();

        const el: any = {
          tagName: tag.toUpperCase(),
          children,
          style: {},
          dataset: {},
          classList: {
            add: (c: string) => classes.add(c),
            remove: (c: string) => classes.delete(c),
            contains: (c: string) => classes.has(c),
          },
          get className() {
            return Array.from(classes).join(' ');
          },
          set className(val: string) {
            classes.clear();
            (val || '').split(/\s+/).filter(Boolean).forEach(c => classes.add(c));
          },
          appendChild: (child: any) => {
            children.push(child);
            return child;
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
            (listeners['click'] || []).forEach(fn => fn({ type: 'click' }));
          },
          querySelector: (sel: string) => {
            const findIn = (node: any): any => {
              if (sel === '.grid-card-thumb' && node.classList?.contains('grid-card-thumb')) return node;
              if (sel === '.grid-card-title' && node.classList?.contains('grid-card-title')) return node;
              if (sel === '.grid-card-badge' && node.classList?.contains('grid-card-badge')) return node;
              if (sel === '.media-video-duration' && node.classList?.contains('media-video-duration')) return node;
              if (sel === 'video' && node.tagName === 'VIDEO') return node;
              for (const ch of node.children || []) {
                const found = findIn(ch);
                if (found) return found;
              }
              return null;
            };
            return findIn(el);
          },
        };
        return el;
      },
    };
  });

  afterEach(() => {
    delete (globalThis as any).document;
  });

  test('buildGridCard creates image card with standard classes and properties', () => {
    let clicked = false;
    let dblClicked = false;

    const card = buildGridCard({
      id: 'img-1',
      title: 'Mountain Sunrise',
      thumbnail: { type: 'image', value: '/media/mountains.jpg' },
      isSelected: true,
      onClick: () => { clicked = true; },
      onDoubleClick: () => { dblClicked = true; },
    });

    expect(card.classList.contains('ui-grid-card')).toBe(true);
    expect(card.classList.contains('selected')).toBe(true);
    expect(card.dataset.id).toBe('img-1');
    expect(card.title).toBe('Mountain Sunrise');

    const thumb = card.querySelector('.grid-card-thumb') as HTMLElement;
    expect(thumb).toBeDefined();
    expect(thumb.style.backgroundImage).toContain('mountains.jpg');

    const titleEl = card.querySelector('.grid-card-title');
    expect(titleEl?.innerHTML).toBe('Mountain Sunrise');

    card.click();
    expect(clicked).toBe(true);

    card.dispatchEvent({ type: 'dblclick' });
    expect(dblClicked).toBe(true);
  });

  test('buildGridCard formats video cards with duration badge', () => {
    const card = buildGridCard({
      id: 'vid-1',
      title: 'Ocean Waves',
      thumbnail: { type: 'video', value: '/media/ocean.mp4', durationSec: 125 },
    });

    const video = card.querySelector('video') as HTMLVideoElement;
    expect(video).toBeDefined();
    expect(video.src).toContain('ocean.mp4');

    const durBadge = card.querySelector('.media-video-duration');
    expect(durBadge?.textContent).toBe('2:05');
  });

  test('buildGridCard adds status badge when provided', () => {
    const card = buildGridCard({
      id: 'theme-1',
      title: 'Default Dark',
      thumbnail: { type: 'css', value: '#121212' },
      badge: { text: '⭐ DEFAULT', color: '#ffd700' },
    });

    const badge = card.querySelector('.grid-card-badge') as HTMLElement;
    expect(badge).toBeDefined();
    expect(badge.textContent).toBe('⭐ DEFAULT');
    expect(badge.style.color).toBe('#ffd700');
  });

  test('GridSelectionController manages selection and arrow navigation', () => {
    const items = [
      { id: '1', name: 'Item 1' },
      { id: '2', name: 'Item 2' },
      { id: '3', name: 'Item 3' },
      { id: '4', name: 'Item 4' },
    ];

    let selectedItem: any = null;
    let activatedItem: any = null;

    const controller = new GridSelectionController({
      getItemId: it => it.id,
      onSelect: it => { selectedItem = it; },
      onActivate: it => { activatedItem = it; },
    });

    controller.setItems(items, true);
    expect(controller.getSelectedId()).toBe('1');
    expect(selectedItem?.id).toBe('1');

    // Arrow Right moves to item 2
    const handledRight = controller.handleKeyDown({ key: 'ArrowRight' } as KeyboardEvent, 2);
    expect(handledRight).toBe(true);
    expect(controller.getSelectedId()).toBe('2');

    // Enter activates item 2
    const handledEnter = controller.handleKeyDown({ key: 'Enter' } as KeyboardEvent, 2);
    expect(handledEnter).toBe(true);
    expect(activatedItem?.id).toBe('2');
  });
});
