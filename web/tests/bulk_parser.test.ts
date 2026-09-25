import { describe, test, expect } from 'bun:test';
import {
  isChordLine,
  isCueLine,
  stripChordsAndAnnotations,
  extractSongMetadata,
  detectSectionHeader,
  splitTextToSlides
} from '../src/editor/bulk_parser';

describe('Bulk Text & Lyric Parser Engine', () => {

  describe('Chord & Performance Cue Detection', () => {
    test('identifies standalone chord lines correctly', () => {
      expect(isChordLine('G   D/F#   Em7   Cadd9')).toBe(true);
      expect(isChordLine('| C | G | Am7 | F |')).toBe(true);
      expect(isChordLine('A#m   Bbsus4   F#m7b5')).toBe(true);
      expect(isChordLine('D - G - A - D')).toBe(true);
      expect(isChordLine('')).toBe(false);
      expect(isChordLine('Bless the Lord O my soul')).toBe(false);
      expect(isChordLine('Great is Thy Faithfulness')).toBe(false);
      expect(isChordLine('Verse 1')).toBe(false);
    });

    test('identifies performance cues and musical annotations', () => {
      expect(isCueLine('(Repeat 2x)')).toBe(true);
      expect(isCueLine('(repeat chorus)')).toBe(true);
      expect(isCueLine('(2x)')).toBe(true);
      expect(isCueLine('[Instrumental]')).toBe(true);
      expect(isCueLine('(Guitar Solo)')).toBe(true);
      expect(isCueLine('Capo 3')).toBe(true);
      expect(isCueLine('BPM: 128')).toBe(true);
      expect(isCueLine('Key: G')).toBe(true);
      expect(isCueLine('Key: Bb minor')).toBe(true);
      expect(isCueLine('Sing like never before')).toBe(false);
    });

    test('strips inline chord brackets, standalone chord lines, and cues', () => {
      const dirtyLyrics = `
Capo 2
Key: G

Verse 1
[G]Bless the [D/F#]Lord O my [Em7]soul
[C]O [G]my [D]soul
(Repeat 2x)

G   D/F#   Em7   Cadd9

Chorus
[C]Sing like [G]never be[D]fore
[Em7]O my [C]soul
[Instrumental]
      `.trim();

      const cleaned = stripChordsAndAnnotations(dirtyLyrics);
      expect(cleaned).not.toContain('Capo');
      expect(cleaned).not.toContain('Key: G');
      expect(cleaned).not.toContain('[G]');
      expect(cleaned).not.toContain('[D/F#]');
      expect(cleaned).not.toContain('G   D/F#');
      expect(cleaned).not.toContain('(Repeat 2x)');
      expect(cleaned).not.toContain('[Instrumental]');
      expect(cleaned).toContain('Verse 1\nBless the Lord O my soul\nO my soul');
      expect(cleaned).toContain('Chorus\nSing like never before\nO my soul');
    });
  });

  describe('Song Metadata & CCLI Extraction', () => {
    test('extracts author, CCLI number, and copyright from lyrics text', () => {
      const songText = `
Verse 1
Amazing grace how sweet the sound
That saved a wretch like me

Words & Music by John Newton
CCLI Song # 27154
© Public Domain
      `.trim();

      const { cleanedText, metadata } = extractSongMetadata(songText);

      expect(metadata.author).toBe('John Newton');
      expect(metadata.ccli_number).toBe('27154');
      expect(metadata.copyright).toContain('Public Domain');

      // The lyrics should be clean of trailing metadata lines
      expect(cleanedText).toContain('Amazing grace how sweet the sound');
      expect(cleanedText).not.toContain('Words & Music by');
      expect(cleanedText).not.toContain('CCLI Song #');
      expect(cleanedText).not.toContain('Public Domain');
    });
  });

  describe('Section Header Detection', () => {
    test('detects verses and maps to canonical tags', () => {
      expect(detectSectionHeader('Verse 1')).toEqual({ tag: 'V1', label: 'Verse 1', isHeader: true });
      expect(detectSectionHeader('[Verse 2]')).toEqual({ tag: 'V2', label: 'Verse 2', isHeader: true });
      expect(detectSectionHeader('Verse 3:')).toEqual({ tag: 'V3', label: 'Verse 3', isHeader: true });
      expect(detectSectionHeader('V4')).toEqual({ tag: 'V4', label: 'Verse 4', isHeader: true });
    });

    test('detects choruses and maps to canonical tags', () => {
      expect(detectSectionHeader('Chorus')).toEqual({ tag: 'C', label: 'Chorus', isHeader: true });
      expect(detectSectionHeader('Chorus 1')).toEqual({ tag: 'C', label: 'Chorus 1', isHeader: true });
      expect(detectSectionHeader('[Chorus 2]')).toEqual({ tag: 'C2', label: 'Chorus 2', isHeader: true });
      expect(detectSectionHeader('C2')).toEqual({ tag: 'C2', label: 'Chorus 2', isHeader: true });
    });

    test('detects bridge, pre-chorus, intro, ending, tag, and slides', () => {
      expect(detectSectionHeader('Bridge')).toEqual({ tag: 'B', label: 'Bridge', isHeader: true });
      expect(detectSectionHeader('Bridge 2')).toEqual({ tag: 'B2', label: 'Bridge 2', isHeader: true });
      expect(detectSectionHeader('Pre-Chorus')).toEqual({ tag: 'P', label: 'Pre-Chorus', isHeader: true });
      expect(detectSectionHeader('[PreChorus 2]')).toEqual({ tag: 'P2', label: 'Pre-Chorus 2', isHeader: true });
      expect(detectSectionHeader('Intro')).toEqual({ tag: 'I', label: 'Intro', isHeader: true });
      expect(detectSectionHeader('Ending')).toEqual({ tag: 'E', label: 'Ending', isHeader: true });
      expect(detectSectionHeader('Outro')).toEqual({ tag: 'E', label: 'Ending', isHeader: true });
      expect(detectSectionHeader('Tag')).toEqual({ tag: 'Tag', label: 'Tag', isHeader: true });
      expect(detectSectionHeader('Slide 3')).toEqual({ tag: 'S3', label: 'Slide 3', isHeader: true });
      expect(detectSectionHeader('Point 1')).toEqual({ tag: 'S1', label: 'Point 1', isHeader: true });
      expect(detectSectionHeader('Random lyric text')).toBeNull();
    });
  });

  describe('Multi-Strategy Text Splitting', () => {
    const rawSong = `
Verse 1
Bless the Lord O my soul
O my soul
Worship His holy name

Chorus
Sing like never before
O my soul
I'll worship Your holy name

Bridge
Ten thousand reasons for my heart to find
    `.trim();

    test('splits by paragraphs (blank lines) with section tags', () => {
      const result = splitTextToSlides(rawSong, { splitRule: 'paragraphs' });
      expect(result.slides.length).toBe(3);

      expect(result.slides[0].tag).toBe('V1');
      expect(result.slides[0].label).toBe('Verse 1');
      expect(result.slides[0].text).toBe('Bless the Lord O my soul\nO my soul\nWorship His holy name');

      expect(result.slides[1].tag).toBe('C');
      expect(result.slides[1].label).toBe('Chorus');
      expect(result.slides[1].text).toBe('Sing like never before\nO my soul\nI\'ll worship Your holy name');

      expect(result.slides[2].tag).toBe('B');
      expect(result.slides[2].label).toBe('Bridge');
      expect(result.slides[2].text).toBe('Ten thousand reasons for my heart to find');
    });

    test('splits every 2 lines per slide while preserving section tag association', () => {
      const fourLineVerse = `
Verse 1
Line 1 of verse
Line 2 of verse
Line 3 of verse
Line 4 of verse
      `.trim();

      const result = splitTextToSlides(fourLineVerse, { splitRule: 'lines-2' });
      expect(result.slides.length).toBe(2);
      expect(result.slides[0].tag).toBe('V1');
      expect(result.slides[0].text).toBe('Line 1 of verse\nLine 2 of verse');
      expect(result.slides[0].label).toBe('Verse 1 (1/2)');

      expect(result.slides[1].tag).toBe('V1');
      expect(result.slides[1].text).toBe('Line 3 of verse\nLine 4 of verse');
      expect(result.slides[1].label).toBe('Verse 1 (2/2)');
    });

    test('splits every 4 lines per slide', () => {
      const eightLines = `
Chorus
Line 1
Line 2
Line 3
Line 4
Line 5
Line 6
Line 7
Line 8
      `.trim();

      const result = splitTextToSlides(eightLines, { splitRule: 'lines-4' });
      expect(result.slides.length).toBe(2);
      expect(result.slides[0].text.split('\n').length).toBe(4);
      expect(result.slides[1].text.split('\n').length).toBe(4);
      expect(result.slides[0].tag).toBe('C');
      expect(result.slides[1].tag).toBe('C');
    });

    test('splits by custom delimiter (e.g. "---")', () => {
      const delimited = `
Slide One Content
Here is the first slide
---
Slide Two Content
Here is the second slide
---
[Chorus]
Final Slide Content
      `.trim();

      const result = splitTextToSlides(delimited, { splitRule: 'custom-delimiter', customDelimiter: '---' });
      expect(result.slides.length).toBe(3);
      expect(result.slides[0].text).toContain('Slide One Content');
      expect(result.slides[1].text).toContain('Slide Two Content');
      expect(result.slides[2].tag).toBe('C');
      expect(result.slides[2].text).toBe('Final Slide Content');
    });

    test('splits strictly by section tag boundary even without blank lines', () => {
      const unspacedSong = `
Verse 1
Line 1
Line 2
Chorus
Line 3
Line 4
Bridge
Line 5
      `.trim();

      const result = splitTextToSlides(unspacedSong, { splitRule: 'tag-boundary' });
      expect(result.slides.length).toBe(3);
      expect(result.slides[0].tag).toBe('V1');
      expect(result.slides[0].text).toBe('Line 1\nLine 2');
      expect(result.slides[1].tag).toBe('C');
      expect(result.slides[1].text).toBe('Line 3\nLine 4');
      expect(result.slides[2].tag).toBe('B');
      expect(result.slides[2].text).toBe('Line 5');
    });

    test('handles bracketed and parenthesized section headers', () => {
      const song = `
[Verse 1]
First line

(Chorus)
Second line

[Bridge 1]
Third line
      `.trim();

      const result = splitTextToSlides(song);
      expect(result.slides.length).toBe(3);
      expect(result.slides[0].tag).toBe('V1');
      expect(result.slides[1].tag).toBe('C');
      expect(result.slides[2].tag).toBe('B');
    });

    test('splits Scripture text by verse numbers in scripture mode', () => {
      const scripture = '1 In the beginning was the Word, and the Word was with God. 2 He was with God in the beginning. 3 Through him all things were made.';
      const result = splitTextToSlides(scripture, { editorType: 'scripture' });

      expect(result.slides.length).toBe(3);
      expect(result.slides[0].tag).toBe('V1');
      expect(result.slides[0].text).toBe('1 In the beginning was the Word, and the Word was with God.');
      expect(result.slides[1].tag).toBe('V2');
      expect(result.slides[1].text).toBe('2 He was with God in the beginning.');
      expect(result.slides[2].tag).toBe('V3');
      expect(result.slides[2].text).toBe('3 Through him all things were made.');
    });

    test('handles empty and whitespace strings gracefully', () => {
      expect(splitTextToSlides('').slides).toEqual([]);
      expect(splitTextToSlides('   \n\n   ').slides).toEqual([]);
    });

    test('integrates chord stripping and metadata extraction together', () => {
      const fullDirtySong = `
Key: D
Capo 2

Verse 1
[D]The Lord is my [G]shepherd
[D]I shall not [A]want

Chorus
[G]He makes me lie [D]down in green pastures
(Repeat 2x)

Written by David
CCLI Song # 12345
      `.trim();

      const result = splitTextToSlides(fullDirtySong, { autoCleanChords: true, extractMetadata: true });
      expect(result.metadata.author).toBe('David');
      expect(result.metadata.ccli_number).toBe('12345');
      expect(result.slides.length).toBe(2);

      expect(result.slides[0].tag).toBe('V1');
      expect(result.slides[0].text).toBe('The Lord is my shepherd\nI shall not want');

      expect(result.slides[1].tag).toBe('C');
      expect(result.slides[1].text).toBe('He makes me lie down in green pastures');
    });
  });
});
