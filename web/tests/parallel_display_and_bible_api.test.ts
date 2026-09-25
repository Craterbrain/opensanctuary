import { describe, expect, test } from "bun:test";
import {
    parseParallelSlide,
    formatParallelSlide,
    ParallelSlideContent,
    areTranslationsEquivalent
} from "../src/core/presentation_helpers.ts";

describe("Parallel Display Slide Parsing & Formatting", () => {
    test("parses parallel slide with delimiter and bracket headers", () => {
        const raw = "[KJV]\n1. In the beginning God created the heaven and the earth.\n|||\n[ESV]\n1. In the beginning, God created the heavens and the earth.";
        const parsed: ParallelSlideContent = parseParallelSlide(raw);

        expect(parsed.isParallel).toBe(true);
        expect(parsed.primaryHeader).toBe("KJV");
        expect(parsed.primaryText).toBe("1. In the beginning God created the heaven and the earth.");
        expect(parsed.secondaryHeader).toBe("ESV");
        expect(parsed.secondaryText).toBe("1. In the beginning, God created the heavens and the earth.");
    });

    test("parses parallel slide without bracket headers", () => {
        const raw = "Column Left Text\n|||\nColumn Right Text";
        const parsed = parseParallelSlide(raw);

        expect(parsed.isParallel).toBe(true);
        expect(parsed.primaryHeader).toBeUndefined();
        expect(parsed.primaryText).toBe("Column Left Text");
        expect(parsed.secondaryHeader).toBeUndefined();
        expect(parsed.secondaryText).toBe("Column Right Text");
    });

    test("returns isParallel false for standard single-column slide text", () => {
        const raw = "Amazing grace how sweet the sound\nThat saved a wretch like me";
        const parsed = parseParallelSlide(raw);

        expect(parsed.isParallel).toBe(false);
        expect(parsed.primaryText).toBe(raw);
        expect(parsed.secondaryText).toBe("");
        expect(parsed.primaryHeader).toBeUndefined();
        expect(parsed.secondaryHeader).toBeUndefined();
    });

    test("handles empty or whitespace-only slide text safely", () => {
        const emptyParsed = parseParallelSlide("");
        expect(emptyParsed.isParallel).toBe(false);
        expect(emptyParsed.primaryText).toBe("");

        const wsParsed = parseParallelSlide("   ");
        expect(wsParsed.isParallel).toBe(false);
        expect(wsParsed.primaryText).toBe("");
    });

    test("formats parallel slide and roundtrips with parseParallelSlide", () => {
        const formatted = formatParallelSlide(
            "KJV",
            "16. For God so loved the world...",
            "NIV",
            "16. For God so loved the world that he gave..."
        );

        expect(formatted).toContain("|||");
        expect(formatted).toContain("[KJV]");
        expect(formatted).toContain("[NIV]");

        const parsed = parseParallelSlide(formatted);
        expect(parsed.isParallel).toBe(true);
        expect(parsed.primaryHeader).toBe("KJV");
        expect(parsed.primaryText).toBe("16. For God so loved the world...");
        expect(parsed.secondaryHeader).toBe("NIV");
        expect(parsed.secondaryText).toBe("16. For God so loved the world that he gave...");
    });

    test("dual translation comparison threshold chunks 1-2 verses per slide", () => {
        // In dual column comparison, max 1 or 2 verses fit per slide to maintain legible font sizes
        const versesKJV = [
            { num: 1, text: "In the beginning God created the heaven and the earth." },
            { num: 2, text: "And the earth was without form, and void." },
            { num: 3, text: "And God said, Let there be light: and there was light." },
            { num: 4, text: "And God saw the light, that it was good." }
        ];

        const versesESV = [
            { num: 1, text: "In the beginning, God created the heavens and the earth." },
            { num: 2, text: "The earth was without form and void." },
            { num: 3, text: "And God said, \"Let there be light,\" and there was light." },
            { num: 4, text: "And God saw that the light was good." }
        ];

        const dualChunkSize = 2;
        const slides: string[] = [];

        for (let i = 0; i < versesKJV.length; i += dualChunkSize) {
            const chunkK = versesKJV.slice(i, i + dualChunkSize);
            const chunkE = versesESV.slice(i, i + dualChunkSize);

            const leftText = chunkK.map(v => `${v.num}. ${v.text}`).join("\n");
            const rightText = chunkE.map(v => `${v.num}. ${v.text}`).join("\n");

            slides.push(formatParallelSlide("KJV", leftText, "ESV", rightText));
        }

        expect(slides.length).toBe(2);

        // First slide has verses 1 & 2
        const slide1 = parseParallelSlide(slides[0]);
        expect(slide1.isParallel).toBe(true);
        expect(slide1.primaryText).toContain("1. In the beginning");
        expect(slide1.primaryText).toContain("2. And the earth");
        expect(slide1.secondaryText).toContain("1. In the beginning");
        expect(slide1.secondaryText).toContain("2. The earth");

        // Second slide has verses 3 & 4
        const slide2 = parseParallelSlide(slides[1]);
        expect(slide2.isParallel).toBe(true);
        expect(slide2.primaryText).toContain("3. And God said");
        expect(slide2.secondaryText).toContain("4. And God saw");
    });

    test("areTranslationsEquivalent correctly matches translations case-insensitively and with aliases", () => {
        // Direct case-insensitive matches
        expect(areTranslationsEquivalent("ASV", "asv")).toBe(true);
        expect(areTranslationsEquivalent("kjv", "KJV")).toBe(true);
        expect(areTranslationsEquivalent("ESV", "esv")).toBe(true);

        // Alias and long-form matching
        expect(areTranslationsEquivalent("American Standard Version", "asv")).toBe(true);
        expect(areTranslationsEquivalent("World English Bible", "WEB")).toBe(true);
        expect(areTranslationsEquivalent("web", "world-english-bible")).toBe(true);
        expect(areTranslationsEquivalent("King James Version", "kjv")).toBe(true);
        expect(areTranslationsEquivalent("Bible in Basic English", "BBE")).toBe(true);
        expect(areTranslationsEquivalent("Holman Christian Standard", "HCSB")).toBe(true);

        // Mismatches
        expect(areTranslationsEquivalent("KJV", "ESV")).toBe(false);
        expect(areTranslationsEquivalent("ASV", "NIV")).toBe(false);
        expect(areTranslationsEquivalent(null, "KJV")).toBe(false);
        expect(areTranslationsEquivalent("KJV", undefined)).toBe(false);

        // Red letter distinction
        expect(areTranslationsEquivalent("KJV", "KJV-RL")).toBe(false);
        expect(areTranslationsEquivalent("King James (Red Letter)", "KJV-RL")).toBe(true);
    });

    test("persisted default translation setting correctly marks default Bible installed item", () => {
        const mockInstalledBibles = [
            { id: "ASV", name: "American Standard Version", abbreviation: "ASV", isDefault: false },
            { id: "KJV", name: "King James Version", abbreviation: "KJV", isDefault: false },
            { id: "WEB", name: "World English Bible", abbreviation: "WEB", isDefault: false }
        ];

        // Simulate backend returning saved setting with different casing e.g. "asv"
        const savedDefaultFromSettings = "asv";

        // Synchronize as app_core.ts / loadAppOptions does:
        mockInstalledBibles.forEach(b => {
            b.isDefault = areTranslationsEquivalent(b.id, savedDefaultFromSettings) ||
                          areTranslationsEquivalent(b.abbreviation, savedDefaultFromSettings) ||
                          areTranslationsEquivalent(b.name, savedDefaultFromSettings);
        });

        expect(mockInstalledBibles[0].isDefault).toBe(true);
        expect(mockInstalledBibles[1].isDefault).toBe(false);
        expect(mockInstalledBibles[2].isDefault).toBe(false);

        // Update default setting to "World English Bible"
        const newSavedDefault = "World English Bible";
        mockInstalledBibles.forEach(b => {
            b.isDefault = areTranslationsEquivalent(b.id, newSavedDefault) ||
                          areTranslationsEquivalent(b.abbreviation, newSavedDefault) ||
                          areTranslationsEquivalent(b.name, newSavedDefault);
        });

        expect(mockInstalledBibles[0].isDefault).toBe(false);
        expect(mockInstalledBibles[1].isDefault).toBe(false);
        expect(mockInstalledBibles[2].isDefault).toBe(true);
    });
});
