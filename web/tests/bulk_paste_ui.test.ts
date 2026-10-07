import { describe, test, expect } from 'bun:test';
import { splitTextToSlides } from '../src/editor/bulk_parser';
import { EditorSlide } from '../src/editor/types';

describe('Bulk Paste UI Integration & Workflow', () => {

  // splitTextToSlides/stripChordsAndAnnotations's own split-rule and chord-
  // stripping behavior (2-line/4-line splitting, CCLI extraction) is covered
  // exhaustively at the function level in bulk_parser.test.ts -- this file
  // only covers what that one doesn't: the *caller's* append-merge workflow
  // (combining freshly parsed slides onto an existing slide list).
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

});
