import { describe, expect, test } from "bun:test";
import { getSlideBadge, getSlideBadgeColor } from "../src/core/presentation_helpers.ts";

describe("Slide Badge & Verse Number Preservation", () => {
    test("extracts uppercase verse badges from scripture headers and labels", () => {
        expect(getSlideBadge({ header: "Genesis 1:1", label: "v1" })).toBe("V1");
        expect(getSlideBadge({ header: "Genesis 1:12", label: "v12" })).toBe("V12");
        expect(getSlideBadge({ header: "John 3:16", label: "v16" })).toBe("V16");
        expect(getSlideBadge({ header: "Psalm 23:1" })).toBe("V1");
        expect(getSlideBadge({ label: "v7" })).toBe("V7");
    });

    test("extracts standard badges from song sections (Verses, Choruses, Bridges, Endings)", () => {
        expect(getSlideBadge({ label: "Verse 1" })).toBe("V1");
        expect(getSlideBadge({ label: "Verse 3" })).toBe("V3");
        expect(getSlideBadge({ label: "Chorus" })).toBe("C");
        expect(getSlideBadge({ label: "Chorus 2" })).toBe("C2");
        expect(getSlideBadge({ label: "Bridge" })).toBe("B");
        expect(getSlideBadge({ label: "Bridge 1" })).toBe("B1");
        expect(getSlideBadge({ label: "Ending" })).toBe("E");
        expect(getSlideBadge({ label: "Outro" })).toBe("E");
    });

    test("assigns badge color based on section type", () => {
        expect(getSlideBadgeColor("V1")).toBe("var(--badge-verse)");
        expect(getSlideBadgeColor("V12")).toBe("var(--badge-verse)");
        expect(getSlideBadgeColor("C1")).toBe("var(--badge-chorus)");
        expect(getSlideBadgeColor("C")).toBe("var(--badge-chorus)");
        expect(getSlideBadgeColor("B")).toBe("var(--badge-bridge)");
        expect(getSlideBadgeColor("E")).toBe("var(--badge-ending)");
    });

    test("reordering slides NEVER changes verse numbers or badges", () => {
        // Create 3 scripture slides
        const slide1 = { text: "In the beginning God created the heaven and the earth.", header: "Genesis 1:1", label: "V1" };
        const slide2 = { text: "And the earth was without form, and void...", header: "Genesis 1:2", label: "V2" };
        const slide3 = { text: "And God said, Let there be light: and there was light.", header: "Genesis 1:3", label: "V3" };

        const slides = [slide1, slide2, slide3];

        // Verify initial badges
        expect(getSlideBadge(slides[0], 0)).toBe("V1");
        expect(getSlideBadge(slides[1], 1)).toBe("V2");
        expect(getSlideBadge(slides[2], 2)).toBe("V3");

        // Reorder: Move Slide 3 (Genesis 1:3) to the first position (index 0)
        const reordered = [slides[2], slides[0], slides[1]];

        // Crucial test: Slide 3 at index 0 MUST still be V3, not V1!
        expect(getSlideBadge(reordered[0], 0)).toBe("V3");
        expect(getSlideBadge(reordered[1], 1)).toBe("V1");
        expect(getSlideBadge(reordered[2], 2)).toBe("V2");

        // Move Slide 2 (V2) to position 0: [V2, V3, V1]
        const reorderedAgain = [reordered[2], reordered[0], reordered[1]];
        expect(getSlideBadge(reorderedAgain[0], 0)).toBe("V2");
        expect(getSlideBadge(reorderedAgain[1], 1)).toBe("V3");
        expect(getSlideBadge(reorderedAgain[2], 2)).toBe("V1");
    });

    test("unlabeled slides receive permanent assigned tags so subsequent reordering preserves them", () => {
        const slideA: any = { text: "First generic slide without label" };
        const slideB: any = { text: "Second generic slide without label" };

        // Initial rendering assigns permanent tags
        expect(getSlideBadge(slideA, 0)).toBe("V1");
        expect(getSlideBadge(slideB, 1)).toBe("V2");
        expect(slideA.tag).toBe("V1");
        expect(slideB.tag).toBe("V2");

        // Now swap order
        const swapped = [slideB, slideA];
        // Slide B at index 0 MUST retain its assigned V2 tag!
        expect(getSlideBadge(swapped[0], 0)).toBe("V2");
        // Slide A at index 1 MUST retain its assigned V1 tag!
        expect(getSlideBadge(swapped[1], 1)).toBe("V1");
    });

    test("extracts verse from leading verse number in slide text when header/label are missing", () => {
        const slideWithVersePrefix = { text: "16 For God so loved the world that he gave..." };
        expect(getSlideBadge(slideWithVersePrefix)).toBe("V16");

        const slideWithDotPrefix = { text: "5. Trust in the LORD with all your heart..." };
        expect(getSlideBadge(slideWithDotPrefix)).toBe("V5");
    });
});

