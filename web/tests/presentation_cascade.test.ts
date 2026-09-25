import { describe, expect, test } from "bun:test";
import {
    resolveSlideBackground,
    extractImageUrl,
    isImageBackground,
    isVideoBackground,
    isAudioMedia,
    formatCssBackground,
    type ThemeDefinition
} from "../src/core/presentation_helpers.ts";

const SAMPLE_THEMES: ThemeDefinition[] = [
    { name: "Midnight Ocean", bg: "linear-gradient(135deg, #0f2027, #203a43, #2c5364)" },
    { name: "Golden Sanctuary Light", bg: "linear-gradient(135deg, #fceabb, #f8b500)" },
    { name: "Cross Photo", bg: "/media/images/cross_sunset.jpg" }
];

describe("7-Tier Background Resolution Cascade", () => {
    test("Tier 0: Arrangement override wins over slide background", () => {
        const bg = resolveSlideBackground(
            { background: "/media/images/slide_custom.jpg" },
            { background: "/media/images/item_bg.jpg", theme_name: "Midnight Ocean" },
            SAMPLE_THEMES,
            { background_value: "#000000" },
            "Golden Sanctuary Light",
            undefined,
            "/media/images/arrangement_special.jpg"
        );
        expect(bg).toBe("/media/images/arrangement_special.jpg");
    });

    test("Tier 0: Arrangement override respects whitespace trimming", () => {
        const bgWhitespace = resolveSlideBackground(
            { background: "/media/images/slide_custom.jpg" },
            { background: "/media/images/item_bg.jpg", theme_name: "Midnight Ocean" },
            SAMPLE_THEMES,
            { background_value: "#000000" },
            "Golden Sanctuary Light",
            undefined,
            "   "
        );
        expect(bgWhitespace).toBe("/media/images/slide_custom.jpg");

        const bgTrimmed = resolveSlideBackground(
            { background: "/media/images/slide_custom.jpg" },
            { background: "/media/images/item_bg.jpg", theme_name: "Midnight Ocean" },
            SAMPLE_THEMES,
            { background_value: "#000000" },
            "Golden Sanctuary Light",
            undefined,
            "  /media/images/trimmed_special.jpg  "
        );
        expect(bgTrimmed).toBe("/media/images/trimmed_special.jpg");
    });

    test("Tier 1: Arrangement per-play override wins over slide, item, theme, and defaults", () => {
        const bg = resolveSlideBackground(
            { background: "/media/images/slide_custom.jpg" },
            { background: "/media/images/item_bg.jpg", theme_name: "Midnight Ocean" },
            SAMPLE_THEMES,
            { background_value: "#000000" },
            "Golden Sanctuary Light",
            undefined,
            "/media/images/arrangement_special.jpg"
        );
        expect(bg).toBe("/media/images/arrangement_special.jpg");
    });

    test("Tier 2: Slide background overrides item, themes, and defaults when no arrangement override", () => {
        const bg = resolveSlideBackground(
            { background: "/media/images/slide_custom.jpg" },
            { background: "/media/images/item_bg.jpg", theme_name: "Midnight Ocean" },
            SAMPLE_THEMES,
            { background_value: "#000000" },
            "Golden Sanctuary Light"
        );
        expect(bg).toBe("/media/images/slide_custom.jpg");
    });

    test("Tier 2: Item background is used when slide has no background override", () => {
        const bg = resolveSlideBackground(
            { background: null },
            { background: "/media/images/item_bg.jpg", theme_name: "Midnight Ocean" },
            SAMPLE_THEMES,
            { background_value: "#000000" },
            "Golden Sanctuary Light"
        );
        expect(bg).toBe("/media/images/item_bg.jpg");
    });

    test("Tier 3: Item theme is used when both slide and item backgrounds are null", () => {
        const bg = resolveSlideBackground(
            null,
            { background: null, theme_name: "Golden Sanctuary Light" },
            SAMPLE_THEMES,
            { background_value: "#000000" },
            "Midnight Ocean"
        );
        expect(bg).toBe("linear-gradient(135deg, #fceabb, #f8b500)");
    });

    test("Tier 4: Engine active theme is used when item has no specific theme", () => {
        const bg = resolveSlideBackground(
            null,
            { background: null, theme_name: null },
            SAMPLE_THEMES,
            { background_value: "radial-gradient(circle, #333, #111)" },
            "Golden Sanctuary Light"
        );
        expect(bg).toBe("radial-gradient(circle, #333, #111)");
    });

    test("Tier 5: Global fallback theme is used when active theme has no background value", () => {
        const bg = resolveSlideBackground(
            null,
            null,
            SAMPLE_THEMES,
            null,
            "Midnight Ocean"
        );
        expect(bg).toBe("linear-gradient(135deg, #0f2027, #203a43, #2c5364)");
    });

    test("Tier 6: Default gradient is returned when no tier specifies a background", () => {
        const bg = resolveSlideBackground(null, null, [], null, null);
        expect(bg).toBe("linear-gradient(135deg, #0f2027, #203a43, #2c5364)");
    });
});

describe("Media Detection & Background Formatting", () => {
    test("extractImageUrl correctly parses CSS url(...) wrappers and raw paths", () => {
        expect(extractImageUrl("url('/media/images/cross.jpg')")).toBe("/media/images/cross.jpg");
        expect(extractImageUrl('url("/backgrounds/stained_glass.png")')).toBe("/backgrounds/stained_glass.png");
        expect(extractImageUrl("photo.webp")).toBe("photo.webp");
        expect(extractImageUrl("data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==")).toBe("data:image/png;base64,iVBORw0KGgoAAAANSUhEUg==");
        expect(extractImageUrl("linear-gradient(to right, #000, #fff)")).toBeNull();
        expect(extractImageUrl("#123456")).toBeNull();
    });

    test("isImageBackground identifies valid image paths", () => {
        expect(isImageBackground("/media/images/nature.jpg")).toBe(true);
        expect(isImageBackground("bg.png")).toBe(true);
        expect(isImageBackground("video.mp4")).toBe(false);
        expect(isImageBackground(null)).toBe(false);
    });

    test("isVideoBackground correctly identifies common video extensions", () => {
        expect(isVideoBackground("/media/videos/countdown_5min.mp4")).toBe(true);
        expect(isVideoBackground("motion_loop.webm")).toBe(true);
        expect(isVideoBackground("clip.mov")).toBe(true);
        expect(isVideoBackground("slide.jpg")).toBe(false);
        expect(isVideoBackground("")).toBe(false);
    });

    test("isAudioMedia correctly identifies audio extensions", () => {
        expect(isAudioMedia("/media/audio/prelude_worship.mp3")).toBe(true);
        expect(isAudioMedia("choir_rehearsal.wav")).toBe(true);
        expect(isAudioMedia("track.m4a")).toBe(true);
        expect(isAudioMedia("track.flac")).toBe(true);
        expect(isAudioMedia("video.mp4")).toBe(false);
        expect(isAudioMedia("image.png")).toBe(false);
    });

    test("formatCssBackground formats images as cover and passes CSS gradients untouched", () => {
        const imgFormatted = formatCssBackground("/media/images/worship.jpg");
        expect(imgFormatted).toBe('url("/media/images/worship.jpg") center/cover no-repeat');

        const grad = "linear-gradient(45deg, #111, #444)";
        expect(formatCssBackground(grad)).toBe(grad);

        const hex = "#1a1a1a";
        expect(formatCssBackground(hex)).toBe(hex);

        expect(formatCssBackground(null)).toBe("linear-gradient(135deg, #0f2027, #203a43, #2c5364)");
    });
});
