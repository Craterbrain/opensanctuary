/**
 * Canonical autoFitLyrics implementation.
 * Shared between live presentation screen and slide editor canvas text blocks.
 * Invariant: inv.text.autofit-single-source
 */

export interface AutoFitOptions {
  minFontSize?: number;
  maxFontSize?: number;
  maxSteps?: number;
  widthFactor?: number;
  heightFactor?: number;
  lineHeightScale?: boolean;
}

export function autoFitLyrics(
  containerEl: HTMLElement | null,
  targetEl: HTMLElement | null,
  options: AutoFitOptions = {}
): number {
  if (!containerEl || !targetEl) return 18;
  const text = targetEl.textContent ? targetEl.textContent.trim() : '';
  if (!text) return 18;

  const widthFactor = options.widthFactor ?? 0.92;
  const heightFactor = options.heightFactor ?? 0.86;
  const availWidth = containerEl.clientWidth * widthFactor;
  const availHeight = containerEl.clientHeight * heightFactor;
  if (availWidth <= 0 || availHeight <= 0) return 18;

  // `targetEl` is normally styled to 100% of `containerEl`'s width by its
  // caller (a wrapping text block needs to be that wide to lay out at all).
  // That means `targetEl.scrollWidth` reports ~100% of the container's width
  // regardless of font size — wrapped text doesn't get narrower as the font
  // shrinks, it just wraps onto more/fewer lines at essentially the same
  // overall width. Comparing that against `availWidth` (deliberately shrunk
  // by `widthFactor` to leave padding) can then never pass at ANY font size,
  // which forces the binary search below to always bottom out at
  // `minFontSize` — the exact "default text is always tiny" bug. Temporarily
  // constraining the element's own width to `availWidth` during measurement
  // makes wrapping happen within the padded box, so the width check only
  // fires for a genuine overflow (e.g. one unbreakable word wider than the
  // box) instead of unconditionally. Restored after measurement so the
  // element keeps tracking its container responsively (e.g. on resize).
  // A hair of slack below `availWidth` itself: the browser rounds computed
  // layout sizes, so setting the element's width to exactly `availWidth` and
  // then comparing `scrollWidth <= availWidth` fails almost every time on a
  // 1px rounding difference (e.g. width 794.88 set, scrollWidth reads back
  // 795) — which reintroduces the same "always bottoms out at minFontSize"
  // failure this whole block exists to fix, just from a rounding error
  // instead of a missing constraint.
  const constrainedWidth = Math.floor(availWidth) - 1;
  const prevWidth = targetEl.style.width;
  const prevMaxWidth = targetEl.style.maxWidth;
  targetEl.style.width = `${constrainedWidth}px`;
  targetEl.style.maxWidth = `${constrainedWidth}px`;

  const hasGrid = targetEl.querySelector('div') !== null;
  let minFont = options.minFontSize ?? 18;
  let maxFont = options.maxFontSize ?? (hasGrid
    ? Math.min(availHeight * 0.32, availWidth * 0.08, 90)
    : Math.min(availHeight * 0.40, availWidth * 0.11, 120));
  let bestFont = minFont;
  const maxSteps = options.maxSteps ?? 10;
  const lineHeightScale = options.lineHeightScale ?? true;

  for (let step = 0; step < maxSteps; step++) {
    const testFont = (minFont + maxFont) / 2;
    targetEl.style.fontSize = `${testFont}px`;
    if (lineHeightScale) {
      targetEl.style.lineHeight = `${Math.max(1.22, 1.38 - testFont / 340)}`;
    }

    if (targetEl.scrollHeight <= availHeight && targetEl.scrollWidth <= availWidth) {
      bestFont = testFont;
      lowFontStep(minFont, testFont);
      minFont = testFont + 0.5;
    } else {
      maxFont = testFont - 0.5;
    }
  }

  const clampedBest = Math.min(options.maxFontSize ?? 120, Math.max(options.minFontSize ?? 12, bestFont));
  targetEl.style.fontSize = `${clampedBest}px`;
  if (lineHeightScale) {
    targetEl.style.lineHeight = `${Math.max(1.22, 1.38 - clampedBest / 340)}`;
  }

  targetEl.style.width = prevWidth;
  targetEl.style.maxWidth = prevMaxWidth;

  return clampedBest;
}

function lowFontStep(_min: number, _test: number) {}

try {
  if (typeof globalThis !== 'undefined') {
    (globalThis as any).autoFitLyrics = autoFitLyrics;
  }
} catch (_) {}
