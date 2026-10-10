import { describe, expect, test } from 'bun:test';
import { slideDeckStyle } from '../src/ui/slide_deck_view.ts';

describe('slide deck style (Preview / Live decks)', () => {
  test('presentations get rendered thumbnails in the grid view', () => {
    expect(slideDeckStyle('presentation', false)).toBe('thumbnails');
    expect(slideDeckStyle('Presentation', false)).toBe('thumbnails');
  });

  test('songs, scriptures and everything else keep the text cards', () => {
    for (const t of ['song', 'scripture', 'media', 'header', '', undefined, null]) {
      expect(slideDeckStyle(t as any, false)).toBe('cards');
    }
  });

  test('the list view always uses text cards (the override)', () => {
    expect(slideDeckStyle('presentation', true)).toBe('cards');
    expect(slideDeckStyle('song', true)).toBe('cards');
  });
});
