// Drawing a Shape's or Line's own `path` (an imported freeform, connector or metafile drawing):
// SVG path data in the element's 0..100 box, drawn in the element's real pixel size rather than a
// stretched viewBox, so line widths and arrowheads keep their shape however the box is sized.

const SVG_NS = 'http://www.w3.org/2000/svg';
const ARGS: Record<string, number> = { M: 2, L: 2, C: 6, Q: 4, A: 7, Z: 0 };

/** Scales 0..100 path data to `w` x `h` pixels (arc radii too; arc flags and rotation untouched). */
export function scalePath(d: string, w: number, h: number): string {
  const tokens = d.match(/[MLCQAZ]|-?\d*\.?\d+(?:e[-+]?\d+)?/gi) || [];
  const out: string[] = [];
  let cmd = '';
  let args: number[] = [];
  const flush = () => {
    const n = ARGS[cmd.toUpperCase()] ?? 0;
    if (n === 0) return;
    for (let i = 0; i + n <= args.length; i += n) {
      const g = args.slice(i, i + n);
      const at = (k: number) => g[k] ?? 0;
      const scaled = cmd.toUpperCase() === 'A'
        ? [at(0) * w / 100, at(1) * h / 100, at(2), at(3), at(4), at(5) * w / 100, at(6) * h / 100]
        : g.map((v, k) => (k % 2 === 0 ? v * w / 100 : v * h / 100));
      out.push(`${cmd} ${scaled.map((v) => +v.toFixed(2)).join(' ')}`);
    }
  };
  for (const t of tokens) {
    if (/^[a-z]$/i.test(t)) {
      flush();
      cmd = t;
      args = [];
      if (t.toUpperCase() === 'Z') out.push('Z');
    } else {
      args.push(parseFloat(t));
    }
  }
  flush();
  return out.join(' ');
}

export interface PathDrawing {
  d: string;
  fill: string;
  stroke?: string;
  /** In canvas units: 1080 to the canvas height (see the `path` field in types.ts). */
  stroke_width?: number;
  start_arrow?: boolean;
  end_arrow?: boolean;
  opacity?: number;
  /** Canvas pixels per canvas unit (projected height / 1080); 1 in the editor's full-size stage. */
  scale?: number;
}

let markerSeq = 0;

export function renderPathDOM(containerEl: HTMLElement, p: PathDrawing): void {
  containerEl.innerHTML = '';
  containerEl.style.opacity = `${p.opacity ?? 1.0}`;
  // The caller sizes the element in px; a flat line's box can be 0 tall, so draw outside it.
  const w = parseFloat(containerEl.style.width) || containerEl.clientWidth || 0;
  const h = parseFloat(containerEl.style.height) || containerEl.clientHeight || 0;
  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('width', `${Math.max(w, 1)}`);
  svg.setAttribute('height', `${Math.max(h, 1)}`);
  svg.style.overflow = 'visible';
  svg.style.position = 'absolute';
  svg.style.left = '0';
  svg.style.top = '0';

  const stroke = p.stroke || 'none';
  const strokePx = Math.max((p.stroke_width || 0) * (p.scale ?? 1), stroke === 'none' ? 0 : 0.75);
  const path = document.createElementNS(SVG_NS, 'path');
  path.setAttribute('d', scalePath(p.d, w, h));
  path.setAttribute('fill', p.fill || 'none');
  path.setAttribute('fill-rule', 'evenodd');
  path.setAttribute('stroke', stroke);
  path.setAttribute('stroke-width', `${strokePx}`);
  path.setAttribute('stroke-linejoin', 'round');

  if ((p.start_arrow || p.end_arrow) && stroke !== 'none') {
    const id = `os-arrow-${++markerSeq}`;
    const defs = document.createElementNS(SVG_NS, 'defs');
    const marker = document.createElementNS(SVG_NS, 'marker');
    marker.setAttribute('id', id);
    marker.setAttribute('viewBox', '0 0 10 10');
    marker.setAttribute('refX', '8');
    marker.setAttribute('refY', '5');
    marker.setAttribute('markerWidth', '4');
    marker.setAttribute('markerHeight', '4');
    marker.setAttribute('orient', 'auto-start-reverse');
    const head = document.createElementNS(SVG_NS, 'path');
    head.setAttribute('d', 'M 0 0 L 10 5 L 0 10 Z');
    head.setAttribute('fill', stroke);
    marker.appendChild(head);
    defs.appendChild(marker);
    svg.appendChild(defs);
    if (p.start_arrow) path.setAttribute('marker-start', `url(#${id})`);
    if (p.end_arrow) path.setAttribute('marker-end', `url(#${id})`);
  }

  svg.appendChild(path);
  containerEl.appendChild(svg);
}
