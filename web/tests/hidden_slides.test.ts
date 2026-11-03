import { describe, expect, test } from "bun:test";
import { firstVisibleSlideIndex, hasShowableSlides } from "../src/core/presentation_helpers.ts";
import { resolveNextPresentationPreview } from "../src/core/presentation_sequence.ts";

const item = (id: string, slides: Array<{ text: string; hidden?: boolean }>, arrangement?: Array<{ section_id: string; source_slide_index: number }>) =>
    ({ id, title: id, slides, arrangement: arrangement ?? null }) as any;

describe("Hidden slides", () => {
    test("firstVisibleSlideIndex steps over hidden slides", () => {
        const it = item("a", [{ text: "0" }, { text: "1", hidden: true }, { text: "2", hidden: true }, { text: "3" }]);
        expect(firstVisibleSlideIndex(it, 0)).toBe(0);
        expect(firstVisibleSlideIndex(it, 1)).toBe(3);
        expect(firstVisibleSlideIndex(it, 4)).toBeNull();
    });

    test("an arrangement that repeats a hidden slide hides every repeat", () => {
        const it = item(
            "a",
            [{ text: "V1" }, { text: "C", hidden: true }],
            [
                { section_id: "V1", source_slide_index: 0 },
                { section_id: "C", source_slide_index: 1 },
                { section_id: "V1", source_slide_index: 0 },
                { section_id: "C", source_slide_index: 1 },
            ],
        );
        expect(firstVisibleSlideIndex(it, 1)).toBe(2);
        expect(firstVisibleSlideIndex(it, 3)).toBeNull();
    });

    test("an item with no slides is showable, an item with only hidden slides is not", () => {
        expect(hasShowableSlides(item("media", []))).toBe(true);
        expect(hasShowableSlides(item("all", [{ text: "x", hidden: true }]))).toBe(false);
        expect(hasShowableSlides(item("some", [{ text: "x", hidden: true }, { text: "y" }]))).toBe(true);
    });

    test("next-slide preview skips hidden slides", () => {
        const live = item("a", [{ text: "0" }, { text: "1", hidden: true }, { text: "2" }]);
        const preview = resolveNextPresentationPreview({ items: [live] }, live, 0);
        expect(preview.isNextSlide).toBe(true);
        expect(preview.text).toBe("2");
        expect(preview.label).toBe("Slide 3");
    });

    test("next-item preview skips fully hidden items and opens on the first visible slide", () => {
        const live = item("a", [{ text: "0" }]);
        const allHidden = item("b", [{ text: "b0", hidden: true }]);
        const partly = item("c", [{ text: "c0", hidden: true }, { text: "c1" }]);
        const preview = resolveNextPresentationPreview({ items: [live, allHidden, partly] }, live, 0);
        expect(preview.isNextScheduleItem).toBe(true);
        expect(preview.nextItem?.id).toBe("c");
        expect(preview.slide?.text).toBe("c1");
    });
});
