import { describe, test, expect, beforeEach, beforeAll } from 'bun:test';
import { SlideEditor } from '../src/editor/slide_editor';
import { formatBulletLines } from '../src/editor/text_block';
import { EditorSlide, SlideBackground, SOLID_PALETTE_PRESETS, GRADIENT_PRESETS, FONT_FAMILY_GROUPS } from '../src/editor/types';

beforeAll(() => {
  const existingRibbon = (globalThis as any).document?.getElementById?.('slide-editor-ribbon');
  if (!existingRibbon) {
    const createMockNode = (tag: string = 'div') => {
      const childCache = new Map<string, any>();
      const classes = new Set<string>();
      const el: any = {
        tagName: tag.toUpperCase(),
        classList: {
          add: (...cs: string[]) => cs.forEach(c => classes.add(c)),
          remove: (...cs: string[]) => cs.forEach(c => classes.delete(c)),
          contains: (c: string) => classes.has(c),
          has: (c: string) => classes.has(c),
          toggle: (c: string, force?: boolean) => {
            const val = force !== undefined ? force : !classes.has(c);
            if (val) classes.add(c); else classes.delete(c);
            return val;
          }
        },
        style: {},
        children: [] as any[],
        appendChild: (child: any) => {
          el.children.push(child);
          return child;
        },
        removeChild: (child: any) => child,
        remove: () => {},
        // Real querySelector only finds real descendants; this mock DOM
        // never actually builds the ribbon's markup, so it auto-vivifies a
        // generic node per selector instead (cached per parent) — enough for
        // EditorToolbar's element lookups (src/editor/toolbar.ts's req()) to
        // succeed without this test needing to know every button id it asks
        // for. Nothing here asserts on toolbar DOM structure/content.
        querySelector: (sel: string) => {
          if (!childCache.has(sel)) childCache.set(sel, createMockNode());
          return childCache.get(sel);
        },
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
        removeEventListener: () => {},
        setAttribute: () => {},
        getAttribute: () => null,
        dataset: {},
        set innerHTML(val: string) {
          if (!val) el.children.length = 0;
        },
        get innerHTML() { return ''; },
        textContent: ''
      };
      return el;
    };

    (globalThis as any).document = {
      createElement: createMockNode,
      // Real documents always have this. slide_editor.ts uses it to find
      // #studio-sidebar-header (a real page element that legitimately
      // doesn't exist in this synthetic DOM, same as a real browser would
      // return for an absent id) and #slide-editor-ribbon (which DOES exist
      // on the real page — SlideEditor throws if it's missing — so this mock
      // has to return a working stand-in, not null).
      getElementById: (id: string) => (id === 'slide-editor-ribbon' ? createMockNode() : null),
      body: createMockNode('body'),
      addEventListener: () => {},
      removeEventListener: () => {},
      activeElement: null
    };
    (globalThis as any).window = globalThis;
    (globalThis as any).window.addEventListener = () => {};
    (globalThis as any).window.removeEventListener = () => {};
    (globalThis as any).requestAnimationFrame = (cb: Function) => setTimeout(cb, 0);
  }
});

describe('Slide Editor Text Tools & Background Picker', () => {
  let container: HTMLElement;
  let editor: SlideEditor;

  const sampleSlide: EditorSlide = {
    id: 'slide-test-1',
    text: 'Amazing grace how sweet the sound',
    elements: [
      {
        type: 'TextBlock',
        id: 'tb-lyrics',
        transform: { x: 0.1, y: 0.1, w: 0.8, h: 0.8, rotation_deg: 0, z_index: 1, locked: false, opacity: 1.0 },
        block: {
          runs: [
            {
              text: 'Amazing grace how sweet the sound',
              font_family: 'Roboto',
              font_size_pt: 36,
              bold: false,
              italic: false,
              underline: false,
              color: '#ffffff'
            }
          ],
          paragraph_style: { align: 'center', line_height: 1.25 },
          autofit: true
        }
      }
    ],
    background: '#000000',
    background_v2: { kind: 'Solid', data: '#000000' }
  };

  beforeEach(() => {
    container = document.createElement('div');
    document.body.appendChild(container);
    editor = new SlideEditor({ container });
    editor.open([sampleSlide], 0);
  });

  test('presets and font families are well-defined and valid', () => {
    expect(SOLID_PALETTE_PRESETS.length).toBeGreaterThanOrEqual(8);
    for (const p of SOLID_PALETTE_PRESETS) {
      expect(p.name).toBeDefined();
      expect(p.color).toMatch(/^#[0-9a-fA-F]{6}$/);
    }

    expect(GRADIENT_PRESETS.length).toBeGreaterThanOrEqual(5);
    for (const g of GRADIENT_PRESETS) {
      expect(g.name).toBeDefined();
      expect(g.background.kind).toBe('Gradient');
      expect(g.preview).toContain('gradient');
    }

    const availableFontFamilies = FONT_FAMILY_GROUPS.flatMap(g => g.fonts);
    expect(availableFontFamilies).toContain('Roboto');
    expect(availableFontFamilies).toContain('Arial');
    expect(availableFontFamilies).toContain('Georgia');
  });

  test('applyTextStyle updates typography properties on targeted text block', () => {
    const cur = editor.getActiveSlide();
    const tb = cur.elements[0] as any;
    expect(tb.block.runs[0].font_family).toBe('Roboto');
    expect(tb.block.runs[0].bold).toBe(false);

    // Select text block and apply formatting
    editor.canvas.selection.select(tb.id);

    editor.applyTextStyle({
      font_family: 'Montserrat',
      font_size_pt: 48,
      bold: true,
      italic: true,
      underline: true,
      color: '#fbbf24',
      align: 'left',
      line_height: 1.5,
      autofit: false
    });

    const updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.runs[0].font_family).toBe('Montserrat');
    expect(updated.block.runs[0].font_size_pt).toBe(48);
    expect(updated.block.runs[0].bold).toBe(true);
    expect(updated.block.runs[0].italic).toBe(true);
    expect(updated.block.runs[0].underline).toBe(true);
    expect(updated.block.runs[0].color).toBe('#fbbf24');
    expect(updated.block.paragraph_style.align).toBe('left');
    expect(updated.block.paragraph_style.line_height).toBe(1.5);
    expect(updated.block.autofit).toBe(false);
  });

  test('applyTextStyle falls back to lone text block when selection is empty', () => {
    editor.canvas.selection.deselectAll();

    editor.applyTextStyle({
      font_family: 'Georgia',
      bold: true,
      align: 'right'
    });

    const updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.runs[0].font_family).toBe('Georgia');
    expect(updated.block.runs[0].bold).toBe(true);
    expect(updated.block.paragraph_style.align).toBe('right');
  });

  test('updateSlideBackground applies solid, gradient, image, and video backgrounds', () => {
    // 1. Solid Color
    editor.updateSlideBackground({ kind: 'Solid', data: '#1e1b4b' });
    let cur = editor.getActiveSlide();
    expect(cur.background_v2?.kind).toBe('Solid');
    expect(cur.background).toBe('#1e1b4b');

    // 2. Gradient
    const gradientBg: SlideBackground = {
      kind: 'Gradient',
      data: {
        kind: 'linear',
        angle_deg: 135,
        stops: [
          { offset: 0, color: '#0f2027' },
          { offset: 1, color: '#2c5364' }
        ]
      }
    };
    editor.updateSlideBackground(gradientBg);
    cur = editor.getActiveSlide();
    expect(cur.background_v2?.kind).toBe('Gradient');
    expect(cur.background).toContain('linear-gradient');

    // 3. Image
    editor.updateSlideBackground({ kind: 'Image', data: { file_path: 'https://example.com/bg.jpg', opacity: 0.8 } });
    cur = editor.getActiveSlide();
    expect(cur.background_v2?.kind).toBe('Image');
    expect(cur.background).toBe('url("https://example.com/bg.jpg")');

    // 4. Video
    editor.updateSlideBackground({ kind: 'Video', data: { file_path: '/media/video.mp4', loop_playback: true, is_muted: true } });
    cur = editor.getActiveSlide();
    expect(cur.background_v2?.kind).toBe('Video');
    expect(cur.background).toBe('/media/video.mp4');

    // 5. Clear / Reset
    editor.updateSlideBackground(null);
    cur = editor.getActiveSlide();
    expect(cur.background_v2).toBeUndefined();
    expect(cur.background).toBeUndefined();
  });

  test('text formatting and background updates participate in undo and redo history', () => {
    expect(editor.history.canUndo()).toBe(false);

    // Change background
    editor.updateSlideBackground({ kind: 'Solid', data: '#3b0764' });
    expect(editor.history.canUndo()).toBe(true);
    expect(editor.getActiveSlide().background).toBe('#3b0764');

    // Change text style
    editor.applyTextStyle({ bold: true });
    expect((editor.getActiveSlide().elements[0] as any).block.runs[0].bold).toBe(true);

    // Undo text style change
    editor.undo();
    expect((editor.getActiveSlide().elements[0] as any).block.runs[0].bold).toBe(false);
    expect(editor.getActiveSlide().background).toBe('#3b0764');

    // Undo background change
    editor.undo();
    expect(editor.getActiveSlide().background).toBe('#000000');

    // Redo background change
    editor.redo();
    expect(editor.getActiveSlide().background).toBe('#3b0764');

    // Redo text style change
    editor.redo();
    expect((editor.getActiveSlide().elements[0] as any).block.runs[0].bold).toBe(true);
  });

  test('applyTextStyle supports strike, subscript, superscript, bullets, and indent', () => {
    const cur = editor.getActiveSlide();
    const tb = cur.elements[0] as any;
    editor.canvas.selection.select(tb.id);

    // 1. Strikethrough and Subscript
    editor.applyTextStyle({ strike: true, baseline_shift: 'sub' });
    let updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.runs[0].strike).toBe(true);
    expect(updated.block.runs[0].baseline_shift).toBe('sub');

    // 2. Superscript replaces Subscript
    editor.applyTextStyle({ baseline_shift: 'super' });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.runs[0].baseline_shift).toBe('super');

    // 3. Bullet list (disc)
    tb.block.runs[0].text = 'Line one\nLine two\nLine three';
    editor.applyTextStyle({ bullet_kind: 'disc' });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.bullet_kind).toBe('disc');
    expect(updated.block.paragraph_style.align).toBe('left');
    expect(updated.block.runs[0].text).toBe('• Line one\n• Line two\n• Line three');

    // 4. Numbered list (decimal) converts from disc
    editor.applyTextStyle({ bullet_kind: 'decimal' });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.bullet_kind).toBe('decimal');
    expect(updated.block.runs[0].text).toBe('1. Line one\n2. Line two\n3. Line three');

    // 5. None removes bullets
    editor.applyTextStyle({ bullet_kind: 'none' });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.bullet_kind).toBe('none');
    expect(updated.block.runs[0].text).toBe('Line one\nLine two\nLine three');

    // 6. Indent increment and decrement
    expect(updated.block.paragraph_style.indent_level || 0).toBe(0);
    editor.applyTextStyle({ indent_delta: 1 });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.indent_level).toBe(1);

    editor.applyTextStyle({ indent_delta: 1 });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.indent_level).toBe(2);

    editor.applyTextStyle({ indent_delta: -1 });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.indent_level).toBe(1);

    // Absolute indent level clamped between 0 and 8
    editor.applyTextStyle({ indent_level: 99 });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.indent_level).toBe(8);

    editor.applyTextStyle({ indent_level: -5 });
    updated = editor.getActiveSlide().elements[0] as any;
    expect(updated.block.paragraph_style.indent_level).toBe(0);
  });

  test('formatBulletLines preserves blank lines and cleanly formats markers', () => {
    const raw = 'First item\n\nSecond item\nThird item';
    const disc = formatBulletLines(raw, 'disc');
    expect(disc).toBe('• First item\n\n• Second item\n• Third item');

    const decimal = formatBulletLines(disc, 'decimal');
    expect(decimal).toBe('1. First item\n\n2. Second item\n3. Third item');

    const stripped = formatBulletLines(decimal, 'none');
    expect(stripped).toBe('First item\n\nSecond item\nThird item');
  });

  test('SlideEditor setSlideTag and setSlideDuration update slide and participate in undo/redo', () => {
    // Initial slide has no tag or duration
    expect(editor.getActiveSlide().tag).toBeUndefined();
    expect(editor.getActiveSlide().duration_seconds).toBeUndefined();

    // 1. Set Tag to V1
    editor.setSlideTag(0, 'V1');
    expect(editor.getActiveSlide().tag).toBe('V1');
    expect(editor.getActiveSlide().label).toBe('Verse 1');

    // 2. Set Duration to 5s
    editor.setSlideDuration(0, 5);
    expect(editor.getActiveSlide().duration_seconds).toBe(5);

    // 3. Set Tag to Chorus (C)
    editor.setSlideTag(0, 'C');
    expect(editor.getActiveSlide().tag).toBe('C');
    expect(editor.getActiveSlide().label).toBe('Chorus');

    // 4. Undo tag change
    expect(editor.history.canUndo()).toBe(true);
    editor.undo();
    expect(editor.getActiveSlide().tag).toBe('V1');
    expect(editor.getActiveSlide().duration_seconds).toBe(5);

    // 5. Undo duration change
    editor.undo();
    expect(editor.getActiveSlide().duration_seconds).toBeUndefined();

    // 6. Clear tag
    editor.setSlideTag(0, null);
    expect(editor.getActiveSlide().tag).toBeUndefined();
  });

  test('SlideEditor hides tagger when set to presentation or scripture mode', () => {
    // Default open mode is song
    expect(editor.isSongMode()).toBe(true);
    expect(editor.getMode()).toBe('song');
    expect(editor.filmstrip.isSongMode()).toBe(true);

    // Switch to presentation mode
    editor.setMode('presentation');
    expect(editor.isSongMode()).toBe(false);
    expect(editor.getMode()).toBe('presentation');
    expect(editor.filmstrip.isSongMode()).toBe(false);

    // Switch to scripture mode
    editor.setMode('scripture');
    expect(editor.isSongMode()).toBe(false);
    expect(editor.getMode()).toBe('scripture');
    expect(editor.filmstrip.isSongMode()).toBe(false);

    // Switch back to song mode
    editor.setMode('song');
    expect(editor.isSongMode()).toBe(true);
    expect(editor.getMode()).toBe('song');
    expect(editor.filmstrip.isSongMode()).toBe(true);
  });
});
