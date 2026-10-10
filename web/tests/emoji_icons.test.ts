import { describe, expect, test } from 'bun:test';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import emojiMap from '../icons/emoji-map.json';
import { ICON_SETS } from '../src/core/icon_sets.ts';
import { splitEmojiText } from '../src/core/emoji_icons.ts';

const entries = emojiMap as { glyph: string; icon: string | null; note?: string }[];
const iconFor = (glyph: string) => entries.find(e => e.glyph === glyph)!.icon;

describe('emoji -> icon text splitting', () => {
  test('text without emoji is returned unchanged', () => {
    expect(splitEmojiText('Save Schedule')).toEqual(['Save Schedule']);
    expect(splitEmojiText('')).toEqual([]);
  });

  test('a leading emoji becomes an icon followed by the rest of the text', () => {
    expect(splitEmojiText('🔍 Search')).toEqual([{ icon: iconFor('🔍')! }, ' Search']);
  });

  test('several emoji in one string, and the emoji variation selector is absorbed', () => {
    const trash = iconFor('🗑')!;
    expect(splitEmojiText('a 🗑️ b 🗑 c')).toEqual(['a ', { icon: trash }, ' b ', { icon: trash }, ' c']);
  });

  test('explicit {icon:name} tokens render that icon', () => {
    expect(splitEmojiText('Guide {icon:help-guide}!')).toEqual(['Guide ', { icon: 'help-guide' }, '!']);
  });

  test('glyphs deliberately left as text are not converted', () => {
    const textual = entries.filter(e => e.icon === null);
    expect(textual.length).toBeGreaterThan(0);
    for (const e of textual) expect(splitEmojiText(`x ${e.glyph} y`)).toEqual([`x ${e.glyph} y`]);
  });
});

describe('emoji-map.json', () => {
  test('every glyph appears once and is documented when left as text', () => {
    const glyphs = entries.map(e => e.glyph);
    expect(new Set(glyphs).size).toBe(glyphs.length);
    for (const e of entries) if (e.icon === null) expect((e.note ?? '').length).toBeGreaterThan(5);
  });

  test('every icon it names exists in all five sets', () => {
    const root = join(import.meta.dir, '..', 'icons');
    for (const e of entries) {
      if (!e.icon) continue;
      for (const set of ICON_SETS) expect(existsSync(join(root, set.id, `${e.icon}.svg`))).toBe(true);
    }
  });
});
