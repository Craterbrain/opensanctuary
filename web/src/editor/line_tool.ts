import { ElementTransform, SlideElement } from './types';

export interface LineElementProps {
  line_kind: string; // "straight", "arrow", "elbow", "curved"
  color: string;
  stroke_width?: number;
  start_arrow?: boolean;
  end_arrow?: boolean;
  opacity?: number;
}

export function createDefaultLine(
  line_kind: string = 'straight',
  transform: ElementTransform,
  color: string = '#ffffff',
  stroke_width: number = 3,
  start_arrow: boolean = false,
  end_arrow: boolean = false
): SlideElement {
  return {
    type: 'Line',
    id: `line-${Date.now()}-${Math.random().toString(36).substr(2, 4)}`,
    transform,
    line_kind,
    color,
    stroke_width,
    start_arrow,
    end_arrow
  };
}

export function renderLineDOM(containerEl: HTMLElement, props: LineElementProps): void {
  // See note in shape_library.ts: don't touch containerEl's width/height here,
  // the caller already sizes it absolutely in px.
  containerEl.innerHTML = '';
  containerEl.style.opacity = `${props.opacity ?? 1.0}`;

  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('width', '100%');
  svg.setAttribute('height', '100%');
  svg.setAttribute('viewBox', '0 0 100 100');
  svg.setAttribute('preserveAspectRatio', 'none');

  const stroke = props.color || '#ffffff';
  const strokeW = props.stroke_width || 3;

  // Arrow markers
  const defs = document.createElementNS('http://www.w3.org/2000/svg', 'defs');
  if (props.end_arrow || props.line_kind === 'arrow') {
    const marker = document.createElementNS('http://www.w3.org/2000/svg', 'marker');
    marker.setAttribute('id', 'arrowhead');
    marker.setAttribute('markerWidth', '10');
    marker.setAttribute('markerHeight', '7');
    marker.setAttribute('refX', '9');
    marker.setAttribute('refY', '3.5');
    marker.setAttribute('orient', 'auto');
    const arrowPoly = document.createElementNS('http://www.w3.org/2000/svg', 'polygon');
    arrowPoly.setAttribute('points', '0 0, 10 3.5, 0 7');
    arrowPoly.setAttribute('fill', stroke);
    marker.appendChild(arrowPoly);
    defs.appendChild(marker);
  }
  svg.appendChild(defs);

  let pathNode: SVGElement;
  if (props.line_kind === 'elbow') {
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d', 'M 5 5 L 50 5 L 50 95 L 95 95');
    path.setAttribute('fill', 'none');
    pathNode = path;
  } else if (props.line_kind === 'curved') {
    const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    path.setAttribute('d', 'M 5 5 Q 50 95 95 50');
    path.setAttribute('fill', 'none');
    pathNode = path;
  } else {
    // straight or arrow
    const line = document.createElementNS('http://www.w3.org/2000/svg', 'line');
    line.setAttribute('x1', '5');
    line.setAttribute('y1', '50');
    line.setAttribute('x2', '95');
    line.setAttribute('y2', '50');
    pathNode = line;
  }

  pathNode.setAttribute('stroke', stroke);
  pathNode.setAttribute('stroke-width', `${strokeW}`);
  pathNode.setAttribute('stroke-linecap', 'round');
  if (props.end_arrow || props.line_kind === 'arrow') {
    pathNode.setAttribute('marker-end', 'url(#arrowhead)');
  }

  svg.appendChild(pathNode);
  containerEl.appendChild(svg);
}
