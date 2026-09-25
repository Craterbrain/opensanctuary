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
