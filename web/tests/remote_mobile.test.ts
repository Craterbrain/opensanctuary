import { describe, test, expect } from 'bun:test';
import { swipeCommand, getItemTypeIcon, getTagClass } from '../src/core/remote_helpers.ts';

// The page itself (pairing gate, live navigation, revocation) is driven end to end in
// e2e_remote_mobile.test.ts; this covers the gesture and label rules it is built on.
describe('Mobile remote: swipes', () => {
  test('a quick horizontal swipe steps slides in every view', () => {
    for (const view of ['view-schedule', 'view-slides', 'view-prompter']) {
      expect(swipeCommand(-80, 5, 200, view)).toBe('NextSlide');
      expect(swipeCommand(95, -10, 180, view)).toBe('PrevSlide');
    }
  });

  test('a vertical swipe steps items only in the prompter; the other views scroll', () => {
    expect(swipeCommand(5, -90, 220, 'view-prompter')).toBe('NextItem');
    expect(swipeCommand(-8, 85, 210, 'view-prompter')).toBe('PrevItem');
    expect(swipeCommand(0, -100, 200, 'view-schedule')).toBeNull();
    expect(swipeCommand(0, 100, 200, 'view-slides')).toBeNull();
  });

  test('diagonal, short and slow swipes do nothing', () => {
    expect(swipeCommand(60, 60, 200, 'view-prompter')).toBeNull();
    expect(swipeCommand(20, 5, 100, 'view-prompter')).toBeNull();
    expect(swipeCommand(-100, 0, 450, 'view-prompter')).toBeNull();
  });
});

describe('Mobile remote: labels', () => {
  test('item types map to icons, case-insensitively, with a default', () => {
    expect(getItemTypeIcon('song')).toBe('🎵');
    expect(getItemTypeIcon('Song')).toBe('🎵');
    expect(getItemTypeIcon('scripture')).toBe('📖');
    expect(getItemTypeIcon('presentation')).toBe('📽️');
    expect(getItemTypeIcon('media')).toBe('🎬');
    expect(getItemTypeIcon('header')).toBe('🏷️');
    expect(getItemTypeIcon('custom')).toBe('📄');
  });

  test('section tags map to colour classes by their first letter', () => {
    expect(getTagClass('V1')).toBe('tag-v');
    expect(getTagClass('verse 2')).toBe('tag-v');
    expect(getTagClass('Chorus 2')).toBe('tag-c');
    expect(getTagClass('Bridge')).toBe('tag-b');
    expect(getTagClass('Pre-Chorus')).toBe('tag-p');
    expect(getTagClass('Ending')).toBe('tag-e');
    expect(getTagClass('Intro')).toBe('tag-i');
    expect(getTagClass('Tag')).toBe('');
    expect(getTagClass('')).toBe('');
  });
});
