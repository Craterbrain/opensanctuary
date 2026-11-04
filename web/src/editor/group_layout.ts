// A Group's children are stored in slide coordinates (the group's transform is just their
// bounding box), but each is drawn inside the group's own positioned box: shift it back.

/** Moves a group child's node from slide coordinates into its group's box at `groupRect`. */
export function placeInGroup(child: HTMLElement, groupRect: { left: number; top: number }): HTMLElement {
  child.style.left = `${(parseFloat(child.style.left) || 0) - groupRect.left}px`;
  child.style.top = `${(parseFloat(child.style.top) || 0) - groupRect.top}px`;
  return child;
}
