import { describe, test, expect, beforeEach } from 'bun:test';
import {
  DialogManager,
} from '../src/ui/dialog_manager';

describe('Unified Dialog & Modal Manager: dialog_manager.ts', () => {
  let dm: DialogManager;
  let mockModal1: HTMLElement;
  let mockModal2: HTMLElement;
  let confirmModal: HTMLElement;

  beforeEach(() => {
    dm = new DialogManager();

    mockModal1 = {
      id: 'test-modal-1',
      style: { display: 'none', zIndex: '9000' } as any,
      querySelector: () => null,
    } as any;

    mockModal2 = {
      id: 'test-modal-2',
      style: { display: 'none', zIndex: '9000' } as any,
      querySelector: () => null,
    } as any;

    confirmModal = {
      id: 'confirm-modal',
      style: { display: 'none', zIndex: '9000' } as any,
      querySelector: () => null,
    } as any;
  });

  test('openModal sets display flex and tracks in stack', () => {
    dm.openModal(mockModal1);
    expect(mockModal1.style.display).toBe('flex');
    expect(dm.isAnyModalOpen()).toBe(true);
    expect(dm.getTopmostModal()).toBe(mockModal1);
  });

  test('opening multiple modals stacks them and adjusts z-index', () => {
    dm.openModal(mockModal1);
    dm.openModal(mockModal2);

    expect(mockModal1.style.display).toBe('flex');
    expect(mockModal2.style.display).toBe('flex');
    expect(dm.getTopmostModal()).toBe(mockModal2);
    // Modal 2 was opened over Modal 1 -> stacked higher
    expect(parseInt(mockModal2.style.zIndex, 10)).toBeGreaterThan(parseInt(mockModal1.style.zIndex, 10));
  });

  test('confirm modal automatically receives high z-index (10000)', () => {
    dm.openModal(confirmModal);
    expect(confirmModal.style.zIndex).toBe('10000');
  });

  test('closeModal hides dialog, pops from stack, and triggers reset hook', () => {
    let resetFired = false;
    dm.registerResetHook('test-modal-1', () => {
      resetFired = true;
    });

    dm.openModal(mockModal1);
    expect(dm.isAnyModalOpen()).toBe(true);

    dm.closeModal(mockModal1);
    expect(mockModal1.style.display).toBe('none');
    expect(dm.isAnyModalOpen()).toBe(false);
    expect(resetFired).toBe(true);
  });

  test('closeTopmostModal closes only the topmost dialog in a layered stack', () => {
    dm.openModal(mockModal1);
    dm.openModal(mockModal2);

    expect(dm.getTopmostModal()).toBe(mockModal2);

    // First dismiss: only closes modal 2
    const handled1 = dm.closeTopmostModal();
    expect(handled1).toBe(true);
    expect(mockModal2.style.display).toBe('none');
    expect(mockModal1.style.display).toBe('flex');
    expect(dm.getTopmostModal()).toBe(mockModal1);

    // Second dismiss: closes modal 1
    const handled2 = dm.closeTopmostModal();
    expect(handled2).toBe(true);
    expect(mockModal1.style.display).toBe('none');
    expect(dm.isAnyModalOpen()).toBe(false);

    // Third dismiss: nothing left to close
    const handled3 = dm.closeTopmostModal();
    expect(handled3).toBe(false);
  });
});
