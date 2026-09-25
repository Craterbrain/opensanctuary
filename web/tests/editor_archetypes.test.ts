import { describe, test, expect } from 'bun:test';
import { applyArchetypeToSlide } from '../src/editor/toolbar';
import { EditorSlide } from '../src/editor/types';

describe('Layout Archetype Generation & Text Preservation Engine', () => {
  const baseSlide: EditorSlide = {
    id: 's-sample',
    text: 'Holy Holy Holy Lord God Almighty',
    elements: []
  };

  test('generates lyric archetype with autofit and centered text block', () => {
    const elements = applyArchetypeToSlide('lyric', baseSlide);
    expect(elements.length).toBe(1);
    expect(elements[0].type).toBe('TextBlock');

    const tb = elements[0] as any;
    expect(tb.block.runs[0].text).toContain('Holy Holy Holy');
    expect(tb.block.autofit).toBe(true);
    expect(tb.block.paragraph_style.align).toBe('center');

    // Invariant: inv.element.transform-normalized
    expect(tb.transform.x).toBeGreaterThanOrEqual(0.0);
    expect(tb.transform.x + tb.transform.w).toBeLessThanOrEqual(1.0);
    expect(tb.transform.y).toBeGreaterThanOrEqual(0.0);
    expect(tb.transform.y + tb.transform.h).toBeLessThanOrEqual(1.0);
  });

  test('generates title archetype with main title and subtitle blocks', () => {
    const elements = applyArchetypeToSlide('title', baseSlide);
    expect(elements.length).toBe(2);
    expect(elements[0].type).toBe('TextBlock');
    expect(elements[1].type).toBe('TextBlock');

    const titleEl = elements[0] as any;
    const subEl = elements[1] as any;
    expect(titleEl.block.runs[0].text).toContain('Holy Holy Holy');
    expect(subEl.block.runs[0].text).toContain('Subtitle');
  });

  test('generates title_body archetype with heading and bulleted body', () => {
    const elements = applyArchetypeToSlide('title_body', baseSlide);
    expect(elements.length).toBe(2);

    const bodyEl = elements[1] as any;
    expect(bodyEl.block.runs[0].text).toContain('Holy Holy Holy');
    expect(bodyEl.block.paragraph_style.bullet_kind).toBe('disc');
  });

  test('generates two_column archetype with two side-by-side text blocks', () => {
    const elements = applyArchetypeToSlide('two_column', baseSlide);
    expect(elements.length).toBe(2);

    const col1 = elements[0];
    const col2 = elements[1];

    expect(col1.transform.x).toBeLessThan(col2.transform.x);
    expect(col1.transform.x + col1.transform.w).toBeLessThanOrEqual(col2.transform.x);
  });

  test('generates scripture archetype with verse body and right-aligned citation', () => {
    const elements = applyArchetypeToSlide('scripture', baseSlide);
    expect(elements.length).toBe(2);

    const citationEl = elements[1] as any;
    expect(citationEl.block.paragraph_style.align).toBe('right');
    expect(citationEl.block.runs[0].text).toContain('John 3:16');
  });

  test('generates quote archetype with quote body and right-aligned attribution', () => {
    const elements = applyArchetypeToSlide('quote', baseSlide);
    expect(elements.length).toBe(2);

    const attrEl = elements[1] as any;
    expect(attrEl.block.paragraph_style.align).toBe('right');
    expect(attrEl.block.runs[0].text).toContain('Martin Luther King Jr.');
  });

  test('generates blank archetype with 0 elements', () => {
    const elements = applyArchetypeToSlide('blank', baseSlide);
    expect(elements.length).toBe(0);
  });

  test('preserves existing text when switching between different archetypes', () => {
    const slide1: EditorSlide = {
      id: 's-switch',
      text: 'Custom worship chorus lines',
      elements: []
    };

    // Apply lyric
    slide1.elements = applyArchetypeToSlide('lyric', slide1);
    expect((slide1.elements[0] as any).block.runs[0].text).toContain('Custom worship chorus lines');

    // Switch to two_column
    slide1.elements = applyArchetypeToSlide('two_column', slide1);
    expect((slide1.elements[0] as any).block.runs[0].text).toContain('Custom worship chorus lines');

    // Switch to quote
    slide1.elements = applyArchetypeToSlide('quote', slide1);
    expect((slide1.elements[0] as any).block.runs[0].text).toContain('Custom worship chorus lines');
  });
});
