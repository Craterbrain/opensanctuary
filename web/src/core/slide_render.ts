// Renders a slide's positioned `elements[]` (text blocks, images, shapes,
// lines, tables, groups) for a non-editing output surface — the in-app Live
// preview, the projector output (live.html), and the confidence monitor
// (stage.html). This is pure composition of the same per-type DOM renderers
// the canvas editor itself uses (see web/src/editor/*.ts), so what an operator
// designs in the editor is exactly what plays live: no separate rendering
// logic is implemented here.
import { CanvasProjection, computeCanvasProjection, projectRect, NormalizedRect } from './canvas_projection';
import { renderTextBlockDOM } from '../editor/text_block';
import { renderImageDOM } from '../editor/image_tools';
import { renderVideoDOM } from '../editor/video_tools';
import { renderShapeDOM } from '../editor/shape_library';
import { renderLineDOM } from '../editor/line_tool';
import { renderTableDOM } from '../editor/table';
import { autoFitLyrics } from './autofit';
import { escapeHtml, parseParallelSlide, applyThemeTypography, type ThemeDefinition } from './presentation_helpers';

// Kept structurally compatible with web/src/editor/types.ts's SlideElement,
// but declared loosely here since this module must also run against slides
// received over the wire as plain JSON (no shared type-only import needed).
export interface RenderableElement {
  type: 'TextBlock' | 'Image' | 'Video' | 'Shape' | 'Line' | 'Table' | 'Group';
  id: string;
  transform: NormalizedRect & { rotation_deg: number; z_index: number; opacity: number };
  block?: any;
  file_path?: string;
  crop?: any;
  mask_shape?: string;
  alt_text?: string;
  in_point_s?: number;
  out_point_s?: number;
  loop_playback?: boolean;
  is_muted?: boolean;
  volume?: number;
  shape_kind?: string;
  fill_color?: string;
  stroke_color?: string;
  stroke_width?: number;
  line_kind?: string;
  color?: string;
  start_arrow?: boolean;
  end_arrow?: boolean;
  rows?: number;
  cols?: number;
  cells?: string[][];
  children?: RenderableElement[];
}

function createElementNode(el: RenderableElement, projection: CanvasProjection): HTMLElement {
  const node = document.createElement('div');
  node.className = `slide-canvas-element slide-canvas-element-${el.type.toLowerCase()}`;
  node.dataset.elementId = el.id;
  node.style.position = 'absolute';
  node.style.boxSizing = 'border-box';

  const rect = projectRect(el.transform, projection);
  node.style.left = `${rect.left}px`;
  node.style.top = `${rect.top}px`;
  node.style.width = `${rect.width}px`;
  node.style.height = `${rect.height}px`;
  node.style.transform = `rotate(${el.transform.rotation_deg || 0}deg)`;
  node.style.opacity = `${el.transform.opacity ?? 1}`;
  node.style.zIndex = `${el.transform.z_index ?? 0}`;
  node.style.pointerEvents = 'none';

  switch (el.type) {
    case 'TextBlock': {
      const scale = projection.contentH > 0 ? projection.contentH / 1080 : 1;
      renderTextBlockDOM(node, el.block, false, undefined, scale);
      break;
    }
    case 'Image':
      renderImageDOM(node, {
        file_path: el.file_path!,
        crop: el.crop,
        mask_shape: el.mask_shape,
        alt_text: el.alt_text,
        opacity: el.transform.opacity
      });
      break;
    case 'Video':
      renderVideoDOM(node, {
        file_path: el.file_path!,
        in_point_s: el.in_point_s,
        out_point_s: el.out_point_s,
        loop_playback: el.loop_playback,
        is_muted: el.is_muted,
        volume: el.volume,
        opacity: el.transform.opacity
      });
      break;
    case 'Shape':
      renderShapeDOM(node, {
        shape_kind: el.shape_kind!,
        fill_color: el.fill_color!,
        stroke_color: el.stroke_color,
        stroke_width: el.stroke_width,
        opacity: el.transform.opacity
      });
      break;
    case 'Line':
      renderLineDOM(node, {
        line_kind: el.line_kind!,
        color: el.color!,
        stroke_width: el.stroke_width,
        start_arrow: el.start_arrow,
        end_arrow: el.end_arrow,
        opacity: el.transform.opacity
      });
      break;
    case 'Table':
      renderTableDOM(node, {
        rows: el.rows!,
        cols: el.cols!,
        cells: el.cells!,
        opacity: el.transform.opacity
      }, false);
      break;
    case 'Group':
      for (const child of el.children || []) {
        node.appendChild(createElementNode(child, projection));
      }
      break;
  }

  return node;
}

/**
 * Renders `elements` (already sorted or not) into `hostEl`, mapped through
 * `projection` (see computeCanvasProjection). `hostEl` should be an
 * absolutely/fixed-positioned wrapper already sized to the target output box;
 * this function only manages its own children.
 */
export function renderSlideElements(
  hostEl: HTMLElement,
  elements: RenderableElement[] | undefined | null,
  projection: CanvasProjection
): void {
  hostEl.innerHTML = '';
  if (!elements || elements.length === 0) return;

  const sorted = [...elements].sort((a, b) => (a.transform?.z_index ?? 0) - (b.transform?.z_index ?? 0));
  for (const el of sorted) {
    hostEl.appendChild(createElementNode(el, projection));
  }

  // Autofit text measurement requires the nodes to already be mounted (their
  // clientWidth/clientHeight must be non-zero), so it runs as a second pass —
  // mirrors the editor canvas's own two-phase render (see EditorCanvas.render).
  for (const el of sorted) {
    if (el.type === 'TextBlock' && el.block?.autofit) {
      const elNode = hostEl.querySelector(`[data-element-id="${el.id}"]`) as HTMLElement | null;
      const textInner = elNode?.querySelector('.slide-text-inner') as HTMLElement | null;
      if (elNode && textInner) {
        autoFitLyrics(elNode, textInner, { minFontSize: 14, maxFontSize: 100 });
      }
    }
  }
}

/**
 * Formats a slide's flat `text` field into lyrics HTML, splitting a
 * parallel-translation slide into its dual-column layout.
 */
export function formatSlideLyricsHtml(textContent: string): string {
  if (!textContent) return '';
  const parsed = parseParallelSlide(textContent);
  if (parsed.isParallel) {
    return `<div class="dual-slide-grid" style="display: grid; grid-template-columns: 1fr 1fr; gap: 8px; width: 100%; text-align: left; align-items: start;">
      <div class="dual-slide-col primary" style="background: rgba(0,0,0,0.3); padding: 4px 8px; border-radius: 4px; border-left: 2px solid #ffa726; font-size: 0.9em; white-space: pre-line;">${escapeHtml(parsed.leftText)}</div>
      <div class="dual-slide-col secondary" style="background: rgba(0,0,0,0.3); padding: 4px 8px; border-radius: 4px; border-left: 2px solid #00e5ff; font-size: 0.9em; white-space: pre-line;">${escapeHtml(parsed.rightText)}</div>
    </div>`;
  }
  return escapeHtml(textContent).replace(/\n/g, '<br>');
}

/**
 * Sets `lyricsEl`'s HTML and binary-searches a font size that actually fits
 * its container. A fixed font-size-by-character-count bucket table can never
 * generalize across window sizes/containers — this is why every lyrics
 * surface (Slide Editor canvas, Live, Preview, projector output) measures
 * the real container instead. Invariant: inv.text.autofit-single-source
 */
export function applyAutoFitLyrics(lyricsEl: HTMLElement, html: string): void {
  lyricsEl.innerHTML = html;
  autoFitLyrics(lyricsEl.parentElement, lyricsEl, { minFontSize: 12 });
}

/**
 * Renders one slide's actual content into a viewport — the single canonical
 * "how does a slide look" entry point, shared by every non-editing surface
 * (Slide Editor filmstrip, Resources tab preview, operator Preview/Live
 * canvases) so they can never independently drift apart on how a slide
 * looks. If the slide has real positioned elements (text runs, images,
 * shapes, lines, tables — the normal case for anything built in the Slide
 * Editor), it renders them exactly as authored via `renderSlideElements`
 * against a projection of the *actual* canvas box; otherwise it falls back
 * to the flattened single-textbox autofit path.
 *
 * NOTE: live_output.ts (the actual projector/FOH output, not an operator-
 * facing surface) has its OWN independent copy of this same
 * elements-vs-flat-text branch -- it isn't calling this function, because it
 * also needs extra cases this one doesn't (parallel-text layout, pinned
 * reference label positioning). If the elements-vs-flat-text decision logic
 * itself changes (not just styling), check live_output.ts's branch too.
 */
export function renderSlideVisual(
  canvasEl: HTMLElement,
  elementsEl: HTMLElement | null,
  lyricsEl: HTMLElement,
  slide: any,
  theme: ThemeDefinition | null = null
): void {
  applyThemeTypography(lyricsEl, theme);
  // Compact preview surfaces always show the reference inline (no separate
  // pinned-overlay element here, unlike live_output.ts's real FOH output) —
  // good enough for a non-broadcast working view.
  const isScriptureTheme = !!(theme && theme.category === 'scripture');
  const inlinePrefix = (isScriptureTheme && slide && slide.reference_label) ? `${slide.reference_label}  ` : '';

  const hasPositionedElements = !!(slide && slide.elements && slide.elements.length > 0);
  if (hasPositionedElements && elementsEl) {
    lyricsEl.style.opacity = '0';
    elementsEl.style.display = 'block';
    const projection = computeCanvasProjection(canvasEl.clientWidth, canvasEl.clientHeight, { authoredAspect: 16 / 9 });
    renderSlideElements(elementsEl, slide.elements, projection);
  } else {
    if (elementsEl) {
      elementsEl.style.display = 'none';
      elementsEl.innerHTML = '';
    }
    lyricsEl.style.opacity = '1';
    const textContent = slide ? (slide.text || '') : '';
    applyAutoFitLyrics(lyricsEl, formatSlideLyricsHtml(inlinePrefix ? `${inlinePrefix}${textContent}` : textContent));
  }
}
