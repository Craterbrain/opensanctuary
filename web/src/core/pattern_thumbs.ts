/**
 * Pre-rendered previews of the animated backgrounds, for every place that shows many of them at
 * once at thumbnail size: the background pickers, the theme cards, and the slide thumbnails in the
 * Preview/Live decks and the editor's filmstrip.
 *
 * Drawing each pattern live (layered SVG and CSS animations) is fine for the one full-size slide,
 * but a grid of them kept the console's CPU and GPU flat out. Each preview is instead a small
 * looping WebP in web/pattern-thumbs/, made by web/scripts/render_pattern_thumbs.ts from the same
 * frame-drawing code the live clips use. manifest.json records the patternRevision each one was
 * made from; a preview older than its pattern isn't used (the live pattern is shown instead), and
 * tests/pattern_thumbs.test.ts fails until it is rendered again.
 */
import manifest from "../../pattern-thumbs/manifest.json";
import { patternRevision } from "./background_patterns";
import { applyResolvedBackground, parseAnimatedPatternMarker } from "./presentation_helpers";

const thumbs: Record<string, { rev: string; file: string }> = manifest;

/** URL of the pattern's preview, or null when there isn't an up-to-date one. */
export function patternThumbUrl(key: string): string | null {
  const t = thumbs[key];
  return t && t.rev === patternRevision(key) ? `/pattern-thumbs/${t.file}` : null;
}

/**
 * `applyResolvedBackground` for a thumbnail: an animated pattern shows its pre-rendered preview
 * (a plain image, scaled to cover), anything else is applied exactly as `applyResolvedBackground`
 * would.
 */
export function applyBackgroundThumbnail(el: HTMLElement, bg: string | null | undefined, ...rest: Parameters<typeof applyResolvedBackground> extends [any, any, ...infer R] ? R : never): void {
  const key = parseAnimatedPatternMarker(bg);
  const url = key ? patternThumbUrl(key) : null;
  if (!url) {
    applyResolvedBackground(el, bg, ...rest);
    return;
  }
  el.style.animation = "";
  el.style.background = `#000 url("${url}") center / cover no-repeat`;
}
