import { CropRect } from './types';

export interface ImageElementProps {
  file_path: string;
  crop?: CropRect;
  mask_shape?: string;
  alt_text?: string;
  opacity?: number;
}

/**
 * Computes the CSS background-size/background-position pair that makes a
 * `crop` window (normalized 0..1 fractions of the FULL source image) fill a
 * container exactly, when applied to a background-image of that source.
 *
 * Derivation: with background-size set to (100/w)% (100/h)%, the crop
 * window's width/height (w/h fraction of the image) each become exactly
 * 100% of the container. background-position's percentage P is defined by
 * the spec as used_position = (container_size - scaled_bg_size) * P/100; to
 * make the crop window's top-left (image fraction x) land at the container's
 * left edge, solving for P gives P = 100 * x / (1 - w) (and analogously for
 * y/h). This is the standard "CSS crop-to-subrect via background-image"
 * formula, guarded against the w=1 / h=1 (no crop on that axis) division.
 */
export function computeCropBackgroundStyle(crop: CropRect): { size: string; position: string } {
  const w = Math.max(0.01, Math.min(1, crop.w));
  const h = Math.max(0.01, Math.min(1, crop.h));
  const x = Math.max(0, Math.min(1 - w, crop.x));
  const y = Math.max(0, Math.min(1 - h, crop.y));
  const sizeX = 100 / w;
  const sizeY = 100 / h;
  const posX = w >= 1 ? 0 : (100 * x) / (1 - w);
  const posY = h >= 1 ? 0 : (100 * y) / (1 - h);
  return {
    size: `${sizeX}% ${sizeY}%`,
    position: `${posX}% ${posY}%`
  };
}

function isEffectivelyUncropped(crop?: CropRect): boolean {
  if (!crop) return true;
  return crop.x === 0 && crop.y === 0 && crop.w >= 1 && crop.h >= 1;
}

export function renderImageDOM(containerEl: HTMLElement, props: ImageElementProps): void {
  // See note in shape_library.ts: don't touch containerEl's width/height here,
  // the caller already sizes it absolutely in px.
  containerEl.innerHTML = '';
  containerEl.style.overflow = 'hidden';
  containerEl.style.position = 'relative';

  // Rendered as a background-image (not <img> + object-fit/clip-path): a crop
  // rect is authored as a fraction of the image's own natural size, and
  // background-size/-position can express "zoom into this exact sub-rect"
  // precisely regardless of the container's aspect ratio — object-fit: none
  // + clip-path only happened to be correct when the natural image size
  // matched the container's pixel box.
  const inner = document.createElement('div');
  inner.setAttribute('role', 'img');
  if (props.alt_text) inner.setAttribute('aria-label', props.alt_text);
  inner.style.position = 'absolute';
  inner.style.inset = '0';
  inner.style.backgroundImage = `url("${props.file_path}")`;
  inner.style.backgroundRepeat = 'no-repeat';
  inner.style.opacity = `${props.opacity ?? 1.0}`;

  if (props.crop && !isEffectivelyUncropped(props.crop)) {
    const { size, position } = computeCropBackgroundStyle(props.crop);
    inner.style.backgroundSize = size;
    inner.style.backgroundPosition = position;
  } else {
    inner.style.backgroundSize = 'cover';
    inner.style.backgroundPosition = 'center';
  }

  if (props.mask_shape) {
    applyMaskShape(inner, props.mask_shape);
  }

  containerEl.appendChild(inner);
}

export function applyMaskShape(el: HTMLElement, maskShape: string): void {
  switch (maskShape) {
    case 'circle':
    case 'ellipse':
      el.style.borderRadius = '50%';
      break;
    case 'rounded':
      el.style.borderRadius = '16px';
      break;
    case 'star':
      (el.style as any).clipPath = 'polygon(50% 0%, 61% 35%, 98% 35%, 68% 57%, 79% 91%, 50% 70%, 21% 91%, 32% 57%, 2% 35%, 39% 35%)';
      break;
    case 'triangle':
      (el.style as any).clipPath = 'polygon(50% 0%, 0% 100%, 100% 100%)';
      break;
    case 'diamond':
      (el.style as any).clipPath = 'polygon(50% 0%, 100% 50%, 50% 100%, 0% 50%)';
      break;
    default:
      el.style.borderRadius = '0px';
      (el.style as any).clipPath = '';
      break;
  }
}
