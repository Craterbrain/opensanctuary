import { test, expect } from "bun:test";
import { stripLicensedText } from "../src/core/licensed";

const lic = { source: "youversion", bible_id: 111, usfm: "JHN.3.16", tag: "NIV", version: "NIV", book: "John", chapter: 3, resolved: true };

test("licensed passages keep their reference and lose their text; everything else is untouched", () => {
  const schedule = {
    title: "Sunday",
    items: [
      { id: "s1", title: "Amazing Grace", slides: [{ text: "Amazing grace" }] },
      { id: "sched_lic_1", title: "John 3", licensed: lic, slides: [{ text: "SECRET verse\n\nNIV", reference_label: "John 3:16 · NIV", elements: [{ type: "TextBlock" }], speaker_notes: "x" }] },
    ],
  };
  const out: any = stripLicensedText(schedule);
  expect(out.items[0]).toBe(schedule.items[0]);
  expect(JSON.stringify(out)).not.toContain("SECRET");
  expect(out.items[1].slides[0].text).toBe("John 3:16 · NIV — fetching the text…");
  expect(out.items[1].slides[0].elements).toEqual([]);
  expect(out.items[1].licensed).toMatchObject({ usfm: "JHN.3.16", bible_id: 111, tag: "NIV", resolved: false });
  expect(schedule.items[1].slides[0].text).toContain("SECRET"); // the original is not modified
});

test("a schedule with nothing licensed comes back as is", () => {
  const s = { items: [{ id: "a", slides: [{ text: "x" }] }] };
  expect(stripLicensedText(s)).toBe(s);
  expect(stripLicensedText(null as any)).toBe(null);
});
