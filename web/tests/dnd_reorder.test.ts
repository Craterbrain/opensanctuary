import { describe, expect, test } from "bun:test";
import { computeSafeDestinationIndex, ScheduleAddGuard, isPrimaryDropTarget } from "../src/core/presentation_helpers.ts";

describe("Pragmatic Drag and Drop Reordering Math", () => {
    test("reordering item downward moves past target when closestEdge is bottom", () => {
        const dest = computeSafeDestinationIndex(0, 2, 'bottom', 'vertical');
        expect(dest).toBe(2);
    });

    test("reordering item downward lands before target when closestEdge is top", () => {
        const dest = computeSafeDestinationIndex(0, 2, 'top', 'vertical');
        expect(dest).toBe(1);
    });

    test("reordering item upward moves before target when closestEdge is top", () => {
        const dest = computeSafeDestinationIndex(3, 1, 'top', 'vertical');
        expect(dest).toBe(1);
    });

    test("reordering item upward lands after target when closestEdge is bottom", () => {
        const dest = computeSafeDestinationIndex(3, 1, 'bottom', 'vertical');
        expect(dest).toBe(2);
    });

    test("immediate neighbor downward: dropping on far edge (bottom) swaps items", () => {
        // Dragging item 0 onto neighbor 1 bottom edge moves past item 1 -> destination 1 (swap)
        const dest = computeSafeDestinationIndex(0, 1, 'bottom', 'vertical');
        expect(dest).toBe(1);
    });

    test("immediate neighbor downward: dropping on near edge (top) is a no-op", () => {
        // Dragging item 0 onto neighbor 1 top edge leaves item 0 above item 1 -> destination 0 (no-op)
        const dest = computeSafeDestinationIndex(0, 1, 'top', 'vertical');
        expect(dest).toBe(0);
    });

    test("immediate neighbor upward: dropping on far edge (top) swaps items", () => {
        // Dragging item 2 onto neighbor 1 top edge moves before item 1 -> destination 1 (swap)
        const dest = computeSafeDestinationIndex(2, 1, 'top', 'vertical');
        expect(dest).toBe(1);
    });

    test("immediate neighbor upward: dropping on near edge (bottom) is a no-op", () => {
        // Dragging item 2 onto neighbor 1 bottom edge leaves item 2 below item 1 -> destination 2 (no-op)
        const dest = computeSafeDestinationIndex(2, 1, 'bottom', 'vertical');
        expect(dest).toBe(2);
    });

    test("horizontal matrix reordering across 2D grid cards", () => {
        const destRight = computeSafeDestinationIndex(0, 2, 'right', 'horizontal');
        expect(destRight).toBe(2);

        const destLeft = computeSafeDestinationIndex(3, 1, 'left', 'horizontal');
        expect(destLeft).toBe(1);
    });

    test("multi-directional edge normalization maps orthogonal edges cleanly", () => {
        // On vertical axis: 'right' normalizes to 'bottom', 'left' normalizes to 'top'
        expect(computeSafeDestinationIndex(0, 2, 'right', 'vertical')).toBe(2);
        expect(computeSafeDestinationIndex(0, 2, 'left', 'vertical')).toBe(1);

        // On horizontal axis: 'bottom' normalizes to 'right', 'top' normalizes to 'left'
        expect(computeSafeDestinationIndex(0, 2, 'bottom', 'horizontal')).toBe(2);
        expect(computeSafeDestinationIndex(0, 2, 'top', 'horizontal')).toBe(1);
    });

    test("ScheduleAddGuard permits initial item addition", () => {
        const guard = new ScheduleAddGuard(400);
        expect(guard.shouldAllow('song-1', 1000)).toBe(true);
    });

    test("ScheduleAddGuard blocks rapid duplicate additions of the same item within cooldown window", () => {
        const guard = new ScheduleAddGuard(400);
        expect(guard.shouldAllow('song-1', 1000)).toBe(true);
        // Rapid duplicate events (e.g. bubbling drop, multi-listener) fired within 50ms, 150ms, 350ms
        expect(guard.shouldAllow('song-1', 1050)).toBe(false);
        expect(guard.shouldAllow('song-1', 1150)).toBe(false);
        expect(guard.shouldAllow('song-1', 1399)).toBe(false);
    });

    test("ScheduleAddGuard allows the same item again after cooldown window expires", () => {
        const guard = new ScheduleAddGuard(400);
        expect(guard.shouldAllow('song-1', 1000)).toBe(true);
        expect(guard.shouldAllow('song-1', 1050)).toBe(false);
        // After 400ms window has passed (e.g. 450ms later)
        expect(guard.shouldAllow('song-1', 1450)).toBe(true);
    });

    test("ScheduleAddGuard allows different items to be added immediately without blocking", () => {
        const guard = new ScheduleAddGuard(400);
        expect(guard.shouldAllow('song-1', 1000)).toBe(true);
        expect(guard.shouldAllow('song-2', 1050)).toBe(true);
        expect(guard.shouldAllow('scripture-1', 1100)).toBe(true);
    });

    test("isPrimaryDropTarget prevents parent container double-processing when dropped on child", () => {
        const childItem = { id: "item-card" };
        const parentList = { id: "schedule-items-list" };

        // Location dropTargets are ordered [innermost, parent, root]
        const targets = [{ element: childItem }, { element: parentList }];

        // Child item IS the primary target -> handles drop
        expect(isPrimaryDropTarget(childItem, targets)).toBe(true);

        // Parent container IS NOT the primary target -> ignores drop (prevents triplication)
        expect(isPrimaryDropTarget(parentList, targets)).toBe(false);
    });

    test("isPrimaryDropTarget allows container when dropped directly into empty schedule", () => {
        const parentList = { id: "schedule-items-list" };
        const targets = [{ element: parentList }];

        // When dropped directly onto empty schedule, parentList IS the primary target
        expect(isPrimaryDropTarget(parentList, targets)).toBe(true);
    });
});
