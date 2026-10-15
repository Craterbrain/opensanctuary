import { ElementTransform, SlideElement } from './types';

export interface ShapeElementProps {
  shape_kind: string;
  fill_color: string;
  stroke_color?: string;
  stroke_width?: number;
  opacity?: number;
}

export function createDefaultShape(
  shape_kind: string,
  transform: ElementTransform,
  fill_color: string = '#3b82f6',
  stroke_color: string = '#ffffff',
  stroke_width: number = 0
): SlideElement {
  return {
    type: 'Shape',
    id: `shp-${Date.now()}-${Math.random().toString(36).substr(2, 4)}`,
    transform,
    shape_kind,
    fill_color,
    stroke_color,
    stroke_width
  };
}

export function renderShapeDOM(containerEl: HTMLElement, props: ShapeElementProps): void {
  // Note: intentionally does not set containerEl's width/height — the caller
  // (canvas.ts / slide_render.ts) already positions and sizes this element
  // absolutely in px; a CSS width/height of '100%' here would resolve against
  // the nearest positioned ANCESTOR instead of preserving that size.
  containerEl.innerHTML = '';
  containerEl.style.opacity = `${props.opacity ?? 1.0}`;

  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('width', '100%');
  svg.setAttribute('height', '100%');
  svg.setAttribute('viewBox', '0 0 100 100');
  svg.setAttribute('preserveAspectRatio', 'none');

  const stroke = props.stroke_color || 'none';
  const strokeW = props.stroke_width || 0;
  const fill = props.fill_color;

  let shapeNode: SVGElement;

  switch (props.shape_kind) {
    case 'ellipse':
    case 'circle': {
      const el = document.createElementNS('http://www.w3.org/2000/svg', 'ellipse');
      el.setAttribute('cx', '50');
      el.setAttribute('cy', '50');
      el.setAttribute('rx', `${50 - strokeW / 2}`);
      el.setAttribute('ry', `${50 - strokeW / 2}`);
      shapeNode = el;
      break;
    }
    case 'rounded_rect': {
      const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
      rect.setAttribute('x', `${strokeW / 2}`);
      rect.setAttribute('y', `${strokeW / 2}`);
      rect.setAttribute('width', `${100 - strokeW}`);
      rect.setAttribute('height', `${100 - strokeW}`);
      rect.setAttribute('rx', '15');
      rect.setAttribute('ry', '15');
      shapeNode = rect;
      break;
    }
    case 'triangle': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '50,5 95,95 5,95');
      shapeNode = poly;
      break;
    }
    case 'diamond': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '50,5 95,50 50,95 5,50');
      shapeNode = poly;
      break;
    }
    case 'star': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '50,5 62,38 97,38 68,59 80,95 50,72 20,95 32,59 3,38 38,38');
      shapeNode = poly;
      break;
    }
    case 'callout': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '5,5 95,5 95,75 50,75 25,95 35,75 5,75');
      shapeNode = poly;
      break;
    }
    case 'trapezoid': {
      // ODF's own "trapezoid" preset shape: wider at the top, narrower at the bottom (same
      // point set odp_export.rs's shape_enhanced_path uses for this exact kind, so an ODP
      // import/export round trip renders identically in-app and in the exported file).
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '10,5 90,5 75,95 25,95');
      shapeNode = poly;
      break;
    }
    case 'left-right-arrow': {
      // A horizontal double-headed arrow (ODF's own "left-right-arrow" preset name).
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '2,50 20,25 20,38 80,38 80,25 98,50 80,75 80,62 20,62 20,75');
      shapeNode = poly;
      break;
    }
    // The following are all plain fixed-point polygons in the same 0..100 space, clean-room
    // designed from scratch (not traced from any other application's source or asset) using
    // real LibreOffice Impress's own gallery only as a naming/proportions *reference* -- see
    // docs/IMPORT_EXPORT_NOTES.md. Each one's exact point set is mirrored in odp_export.rs's
    // shape_enhanced_path so the in-app render and the exported ODF file's render agree.
    case 'parallelogram': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '30,5 95,5 70,95 5,95');
      shapeNode = poly;
      break;
    }
    case 'hexagon': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '25,5 75,5 95,50 75,95 25,95 5,50');
      shapeNode = poly;
      break;
    }
    case 'octagon': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '30,5 70,5 95,30 95,70 70,95 30,95 5,70 5,30');
      shapeNode = poly;
      break;
    }
    case 'cross': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '35,5 65,5 65,35 95,35 95,65 65,65 65,95 35,95 35,65 5,65 5,35 35,35');
      shapeNode = poly;
      break;
    }
    case 'pentagon': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '50,5 95,38 78,95 22,95 5,38');
      shapeNode = poly;
      break;
    }
    case 'heart': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '50,20 38,5 15,5 5,20 5,40 50,95 95,40 95,20 85,5 62,5');
      shapeNode = poly;
      break;
    }
    case 'lightning': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '60,5 25,55 50,55 35,95 85,40 55,40 70,5');
      shapeNode = poly;
      break;
    }
    case 'left-arrow': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '95,30 35,30 35,10 5,50 35,90 35,70 95,70');
      shapeNode = poly;
      break;
    }
    case 'right-arrow': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '5,30 65,30 65,10 95,50 65,90 65,70 5,70');
      shapeNode = poly;
      break;
    }
    case 'up-arrow': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '30,95 30,35 10,35 50,5 90,35 70,35 70,95');
      shapeNode = poly;
      break;
    }
    case 'down-arrow': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '30,5 30,65 10,65 50,95 90,65 70,65 70,5');
      shapeNode = poly;
      break;
    }
    case 'up-down-arrow': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '30,25 10,25 50,5 90,25 70,25 70,75 90,75 50,95 10,75 30,75');
      shapeNode = poly;
      break;
    }
    case 'quad-arrow': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute(
        'points',
        '50,5 65,20 58,20 58,42 80,42 80,35 95,50 80,65 80,58 58,58 58,80 65,80 50,95 35,80 42,80 42,58 20,58 20,65 5,50 20,35 20,42 42,42 42,20 35,20'
      );
      shapeNode = poly;
      break;
    }
    case 'chevron': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '5,5 65,5 95,50 65,95 5,95 35,50');
      shapeNode = poly;
      break;
    }
    case 'notched-right-arrow': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '5,30 55,30 55,10 95,50 55,90 55,70 5,70 25,50');
      shapeNode = poly;
      break;
    }
    case 'striped-right-arrow': {
      // Same outline as right-arrow; the gallery original also has two tail stripes, simplified
      // away here (this module only ever emits one SVG node per shape -- see the module doc).
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '5,30 65,30 65,10 95,50 65,90 65,70 5,70');
      shapeNode = poly;
      break;
    }
    case 'triangle-down': {
      const poly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
      poly.setAttribute('points', '5,5 95,5 50,95');
      shapeNode = poly;
      break;
    }
    case 'ring': {
      // A hollow octagon-approximated annulus: an outer ring and an inner ring wound in
      // opposite directions in one path, so the nonzero fill rule renders the gap between them
      // as a hole -- not a stroke trick, so this survives an ODP export/import round trip the
      // same way every other shape_kind does (odp_export.rs's shape_enhanced_path uses the exact
      // same two point rings, in the same winding order, via the ODF-equivalent path mini-language).
      const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      path.setAttribute(
        'd',
        'M 50 5 L 82 18 L 95 50 L 82 82 L 50 95 L 18 82 L 5 50 L 18 18 Z ' +
          'M 32 32 L 25 50 L 32 68 L 50 75 L 68 68 L 75 50 L 68 32 L 50 25 Z'
      );
      path.setAttribute('fill-rule', 'nonzero');
      shapeNode = path;
      break;
    }
    case 'rect':
    default: {
      const rect = document.createElementNS('http://www.w3.org/2000/svg', 'rect');
      rect.setAttribute('x', `${strokeW / 2}`);
      rect.setAttribute('y', `${strokeW / 2}`);
      rect.setAttribute('width', `${100 - strokeW}`);
      rect.setAttribute('height', `${100 - strokeW}`);
      shapeNode = rect;
      break;
    }
  }

  shapeNode.setAttribute('fill', fill);
  shapeNode.setAttribute('stroke', stroke);
  shapeNode.setAttribute('stroke-width', `${strokeW}`);
  svg.appendChild(shapeNode);
  containerEl.appendChild(svg);
}
