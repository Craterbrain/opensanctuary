import { SlideElement } from './types';

export class SelectionManager {
  private selectedIds: Set<string> = new Set();
  private listeners: Array<(selectedIds: string[]) => void> = [];

  public getSelectedIds(): string[] {
    return Array.from(this.selectedIds);
  }

  public isSelected(id: string): boolean {
    return this.selectedIds.has(id);
  }

  public select(id: string, multi = false): void {
    if (multi) {
      if (this.selectedIds.has(id)) {
        this.selectedIds.delete(id);
      } else {
        this.selectedIds.add(id);
      }
    } else {
      this.selectedIds.clear();
      this.selectedIds.add(id);
    }
    this.notify();
  }

  public selectAll(elements: SlideElement[]): void {
    this.selectedIds.clear();
    for (const el of elements) {
      this.selectedIds.add(el.id);
    }
    this.notify();
  }

  public deselectAll(): void {
    if (this.selectedIds.size > 0) {
      this.selectedIds.clear();
      this.notify();
    }
  }

  public setSelection(ids: string[]): void {
    this.selectedIds = new Set(ids);
    this.notify();
  }

  public onSelectionChange(cb: (selectedIds: string[]) => void): () => void {
    this.listeners.push(cb);
    return () => {
      this.listeners = this.listeners.filter(l => l !== cb);
    };
  }

  public onChange(cb: (selectedIds: string[]) => void): () => void {
    return this.onSelectionChange(cb);
  }

  private notify(): void {
    const list = this.getSelectedIds();
    for (const cb of this.listeners) {
      cb(list);
    }
  }

  /**
   * Determine which elements intersect the marquee rectangle (normalized 0..1).
   */
  public selectByMarquee(
    mx: number,
    my: number,
    mw: number,
    mh: number,
    elements: SlideElement[]
  ): string[] {
    const hitIds: string[] = [];
    const rx1 = Math.min(mx, mx + mw);
    const rx2 = Math.max(mx, mx + mw);
    const ry1 = Math.min(my, my + mh);
    const ry2 = Math.max(my, my + mh);

    for (const el of elements) {
      const ex1 = el.transform.x;
      const ex2 = el.transform.x + el.transform.w;
      const ey1 = el.transform.y;
      const ey2 = el.transform.y + el.transform.h;

      // Check AABB overlap
      const overlapX = ex1 < rx2 && ex2 > rx1;
      const overlapY = ey1 < ry2 && ey2 > ry1;
      if (overlapX && overlapY) {
        hitIds.push(el.id);
      }
    }

    this.selectedIds = new Set(hitIds);
    this.notify();
    return hitIds;
  }
}
