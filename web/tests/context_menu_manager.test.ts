import { describe, test, expect, beforeEach } from 'bun:test';
import { ContextMenuManager } from '../src/ui/context_menu_manager';

describe('Universal Context Menu Manager: context_menu_manager.ts', () => {
  let cm: ContextMenuManager;
  let mockEl: HTMLElement;

  beforeEach(() => {
    cm = new ContextMenuManager();
    mockEl = {
      style: { display: 'none', left: '0px', top: '0px' } as any,
      offsetWidth: 200,
      offsetHeight: 240,
      getBoundingClientRect: () => ({ width: 200, height: 240, top: 0, left: 0, right: 200, bottom: 240 } as any),
      querySelectorAll: () => [],
      contains: () => false,
    } as any;
  });

  test('showElement displays menu and clamps to viewport bounds', () => {
    // Attempting to spawn near bottom-right corner of 1440x900 viewport
    const handled = cm.showElement(mockEl, 1430, 890);
    expect(handled).toBe(true);
    expect(mockEl.style.display).toBe('block');
    expect(cm.isAnyMenuOpen()).toBe(true);

    const left = parseInt(mockEl.style.left, 10);
    const top = parseInt(mockEl.style.top, 10);

    // With 200px width + 8px margin, max X should be 1440 - 200 - 8 = 1232
    expect(left).toBeLessThanOrEqual(1232);
    // With 240px height + 8px margin, max Y should be 900 - 240 - 8 = 652
    expect(top).toBeLessThanOrEqual(652);
  });

  test('showElement clamps negative coordinates to margin', () => {
    cm.showElement(mockEl, -50, -20);
    const left = parseInt(mockEl.style.left, 10);
    const top = parseInt(mockEl.style.top, 10);

    expect(left).toBeGreaterThanOrEqual(8);
    expect(top).toBeGreaterThanOrEqual(8);
  });

  test('hideAll closes active menu', () => {
    cm.showElement(mockEl, 100, 100);
    expect(cm.isAnyMenuOpen()).toBe(true);

    cm.hideAll();
    expect(mockEl.style.display).toBe('none');
    expect(cm.isAnyMenuOpen()).toBe(false);
  });

  test('showElement returns false for null/undefined elements', () => {
    expect(cm.showElement(null, 10, 10)).toBe(false);
    expect(cm.showElement(undefined, 10, 10)).toBe(false);
  });
});
