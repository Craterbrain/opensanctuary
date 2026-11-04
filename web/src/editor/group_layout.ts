import type { ElementTransform, SlideElement } from './types';

// A Group's children are stored in slide coordinates (the group's transform is just their
// bounding box), but each is drawn inside the group's own positioned box: shift it back.

/** Moves a group child's node from slide coordinates into its group's box at `groupRect`. */
export function placeInGroup(child: HTMLElement, groupRect: { left: number; top: number }): HTMLElement {
  child.style.left = `${(parseFloat(child.style.left) || 0) - groupRect.left}px`;
  child.style.top = `${(parseFloat(child.style.top) || 0) - groupRect.top}px`;
  return child;
}

/**
 * After a group's transform changed from `before`, moves (and scales) its children with it -- the
 * same rule the engine applies (`SlideElement::set_transform`), so the editor's draft matches what
 * Apply will save, and Ungroup drops the children where they're shown.
 */
export function carryGroupChildren(el: SlideElement, before: ElementTransform): void {
  if (el.type !== 'Group') return;
  const t = el.transform;
  const sx = before.w > 0 ? t.w / before.w : 1;
  const sy = before.h > 0 ? t.h / before.h : 1;
  for (const child of el.children) {
    const c = { ...child.transform };
    child.transform = { ...c, x: t.x + (c.x - before.x) * sx, y: t.y + (c.y - before.y) * sy, w: c.w * sx, h: c.h * sy };
    carryGroupChildren(child, c);
  }
}
