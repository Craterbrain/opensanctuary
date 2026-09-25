import { TextBlock, TextRun, ElementEffects, TextParagraphStyle, ElementTransform, SlideElement } from './types';
import { autoFitLyrics } from '../core/autofit';
import { reconcileTextBlockFromDOM } from './text_selection';

export function formatBulletLines(text: string, kind: 'none' | 'disc' | 'decimal' | 'liturgical'): string {
  const lines = text.split('\n');
  let numIndex = 1;
  return lines.map(line => {
    const stripped = line.replace(/^\s*([•\-\*]\s+|\d+[\.\)]\s+)/, '');
    if (!stripped.trim()) return line;
    if (kind === 'disc') {
      return `• ${stripped}`;
    } else if (kind === 'decimal') {
      const formatted = `${numIndex}. ${stripped}`;
      numIndex++;
      return formatted;
    } else {
      return stripped;
    }
  }).join('\n');
}

export function createDefaultTextBlock(
  transform: ElementTransform,
  text: string = 'Enter text here...',
  paragraphStyle?: TextParagraphStyle,
  autofit: boolean = false,
  fontSizePt: number = 36
): SlideElement {
  return {
    type: 'TextBlock',
    id: `tb-${Date.now()}-${Math.random().toString(36).substr(2, 4)}`,
    transform,
    block: {
      runs: [{ text, font_family: 'Roboto', font_size_pt: fontSizePt, color: '#ffffff' }],
      paragraph_style: paragraphStyle ?? { align: 'center', line_height: 1.25 },
      autofit
    }
  };
}

export function renderTextBlockDOM(
  containerEl: HTMLElement,
  block: TextBlock,
  isEditing: boolean = false,
  onContentChange?: (updatedBlock: TextBlock) => void
): void {
  // Note: intentionally does not set containerEl's own width/height — the
  // caller (canvas.ts / slide_render.ts) already positions and sizes this
  // element absolutely in px; a CSS width/height of '100%' here would resolve
  // against the nearest positioned ANCESTOR instead of preserving that size.
  containerEl.innerHTML = '';
  containerEl.classList.add('slide-element-text-content');
  containerEl.style.boxSizing = 'border-box';
  containerEl.style.wordBreak = 'break-word';
  containerEl.style.whiteSpace = 'pre-wrap';
  containerEl.style.display = 'flex';
  containerEl.style.flexDirection = 'column';

  const pStyle = block.paragraph_style || {};
  containerEl.style.textAlign = pStyle.align || 'center';
  containerEl.style.lineHeight = `${pStyle.line_height || 1.25}`;
  const indentPx = (pStyle.indent_level || 0) * 24;
  containerEl.style.paddingLeft = indentPx > 0 ? `${indentPx}px` : '';
  if (pStyle.align === 'center') {
    containerEl.style.alignItems = 'center';
    containerEl.style.justifyContent = 'center';
  } else if (pStyle.align === 'right') {
    containerEl.style.alignItems = 'flex-end';
    containerEl.style.justifyContent = 'center';
  } else {
    containerEl.style.alignItems = 'flex-start';
    containerEl.style.justifyContent = 'center';
  }

  // Apply Effects to container
  applyEffectsToElement(containerEl, block.effects);

  if (isEditing) {
    containerEl.contentEditable = 'true';
    containerEl.style.cursor = 'text';
    containerEl.style.userSelect = 'text';
    containerEl.style.outline = 'none';

    // Combine runs into text with basic styling
    for (const run of block.runs) {
      const span = createRunSpan(run, block.autofit);
      containerEl.appendChild(span);
    }

    // Reconciles the contenteditable's actual per-span DOM back into a real
    // TextRun[] instead of collapsing everything into one run — preserves
    // per-run formatting (e.g. one bolded word) across edits. The backend
    // coalesces adjacent identically-styled runs on save (see
    // merge_contiguous_text_runs in src/core/engine.rs), so no need to merge
    // client-side here.
    //
    // Note: an autofit-enabled block intentionally shows no explicit
    // font-size while actively being edited (createRunSpan skips it below),
    // so it renders at the page's small default until the next blur commits
    // it through the non-editing path's real autoFitLyrics measurement.
    // Re-running that same measurement here would need to treat `containerEl`
    // as both the sizing reference AND the resized target (there's no
    // separate inner wrapper in edit mode, unlike the non-editing branch
    // below) — risky, since this container is also the absolutely-positioned,
    // flex-centered element the whole slide canvas uses for layout, not an
    // inert measuring box. Left as a known, self-correcting-on-blur rough
    // edge rather than risk that.
    const handleInput = () => {
      if (onContentChange) {
        const updatedRuns = reconcileTextBlockFromDOM(containerEl);
        onContentChange({
          ...block,
          runs: updatedRuns
        });
      }
    };

    containerEl.oninput = handleInput;
    containerEl.onblur = () => {
      containerEl.contentEditable = 'false';
      containerEl.style.cursor = 'default';
      containerEl.style.userSelect = 'none';
      handleInput();
    };

    setTimeout(() => {
      containerEl.focus();
      // Select all or move caret to end
      const range = document.createRange();
      range.selectNodeContents(containerEl);
      const sel = window.getSelection();
      sel?.removeAllRanges();
      sel?.addRange(range);
    }, 10);
  } else {
    containerEl.contentEditable = 'false';
    containerEl.style.cursor = 'inherit';
    containerEl.style.userSelect = 'none';

    const textInner = document.createElement('div');
    textInner.className = 'slide-text-inner';
    textInner.style.width = '100%';
    textInner.style.textAlign = pStyle.align || 'center';
    textInner.style.lineHeight = `${pStyle.line_height || 1.25}`;
    textInner.style.whiteSpace = 'pre-wrap';
    textInner.style.wordBreak = 'break-word';

    const basePt = block.runs[0]?.font_size_pt || 36;
    textInner.style.fontSize = block.autofit ? '48px' : `${basePt}pt`;

    for (const run of block.runs) {
      const span = createRunSpan(run, block.autofit);
      textInner.appendChild(span);
    }
    containerEl.appendChild(textInner);

    if (block.autofit && containerEl.clientWidth > 0 && containerEl.clientHeight > 0) {
      autoFitLyrics(containerEl, textInner, {
        minFontSize: 14,
        maxFontSize: 100
      });
    }
  }
}

function createRunSpan(run: TextRun, inheritFontSize: boolean = false): HTMLElement {
  const span = document.createElement('span');
  span.textContent = run.text;
  if (run.bold) span.style.fontWeight = 'bold';
  if (run.italic) span.style.fontStyle = 'italic';
  if (run.underline) span.style.textDecoration = 'underline';
  if (run.strike) span.style.textDecoration = `${span.style.textDecoration} line-through`.trim();
  if (run.color) span.style.color = run.color;
  if (run.font_family) span.style.fontFamily = run.font_family;
  if (!inheritFontSize && run.font_size_pt) span.style.fontSize = `${run.font_size_pt}pt`;
  if (run.letter_spacing_px) span.style.letterSpacing = `${run.letter_spacing_px}px`;
  if (run.baseline_shift === 'sub') {
    span.style.verticalAlign = 'sub';
    span.style.fontSize = '0.75em';
  } else if (run.baseline_shift === 'super') {
    span.style.verticalAlign = 'super';
    span.style.fontSize = '0.75em';
  }
  return span;
}

export function applyEffectsToElement(el: HTMLElement, effects?: ElementEffects): void {
  if (!effects) return;

  // 1. Text Outline
  if (effects.outline && effects.outline.width > 0) {
    (el.style as any).webkitTextStroke = `${effects.outline.width}px ${effects.outline.color}`;
  } else {
    (el.style as any).webkitTextStroke = '';
  }

  // 2. Text Shadow
  if (effects.shadow) {
    el.style.textShadow = `${effects.shadow.dx}px ${effects.shadow.dy}px ${effects.shadow.blur}px ${effects.shadow.color}`;
  } else {
    el.style.textShadow = '';
  }

  // 3. Reflection
  if (effects.reflection && effects.reflection.enabled) {
    (el.style as any).webkitBoxReflect = `below 2px linear-gradient(transparent, rgba(0,0,0,${effects.reflection.opacity}))`;
  } else {
    (el.style as any).webkitBoxReflect = '';
  }

  // 4. Glow
  if (effects.glow && effects.glow.radius > 0) {
    el.style.filter = `drop-shadow(0 0 ${effects.glow.radius}px ${effects.glow.color})`;
  } else {
    el.style.filter = '';
  }
}
