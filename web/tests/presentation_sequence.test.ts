import { describe, test, expect } from "bun:test";
import {
  parseBilingualSlideText,
  resolveNextPresentationPreview
} from "../src/core/presentation_sequence.ts";

describe("Universal Presentation Sequencing: presentation_sequence.ts", () => {
  test("parseBilingualSlideText splits ||| into primary and secondary parts", () => {
    const single = parseBilingualSlideText("Amazing Grace how sweet the sound");
    expect(single.isBilingual).toBe(false);
    expect(single.primary).toBe("Amazing Grace how sweet the sound");
    expect(single.secondary).toBe("");

    const bilingual = parseBilingualSlideText("  Holy, Holy, Holy  |||  Santo, Santo, Santo  ");
    expect(bilingual.isBilingual).toBe(true);
    expect(bilingual.primary).toBe("Holy, Holy, Holy");
    expect(bilingual.secondary).toBe("Santo, Santo, Santo");

    const empty = parseBilingualSlideText(null);
    expect(empty.isBilingual).toBe(false);
    expect(empty.primary).toBe("");
  });

  test("resolveNextPresentationPreview returns next slide in active item", () => {
    const item = {
      id: "song-1",
      title: "How Great Thou Art",
      slides: [
        { label: "Verse 1", text: "O Lord, my God, when I in awesome wonder..." },
        { label: "Chorus", text: "Then sings my soul, my Savior God, to Thee..." }
      ]
    };

    const preview = resolveNextPresentationPreview({ items: [item] }, item, 0);
    expect(preview.isNextSlide).toBe(true);
    expect(preview.isNextScheduleItem).toBe(false);
    expect(preview.isEndOfSchedule).toBe(false);
    expect(preview.label).toBe("Chorus");
    expect(preview.text).toContain("Then sings my soul");
    expect(preview.bilingual.isBilingual).toBe(false);
  });

  test("resolveNextPresentationPreview rolls over to next schedule item at end of slides", () => {
    const item1 = {
      id: "song-1",
      title: "Opening Hymn",
      slides: [
        { label: "Verse 1", text: "Praise God from whom all blessings flow" }
      ]
    };

    const item2 = {
      id: "scripture-1",
      title: "Psalm 23",
      slides: [
        { label: "Slide 1", text: "The Lord is my shepherd; I shall not want." }
      ]
    };

    const preview = resolveNextPresentationPreview(
      { items: [item1, item2] },
      item1,
      0 // At last slide of item 1
    );

    expect(preview.isNextSlide).toBe(false);
    expect(preview.isNextScheduleItem).toBe(true);
    expect(preview.isEndOfSchedule).toBe(false);
    expect(preview.label).toBe("NEXT SERVICE ITEM");
    expect(preview.text).toContain("Next: Psalm 23");
    expect(preview.text).toContain("The Lord is my shepherd");
    expect(preview.nextItem?.id).toBe("scripture-1");
  });

  test("resolveNextPresentationPreview handles end of schedule", () => {
    const lastItem = {
      id: "benediction",
      title: "Closing Benediction",
      slides: [
        { label: "Benediction", text: "May the grace of the Lord Jesus Christ..." }
      ]
    };

    const preview = resolveNextPresentationPreview(
      { items: [lastItem] },
      lastItem,
      0 // Last slide of only item
    );

    expect(preview.isNextSlide).toBe(false);
    expect(preview.isNextScheduleItem).toBe(false);
    expect(preview.isEndOfSchedule).toBe(true);
    expect(preview.text).toBe("[ End of Service Schedule ]");
  });

  test("resolveNextPresentationPreview handles no active live item", () => {
    const preview = resolveNextPresentationPreview({ items: [] }, null, 0);
    expect(preview.isEndOfSchedule).toBe(true);
    expect(preview.text).toBe("");
  });
});
