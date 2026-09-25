// Renders a slide's positioned `elements[]` (text blocks, images, shapes,
// lines, tables, groups) for a non-editing output surface — the in-app Live
// preview, the projector output (live.html), and the confidence monitor
// (stage.html). This is pure composition of the same per-type DOM renderers
// the canvas editor itself uses (see web/src/editor/*.ts), so what an operator
// designs in the editor is exactly what plays live: no separate rendering
// logic is implemented here.
import { CanvasProjection, projectRect, NormalizedRect } from './canvas_projection';
import { renderTextBlockDOM } from '../editor/text_block';
import { renderImageDOM } from '../editor/image_tools';
import { renderVideoDOM } from '../editor/video_tools';
import { renderShapeDOM } from '../editor/shape_library';
import { renderLineDOM } from '../editor/line_tool';
import { renderTableDOM } from '../editor/table';
import { autoFitLyrics } from './autofit';

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
  start_arrow?: string;
  end_arrow?: string;
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
    case 'TextBlock':
      renderTextBlockDOM(node, el.block, false);
      break;
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
        fill_color: el.fill_color,
        stroke_color: el.stroke_color,
        stroke_width: el.stroke_width,
        opacity: el.transform.opacity
      });
      break;
    case 'Line':
      renderLineDOM(node, {
        line_kind: el.line_kind!,
        color: el.color,
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
