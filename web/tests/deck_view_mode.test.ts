import { beforeEach, describe, expect, test } from 'bun:test';
import { defaultDeckMode, deckModeFor, itemKind, rememberDeckMode, resetDeckModesForTests } from '../src/ui/deck_view_mode.ts';

describe('Preview / Live deck view mode', () => {
  beforeEach(() => resetDeckModesForTests());

  test('songs and scriptures are text and open in the list; everything else is visual and opens in the grid', () => {
    for (const t of ['song', 'scripture', 'Song', 'SCRIPTURE']) {
      expect(itemKind(t)).toBe('text');
      expect(defaultDeckMode(t)).toBe('list');
      expect(deckModeFor('preview', t)).toBe('list');
    }
    for (const t of ['presentation', 'media', 'header', '', undefined, null]) {
      expect(itemKind(t as any)).toBe('visual');
      expect(defaultDeckMode(t as any)).toBe('matrix');
      expect(deckModeFor('live', t as any)).toBe('matrix');
    }
  });

  test('a choice is remembered for that kind of item on that deck only', () => {
    rememberDeckMode('preview', 'song', 'matrix');
    expect(deckModeFor('preview', 'song')).toBe('matrix');
    expect(deckModeFor('preview', 'scripture')).toBe('matrix'); // another text item opens the same way
    expect(deckModeFor('live', 'song')).toBe('list');           // the other deck is independent
    expect(deckModeFor('preview', 'presentation')).toBe('matrix'); // visual items keep their own default

    rememberDeckMode('preview', 'presentation', 'list');
    expect(deckModeFor('preview', 'media')).toBe('list');        // media is visual too
    expect(deckModeFor('preview', 'song')).toBe('matrix');       // text choice untouched
  });

  test('choosing the default again just sets it', () => {
    rememberDeckMode('live', 'scripture', 'list');
    expect(deckModeFor('live', 'song')).toBe('list');
  });
});
