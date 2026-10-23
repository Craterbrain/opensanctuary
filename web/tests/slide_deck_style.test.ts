import { describe, expect, test } from 'bun:test';
import { slideDeckStyle } from '../src/ui/slide_deck_view.ts';

describe('slide deck style (Preview / Live decks)', () => {
  test('songs, scriptures and presentations show rendered slide thumbnails in the grid view', () => {
    for (const t of ['song', 'scripture', 'presentation', 'Song', 'Scripture', 'Presentation']) {
      expect(slideDeckStyle(t, false)).toBe('thumbnails');
    }
  });

  test('everything else keeps the text cards', () => {
    for (const t of ['media', 'header', '', undefined, null]) {
      expect(slideDeckStyle(t as any, false)).toBe('cards');
    }
  });

  test('the list view always uses text cards (the override)', () => {
    for (const t of ['presentation', 'song', 'scripture', 'media']) expect(slideDeckStyle(t, true)).toBe('cards');
  });
});
