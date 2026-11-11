import { describe, test, expect } from 'bun:test';
import { previewBackgroundCss } from '../src/remote_client.ts';

describe('Phone remote preview card background', () => {
  const item = (slideBg: string | null, itemBg: string | null = null) => ({ background: itemBg, theme_name: null, arrangement: [], slides: [{ text: 'x', background: slideBg }] });

  test('a stored image path is used as it is, not encoded again under /media/', () => {
    expect(previewBackgroundCss(item('/media/images/sun rise.jpg'), 0)).toBe('url("/media/images/sun%20rise.jpg") center/cover no-repeat #000000');
  });

  test("the item's and the global background apply when the slide has none", () => {
    expect(previewBackgroundCss(item(null, '#112233'), 0)).toBe('#112233');
    expect(previewBackgroundCss(item(null), 0, 'linear-gradient(90deg, #000, #fff)')).toBe('linear-gradient(90deg, #000, #fff)');
  });

  test('a video background has no still to show', () => {
    expect(previewBackgroundCss(item('/media/videos/loop.mp4'), 0)).toBe('#000000');
  });
});
