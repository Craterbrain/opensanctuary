import { describe, test, expect } from 'bun:test';
import { splitTextToSlides, stripChordsAndAnnotations, BulkSplitRule } from '../src/editor/bulk_parser';
import { EditorSlide } from '../src/editor/types';

describe('Bulk Paste UI Integration & Workflow', () => {

  test('stripChordsAndAnnotations preserves indentation and handles multiline lyrics', () => {
    const rawInput = `
  [C]Line with leading space
  [G]Second line
    `.trim();

    const cleaned = stripChordsAndAnnotations(rawInput);
    expect(cleaned).toBe('Line with leading space\nSecond line');
  });

  test('splitTextToSlides with append logic combines slides seamlessly', () => {
    const existingSlides: EditorSlide[] = [
      {
        id: 'slide-1',
        text: 'Pre-existing Slide 1',
        tag: 'V1',
        label: 'Verse 1',
        elements: []
      }
    ];

    const bulkText = `
Chorus
New chorus line 1
New chorus line 2

Bridge
New bridge line
    `.trim();

    const { slides: parsedSlides, metadata } = splitTextToSlides(bulkText, {
      splitRule: 'paragraphs',
      editorType: 'song',
      extractMetadata: true
    });

    expect(parsedSlides.length).toBe(2);

    const newSlides: EditorSlide[] = parsedSlides.map((ps, idx) => ({
      id: `slide-new-${idx}`,
      text: ps.text,
      tag: ps.tag,
      label: ps.label,
      elements: []
    }));

    const appended = [...existingSlides, ...newSlides];
    expect(appended.length).toBe(3);
    expect(appended[0].id).toBe('slide-1');
    expect(appended[0].tag).toBe('V1');
    expect(appended[1].tag).toBe('C');
    expect(appended[1].text).toContain('New chorus line 1');
    expect(appended[2].tag).toBe('B');
  });

  test('handles 2-line split mode with song arrangement tags', () => {
    const input = `
Verse 1
Line 1
Line 2
Line 3
Line 4
    `.trim();

    const res = splitTextToSlides(input, { splitRule: 'lines-2' });
    expect(res.slides.length).toBe(2);
    expect(res.slides[0].tag).toBe('V1');
    expect(res.slides[0].label).toBe('Verse 1 (1/2)');
    expect(res.slides[0].text).toBe('Line 1\nLine 2');
    expect(res.slides[1].tag).toBe('V1');
    expect(res.slides[1].label).toBe('Verse 1 (2/2)');
    expect(res.slides[1].text).toBe('Line 3\nLine 4');
  });

  test('handles 4-line split mode', () => {
    const input = `
Chorus
1
2
3
4
5
6
7
8
    `.trim();

    const res = splitTextToSlides(input, { splitRule: 'lines-4' });
    expect(res.slides.length).toBe(2);
    expect(res.slides[0].tag).toBe('C');
    expect(res.slides[0].label).toBe('Chorus (1/2)');
    expect(res.slides[1].tag).toBe('C');
    expect(res.slides[1].label).toBe('Chorus (2/2)');
  });

  test('auto-extracts CCLI metadata and separates it from slide presentation elements', () => {
    const input = `
Verse 1
When peace like a river attendeth my way

Words: Horatio G. Spafford
Music: Philip P. Bliss
CCLI Song # 25376
Copyright Public Domain
    `.trim();

    const { slides, metadata } = splitTextToSlides(input, { extractMetadata: true });
    expect(slides.length).toBe(1);
    expect(slides[0].tag).toBe('V1');
    expect(slides[0].text).toBe('When peace like a river attendeth my way');
    expect(metadata.author).toBe('Horatio G. Spafford, Philip P. Bliss');
    expect(metadata.ccli_number).toBe('25376');
    expect(metadata.copyright).toContain('Public Domain');
  });
});
