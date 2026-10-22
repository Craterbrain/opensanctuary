// Loaded into a blank headless page by render_pattern_thumbs.ts: the real pattern code, two hooks.
import { BACKGROUND_PATTERNS, patternLoopSec, patternRevision } from "../src/core/background_patterns";
import { renderPatternFrame } from "../src/core/pattern_render";

(window as any).thumbPatterns = () =>
  BACKGROUND_PATTERNS.filter(p => p.animated && patternLoopSec(p.key) > 0)
    .map(p => ({ key: p.key, rev: patternRevision(p.key), loopSec: patternLoopSec(p.key) }));

(window as any).thumbFrame = async (key: string, t: number, width: number, height: number): Promise<string> => {
  const blob = await renderPatternFrame(key, t, { width, height });
  const bytes = new Uint8Array(await blob.arrayBuffer());
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
};
