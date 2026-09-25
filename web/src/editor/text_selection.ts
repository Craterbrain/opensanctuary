import { TextRun } from './types';

// Selection-aware run splitting for the text-box contenteditable editor.
// Deliberately does not use document.execCommand (deprecated/inconsistent,
// already avoided elsewhere in this editor) — instead it manipulates the
// per-run <span> DOM directly, which reconcileTextBlockFromDOM then reads
// back into a real TextRun[] on blur/input.

function applyRunStyleToSpan(span: HTMLElement, style: Partial<TextRun>): void {
  if (style.bold !== undefined) span.style.fontWeight = style.bold ? 'bold' : 'normal';
  if (style.italic !== undefined) span.style.fontStyle = style.italic ? 'italic' : 'normal';
  if (style.underline !== undefined || style.strike !== undefined) {
    const decorations: string[] = [];
    const isUnderline = style.underline !== undefined ? style.underline : span.style.textDecoration.includes('underline');
    const isStrike = style.strike !== undefined ? style.strike : span.style.textDecoration.includes('line-through');
    if (isUnderline) decorations.push('underline');
    if (isStrike) decorations.push('line-through');
    span.style.textDecoration = decorations.join(' ') || 'none';
  }
  if (style.color !== undefined) span.style.color = style.color;
  if (style.font_family !== undefined) span.style.fontFamily = style.font_family;
  if (style.font_size_pt !== undefined) span.style.fontSize = `${style.font_size_pt}pt`;
  if (style.letter_spacing_px !== undefined) span.style.letterSpacing = `${style.letter_spacing_px}px`;
  if (style.baseline_shift !== undefined) {
    if (style.baseline_shift === 'sub') {
      span.style.verticalAlign = 'sub';
      span.style.fontSize = '0.75em';
    } else if (style.baseline_shift === 'super') {
      span.style.verticalAlign = 'super';
      span.style.fontSize = '0.75em';
    } else {
      span.style.verticalAlign = '';
      span.style.fontSize = style.font_size_pt ? `${style.font_size_pt}pt` : '';
    }
  }
}

/**
 * Applies `style` to the current browser selection inside `containerEl`,
 * splitting the run span the selection falls within into up to 3 sibling
 * spans (before/selected/after) so only the selected text is restyled.
 *
 * Only supports a selection fully contained within a single top-level run
 * span (or a bare unstyled text node directly under containerEl) — the
 * common "select part of a run and restyle it" case. Returns false for a
 * cross-run selection or no/collapsed selection, so the caller can fall back
 * to whole-block styling.
 */
export function applyStyleToSelection(containerEl: HTMLElement, style: Partial<TextRun>): boolean {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0 || sel.isCollapsed) return false;

  const range = sel.getRangeAt(0);
  if (!containerEl.contains(range.commonAncestorContainer)) return false;

  const resolveTopLevelNode = (node: Node): HTMLElement | null => {
    let n: Node | null = node.nodeType === Node.TEXT_NODE ? node.parentElement : node;
    while (n && n.parentElement && n.parentElement !== containerEl) {
      n = n.parentElement;
    }
    return n instanceof HTMLElement && n.parentElement === containerEl ? n : null;
  };

  const startSpan = resolveTopLevelNode(range.startContainer);
  const endSpan = resolveTopLevelNode(range.endContainer);
  if (!startSpan || !endSpan || startSpan !== endSpan) {
    return false;
  }

  const span = startSpan;
  // Only handle the simple case where the span's own text node is the
  // direct selection boundary (true for spans created by createRunSpan,
  // which have exactly one text child).
  if (range.startContainer !== range.endContainer || range.startContainer.nodeType !== Node.TEXT_NODE) {
    return false;
  }

  const text = span.textContent || '';
  const startOffset = range.startOffset;
  const endOffset = range.endOffset;
  if (startOffset === endOffset) return false;

  const before = text.slice(0, startOffset);
  const selected = text.slice(startOffset, endOffset);
  const after = text.slice(endOffset);
  const baseStyle = span.getAttribute('style') || '';

  const newNodes: HTMLElement[] = [];
  if (before) {
    const b = document.createElement('span');
    b.setAttribute('style', baseStyle);
    b.textContent = before;
    newNodes.push(b);
  }

  const mid = document.createElement('span');
  mid.setAttribute('style', baseStyle);
  applyRunStyleToSpan(mid, style);
  mid.textContent = selected;
  newNodes.push(mid);

  if (after) {
    const a = document.createElement('span');
    a.setAttribute('style', baseStyle);
    a.textContent = after;
    newNodes.push(a);
  }

  span.replaceWith(...newNodes);

  const newRange = document.createRange();
  newRange.selectNodeContents(mid);
  sel.removeAllRanges();
  sel.addRange(newRange);
  return true;
}

function styleFromSpan(span: HTMLElement): Partial<TextRun> {
  const style = span.style;
  const result: Partial<TextRun> = {};
  if (style.fontWeight) result.bold = style.fontWeight === 'bold' || parseInt(style.fontWeight, 10) >= 700;
  if (style.fontStyle) result.italic = style.fontStyle === 'italic';
  if (style.textDecoration) {
    result.underline = style.textDecoration.includes('underline');
    result.strike = style.textDecoration.includes('line-through');
  }
  if (style.color) result.color = style.color;
  if (style.fontFamily) result.font_family = style.fontFamily;
  if (style.fontSize) {
    const parsed = parseFloat(style.fontSize);
    if (!isNaN(parsed)) result.font_size_pt = parsed;
  }
  if (style.letterSpacing) {
    const parsed = parseFloat(style.letterSpacing);
    if (!isNaN(parsed)) result.letter_spacing_px = parsed;
  }
  if (style.verticalAlign === 'sub') {
    result.baseline_shift = 'sub';
  } else if (style.verticalAlign === 'super') {
    result.baseline_shift = 'super';
  }
  return result;
}

/**
 * Reads the contenteditable's current per-span DOM structure back into a
 * real TextRun[], preserving whatever per-run styling the spans carry
 * (including splits made by applyStyleToSelection) instead of collapsing
 * everything into a single run.
 */
export function reconcileTextBlockFromDOM(containerEl: HTMLElement): TextRun[] {
  const runs: TextRun[] = [];

  const pushText = (text: string, style: Partial<TextRun>) => {
    if (!text) return;
    runs.push({ text, ...style } as TextRun);
  };

  const walk = (node: ChildNode, inheritedStyle: Partial<TextRun>) => {
    if (node.nodeType === Node.TEXT_NODE) {
      pushText(node.textContent || '', inheritedStyle);
    } else if (node.nodeName === 'BR') {
      pushText('\n', inheritedStyle);
    } else if (node.nodeName === 'SPAN') {
      const el = node as HTMLElement;
      const style = { ...inheritedStyle, ...styleFromSpan(el) };
      for (const child of Array.from(el.childNodes)) walk(child, style);
    } else if (node.nodeName === 'DIV' || node.nodeName === 'P') {
      // Enter key creates a new block in some browsers — treat as a
      // line break rather than losing the content.
      if (runs.length > 0) pushText('\n', {});
      const el = node as HTMLElement;
      for (const child of Array.from(el.childNodes)) walk(child, inheritedStyle);
    } else {
      const el = node as unknown as HTMLElement;
      if (el.childNodes) {
        for (const child of Array.from(el.childNodes)) walk(child, inheritedStyle);
      }
    }
  };

  for (const child of Array.from(containerEl.childNodes)) walk(child, {});
  if (runs.length === 0) runs.push({ text: '' } as TextRun);
  return runs;
}
