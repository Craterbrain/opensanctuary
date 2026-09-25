import { describe, expect, test } from "bun:test";
import {
    effectiveArrangement,
    resolveSlideAt,
    resolveBackgroundAt,
    type ArrangementEntry
} from "../src/core/presentation_helpers.ts";

describe("Song Arrangement Resolution", () => {
    test("effectiveArrangement returns identity when arrangement is null or empty", () => {
        const item = {
            title: "Amazing Grace",
            slides: [
                { text: "Verse 1", label: "V1", tag: "V1" },
                { text: "Verse 2", label: "V2", tag: "V2" },
                { text: "Chorus", label: "C1", tag: "C1" },
            ],
            arrangement: null,
        };

        const arr = effectiveArrangement(item);
        expect(arr.length).toBe(3);
        expect(arr[0]).toEqual({ section_id: "V1", source_slide_index: 0, background_override: null });
        expect(arr[1]).toEqual({ section_id: "V2", source_slide_index: 1, background_override: null });
        expect(arr[2]).toEqual({ section_id: "C1", source_slide_index: 2, background_override: null });
    });

    test("effectiveArrangement generates fallback section_id for unlabeled slides", () => {
        const item = {
            title: "Unlabeled Song",
            slides: [
                { text: "First slide without label" },
                { text: "Second slide without label" },
            ],
            arrangement: null,
        };

        const arr = effectiveArrangement(item);
        expect(arr.length).toBe(2);
        expect(arr[0].section_id).toBe("V1");
        expect(arr[1].section_id).toBe("V2");
    });

    test("effectiveArrangement returns custom play order with repeated sections", () => {
        const customArr: ArrangementEntry[] = [
            { section_id: "V1", source_slide_index: 0, background_override: null },
            { section_id: "C1", source_slide_index: 2, background_override: null },
            { section_id: "V2", source_slide_index: 1, background_override: null },
            { section_id: "C1", source_slide_index: 2, background_override: null },
            { section_id: "E1", source_slide_index: 3, background_override: null },
        ];

        const item = {
            title: "Way Maker",
            slides: [
                { text: "Verse 1 content", label: "V1", tag: "V1" },
                { text: "Verse 2 content", label: "V2", tag: "V2" },
                { text: "Chorus content", label: "C1", tag: "C1" },
                { text: "Ending content", label: "E1", tag: "E1" },
            ],
            arrangement: customArr,
        };

        const arr = effectiveArrangement(item);
        expect(arr.length).toBe(5);
        expect(arr).toEqual(customArr);
    });

    test("resolveSlideAt maps play order to master slide content and preserves identity", () => {
        const item = {
            title: "10,000 Reasons",
            slides: [
                { text: "Chorus text", label: "C", tag: "C" },
                { text: "Verse 1 text", label: "V1", tag: "V1" },
                { text: "Verse 2 text", label: "V2", tag: "V2" },
            ],
            arrangement: [
                { section_id: "C", source_slide_index: 0 },
                { section_id: "V1", source_slide_index: 1 },
                { section_id: "C", source_slide_index: 0 },
                { section_id: "V2", source_slide_index: 2 },
                { section_id: "C", source_slide_index: 0 },
            ],
        };

        // Positions 0, 2, and 4 all map to master slide 0 (Chorus)
        const play0 = resolveSlideAt(item, 0);
        const play2 = resolveSlideAt(item, 2);
        const play4 = resolveSlideAt(item, 4);

        expect(play0?.text).toBe("Chorus text");
        expect(play2?.text).toBe("Chorus text");
        expect(play4?.text).toBe("Chorus text");

        // Position 1 maps to Verse 1
        expect(resolveSlideAt(item, 1)?.text).toBe("Verse 1 text");
        // Position 3 maps to Verse 2
        expect(resolveSlideAt(item, 3)?.text).toBe("Verse 2 text");
        // Out of bounds returns null
        expect(resolveSlideAt(item, 5)).toBeNull();
    });

    test("resolveSlideAt applies per-play background_override without mutating master slide", () => {
        const item = {
            title: "Build My Life",
            slides: [
                { text: "Verse 1", label: "V1", tag: "V1", background: "/media/images/standard.jpg" },
                { text: "Chorus", label: "C1", tag: "C1", background: "/media/images/standard.jpg" },
            ],
            arrangement: [
                { section_id: "V1", source_slide_index: 0 },
                { section_id: "C1", source_slide_index: 1, background_override: null },
                { section_id: "C1", source_slide_index: 1, background_override: "/media/videos/glorious_sunset.mp4" },
            ],
        };

        const play1 = resolveSlideAt(item, 1);
        const play2 = resolveSlideAt(item, 2);

        expect(play1?.background).toBe("/media/images/standard.jpg");
        expect(play2?.background).toBe("/media/videos/glorious_sunset.mp4");
        // Master slide remains untouched
        expect(item.slides[1].background).toBe("/media/images/standard.jpg");
    });

    test("resolveBackgroundAt resolves through 7-tier cascade", () => {
        const item = {
            title: "Test Song",
            background: "/media/images/item_default.jpg",
            slides: [
                { text: "Verse 1", background: "/media/images/slide_bg.jpg" },
                { text: "Chorus", background: null },
            ],
            arrangement: [
                { section_id: "V1", source_slide_index: 0, background_override: "/media/images/play_override.jpg" },
                { section_id: "C1", source_slide_index: 1, background_override: null },
            ],
        };

        // Play 0 has arrangement override -> wins over slide background
        const bg0 = resolveBackgroundAt(item, 0);
        expect(bg0).toBe("/media/images/play_override.jpg");

        // Play 1 has no arrangement override, no slide background -> falls back to item background
        const bg1 = resolveBackgroundAt(item, 1);
        expect(bg1).toBe("/media/images/item_default.jpg");
    });

    test("resolveSlideAt handles duplicate section_id entries sharing content", () => {
        const item = {
            title: "Cornerstone",
            slides: [
                { text: "Verse 1 lyrics", label: "V1", tag: "V1" },
                { text: "Chorus lyrics", label: "C1", tag: "C1" },
            ],
            arrangement: [
                { section_id: "V1", source_slide_index: 0 },
                { section_id: "C1", source_slide_index: 1 },
                { section_id: "C1", source_slide_index: 1 },
                { section_id: "V1", source_slide_index: 0 },
            ],
        };

        const play1 = resolveSlideAt(item, 1);
        const play2 = resolveSlideAt(item, 2);

        expect(play1?.text).toBe("Chorus lyrics");
        expect(play2?.text).toBe("Chorus lyrics");
        expect(play1?.tag).toBe("C1");
        expect(play2?.tag).toBe("C1");
    });
});

