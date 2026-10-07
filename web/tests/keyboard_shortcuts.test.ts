import { describe, expect, test } from "bun:test";
import { resolveKeyboardShortcut, resolveTargetItemToDelete, UndoPlaceholderManager } from "../src/core/presentation_helpers.ts";

describe("Keyboard Shortcuts Matrix & Resolution Engine", () => {
    test("Ctrl+F resolves to search_focus and prevents default", () => {
        const res = resolveKeyboardShortcut({ key: "f", ctrlKey: true });
        expect(res).not.toBeNull();
        expect(res?.action).toBe("search_focus");
        expect(res?.preventDefault).toBe(true);
    });

    test("Escape inside input/textarea/select closes modals without swallowing Escape", () => {
        const inInput = resolveKeyboardShortcut({ key: "Escape", targetTagName: "INPUT" });
        expect(inInput?.action).toBe("close_modals");
        expect(inInput?.preventDefault).toBe(false);

        const inTextarea = resolveKeyboardShortcut({ key: "Escape", targetTagName: "TEXTAREA" });
        expect(inTextarea?.action).toBe("close_modals");

        // Typing normal letters inside input does NOT fire global shortcuts
        const letterInInput = resolveKeyboardShortcut({ key: "v", targetTagName: "INPUT" });
        expect(letterInInput).toBeNull();
    });

    test("Global shortcuts suppressed while document.body.classList.contains('editor-open')", () => {
        const ctrlZ = resolveKeyboardShortcut({ key: "z", ctrlKey: true, isEditorOpen: true });
        expect(ctrlZ).toBeNull();

        const f5 = resolveKeyboardShortcut({ key: "F5", isEditorOpen: true });
        expect(f5).toBeNull();

        const space = resolveKeyboardShortcut({ key: " ", isEditorOpen: true });
        expect(space).toBeNull();

        const esc = resolveKeyboardShortcut({ key: "Escape", isEditorOpen: true });
        expect(esc?.action).toBe("close_modals");
        expect(esc?.preventDefault).toBe(false);
    });

    test("Ctrl+Z triggers Undo", () => {
        const res = resolveKeyboardShortcut({ key: "z", ctrlKey: true, shiftKey: false });
        expect(res?.action).toBe("undo");
        expect(res?.command).toBe("Undo");
        expect(res?.preventDefault).toBe(true);
    });

    test("Ctrl+Y and Ctrl+Shift+Z trigger Redo symmetrically", () => {
        const ctrlY = resolveKeyboardShortcut({ key: "y", ctrlKey: true });
        expect(ctrlY?.action).toBe("redo");
        expect(ctrlY?.command).toBe("Redo");

        const ctrlShiftZ = resolveKeyboardShortcut({ key: "z", ctrlKey: true, shiftKey: true });
        expect(ctrlShiftZ?.action).toBe("redo");
        expect(ctrlShiftZ?.command).toBe("Redo");
    });

    test("F1 opens schedule guide modal", () => {
        const res = resolveKeyboardShortcut({ key: "F1" });
        expect(res?.action).toBe("guide");
        expect(res?.preventDefault).toBe(true);
    });

    test("? opens shortcuts cheat sheet modal", () => {
        const res = resolveKeyboardShortcut({ key: "?" });
        expect(res?.action).toBe("shortcuts");
        expect(res?.preventDefault).toBe(true);
    });

    test("F2 and Ctrl+, open options dialog symmetrically", () => {
        const f2 = resolveKeyboardShortcut({ key: "F2" });
        expect(f2?.action).toBe("options");

        const ctrlComma = resolveKeyboardShortcut({ key: ",", ctrlKey: true });
        expect(ctrlComma?.action).toBe("options");
    });

    test("F5 and Ctrl+B toggle blackout", () => {
        const f5 = resolveKeyboardShortcut({ key: "F5" });
        expect(f5?.action).toBe("blackout");
        expect(f5?.command).toBe("ToggleBlackout");

        const ctrlB = resolveKeyboardShortcut({ key: "b", ctrlKey: true });
        expect(ctrlB?.action).toBe("blackout");
        expect(ctrlB?.command).toBe("ToggleBlackout");
    });

    test("F6 and Ctrl+Shift+C toggle clear text", () => {
        const f6 = resolveKeyboardShortcut({ key: "F6" });
        expect(f6?.action).toBe("clear_text");
        expect(f6?.command).toBe("ToggleClearText");

        const ctrlShiftC = resolveKeyboardShortcut({ key: "c", ctrlKey: true, shiftKey: true });
        expect(ctrlShiftC?.action).toBe("clear_text");
        expect(ctrlShiftC?.command).toBe("ToggleClearText");
    });

    test("F7 and Ctrl+L toggle logo", () => {
        const f7 = resolveKeyboardShortcut({ key: "F7" });
        expect(f7?.action).toBe("logo");
        expect(f7?.command).toBe("ToggleLogo");

        const ctrlL = resolveKeyboardShortcut({ key: "l", ctrlKey: true });
        expect(ctrlL?.action).toBe("logo");
        expect(ctrlL?.command).toBe("ToggleLogo");
    });

    test("F8 opens alert broadcaster modal", () => {
        const res = resolveKeyboardShortcut({ key: "F8" });
        expect(res?.action).toBe("alert");
        expect(res?.preventDefault).toBe(true);
    });

    test("Enter and PageDown dispatch GoLive", () => {
        const enter = resolveKeyboardShortcut({ key: "Enter" });
        expect(enter?.action).toBe("go_live");
        expect(enter?.command).toEqual({ GoLive: { item_index: null, slide_index: null } });

        const pageDown = resolveKeyboardShortcut({ key: "PageDown" });
        expect(pageDown?.action).toBe("go_live");
        expect(pageDown?.command).toEqual({ GoLive: { item_index: null, slide_index: null } });
    });

    test("Space and ArrowRight dispatch NextSlide", () => {
        const space = resolveKeyboardShortcut({ key: " " });
        expect(space?.action).toBe("next_slide");
        expect(space?.command).toBe("NextSlide");

        const arrowRight = resolveKeyboardShortcut({ key: "ArrowRight" });
        expect(arrowRight?.action).toBe("next_slide");
        expect(arrowRight?.command).toBe("NextSlide");
    });

    test("ArrowLeft dispatches PrevSlide; Backspace is claimed by delete_selected_item instead", () => {
        // Backspace intentionally does NOT map to prev_slide (unlike most presentation
        // software) — it's bound to delete-selected-item-with-undo, matching the
        // behavior already shipped in the operator console. See the Delete/Backspace
        // test below.
        const arrowLeft = resolveKeyboardShortcut({ key: "ArrowLeft" });
        expect(arrowLeft?.action).toBe("prev_slide");
        expect(arrowLeft?.command).toBe("PrevSlide");
    });

    test("ArrowDown and ArrowUp navigate schedule items", () => {
        const down = resolveKeyboardShortcut({ key: "ArrowDown" });
        expect(down?.action).toBe("next_item");
        expect(down?.command).toBe("NextItem");

        const up = resolveKeyboardShortcut({ key: "ArrowUp" });
        expect(up?.action).toBe("prev_item");
        expect(up?.command).toBe("PrevItem");
    });

    test("Number keys 1..9 dispatch JumpSlide (0-indexed)", () => {
        const key1 = resolveKeyboardShortcut({ key: "1" });
        expect(key1?.action).toBe("jump_slide");
        expect(key1?.command).toEqual({ JumpSlide: 0 });

        const key5 = resolveKeyboardShortcut({ key: "5" });
        expect(key5?.command).toEqual({ JumpSlide: 4 });

        const key9 = resolveKeyboardShortcut({ key: "9" });
        expect(key9?.command).toEqual({ JumpSlide: 8 });
    });

    test("Liturgical keys dispatch JumpSection with uppercase code", () => {
        const v = resolveKeyboardShortcut({ key: "v" });
        expect(v?.action).toBe("jump_section");
        expect(v?.command).toEqual({ JumpSection: "V" });

        const c = resolveKeyboardShortcut({ key: "c" });
        expect(c?.command).toEqual({ JumpSection: "C" });

        const b = resolveKeyboardShortcut({ key: "B" });
        expect(b?.command).toEqual({ JumpSection: "B" });
    });

    test("Ctrl+N, Ctrl+O, Ctrl+S, Ctrl+I, and Ctrl+Shift+N handle schedule and modal actions", () => {
        const ctrlN = resolveKeyboardShortcut({ key: "n", ctrlKey: true, shiftKey: false });
        expect(ctrlN?.action).toBe("new_schedule");
        expect(ctrlN?.command).toBe("NewSchedule");

        const ctrlO = resolveKeyboardShortcut({ key: "o", ctrlKey: true });
        expect(ctrlO?.action).toBe("open_schedule");

        const ctrlS = resolveKeyboardShortcut({ key: "s", ctrlKey: true });
        expect(ctrlS?.action).toBe("save_schedule");

        // Ctrl+Shift+S must resolve to a distinct action from plain Ctrl+S --
        // previously both fell into "save_schedule", which app_ui.ts's
        // dispatch always routed to the Save As modal, so Ctrl+S alone never
        // actually did the silent quick-save its own doc comment promised.
        const ctrlShiftS = resolveKeyboardShortcut({ key: "s", ctrlKey: true, shiftKey: true });
        expect(ctrlShiftS?.action).toBe("save_schedule_as");
        expect(ctrlShiftS?.action).not.toBe(ctrlS?.action);

        const ctrlI = resolveKeyboardShortcut({ key: "i", ctrlKey: true });
        expect(ctrlI?.action).toBe("import_modal");

        const ctrlShiftN = resolveKeyboardShortcut({ key: "n", ctrlKey: true, shiftKey: true });
        expect(ctrlShiftN?.action).toBe("create_song");
    });

    test("Escape in normal document context triggers close_modals", () => {
        const esc = resolveKeyboardShortcut({ key: "Escape" });
        expect(esc?.action).toBe("close_modals");
    });

    test("Unbound key returns null", () => {
        const unbound = resolveKeyboardShortcut({ key: "x" });
        expect(unbound).toBeNull();
    });

    test("Delete and Backspace both resolve to delete_selected_item when outside inputs, null inside inputs", () => {
        const delGlobal = resolveKeyboardShortcut({ key: "Delete" });
        expect(delGlobal).not.toBeNull();
        expect(delGlobal?.action).toBe("delete_selected_item");
        expect(delGlobal?.preventDefault).toBe(true);

        const backspaceGlobal = resolveKeyboardShortcut({ key: "Backspace" });
        expect(backspaceGlobal?.action).toBe("delete_selected_item");
        expect(backspaceGlobal?.preventDefault).toBe(true);

        // Does NOT trigger inside input / textarea
        const delInInput = resolveKeyboardShortcut({ key: "Delete", targetTagName: "INPUT" });
        expect(delInInput).toBeNull();

        const delInTextarea = resolveKeyboardShortcut({ key: "Delete", targetTagName: "TEXTAREA" });
        expect(delInTextarea).toBeNull();

        // Backspace inside an input must still type/delete a character normally,
        // not trigger the app's delete-selected-item shortcut.
        const backspaceInInput = resolveKeyboardShortcut({ key: "Backspace", targetTagName: "INPUT" });
        expect(backspaceInInput).toBeNull();
    });

    test("contentEditable targets (rich text slide editing) suppress shortcuts like input/textarea do", () => {
        const letterInContentEditable = resolveKeyboardShortcut({ key: "v", isContentEditable: true });
        expect(letterInContentEditable).toBeNull();

        const backspaceInContentEditable = resolveKeyboardShortcut({ key: "Backspace", isContentEditable: true });
        expect(backspaceInContentEditable).toBeNull();

        const escInContentEditable = resolveKeyboardShortcut({ key: "Escape", isContentEditable: true });
        expect(escInContentEditable?.action).toBe("close_modals");
    });

    test("resolveTargetItemToDelete resolves selected item across schedule, preview, and live contexts", () => {
        const items = [
            { id: "item-1", title: "Opening Praise" },
            { id: "item-2", title: "Scripture Reading" },
            { id: "item-3", title: "Closing Benediction" }
        ];

        // 1. Schedule Context with selected item 1
        const schedTarget = resolveTargetItemToDelete({
            activeContext: "schedule",
            scheduleItems: items,
            selectedItemIndex: 1
        });
        expect(schedTarget).toEqual({ index: 1, title: "Scripture Reading" });

        // 2. Preview Context with staged item-3
        const prevTarget = resolveTargetItemToDelete({
            activeContext: "preview",
            scheduleItems: items,
            selectedItemIndex: 0,
            stagedItemId: "item-3"
        });
        expect(prevTarget).toEqual({ index: 2, title: "Closing Benediction" });

        // 3. Live Context with live item-1
        const liveTarget = resolveTargetItemToDelete({
            activeContext: "live",
            scheduleItems: items,
            selectedItemIndex: 2,
            liveItemId: "item-1"
        });
        expect(liveTarget).toEqual({ index: 0, title: "Opening Praise" });

        // 4. Fallback when active context item is not in schedule
        const fallbackTarget = resolveTargetItemToDelete({
            activeContext: "preview",
            scheduleItems: items,
            selectedItemIndex: 0,
            stagedItemId: "adhoc-item"
        });
        expect(fallbackTarget).toEqual({ index: 0, title: "Opening Praise" });

        // 5. Empty schedule returns null
        const emptyTarget = resolveTargetItemToDelete({
            activeContext: "schedule",
            scheduleItems: [],
            selectedItemIndex: 0
        });
        expect(emptyTarget).toBeNull();
    });

    test("UndoPlaceholderManager correctly tracks creation, expiration after 1.5s, and clearing", () => {
        const manager = new UndoPlaceholderManager(1500);
        expect(manager.get()).toBeNull();

        const created = manager.create(1, "Great Are You Lord", 1000);
        expect(created.index).toBe(1);
        expect(created.title).toBe("Great Are You Lord");
        expect(manager.get()?.title).toBe("Great Are You Lord");

        // After 1000ms (at t=2000), not yet expired (duration is 1500ms)
        expect(manager.isExpired(2000)).toBe(false);

        // After 1500ms (at t=2500), has expired
        expect(manager.isExpired(2500)).toBe(true);

        // Clear resets placeholder
        manager.clear();
        expect(manager.get()).toBeNull();
        expect(manager.isExpired(2500)).toBe(true);
    });
});
