import { test, expect, describe } from "bun:test";
// The plugin's pure module (CommonJS, also loaded as a plain <script> in the sandbox).
// @ts-ignore
import Stock from "../plugins/stock_media/stock.js";

describe("Stock Images plugin: requests", () => {
  test("Pexels sends the key in a header, never in the URL", () => {
    const r = Stock.pexelsRequest("sun rise", "KEY123");
    expect(r.url).toBe("https://api.pexels.com/v1/search?query=sun%20rise&per_page=24");
    expect(r.headers).toEqual({ Authorization: "KEY123" });
    expect(r.url).not.toContain("KEY123");
  });
  test("Pixabay puts the key in the query, asks for safe photos and trims the query to its limit", () => {
    const r = Stock.pixabayRequest("cross & light", "PXK");
    expect(r.url).toBe("https://pixabay.com/api/?key=PXK&q=cross%20%26%20light&image_type=photo&per_page=24&safesearch=true");
    expect(Stock.pixabayRequest("x".repeat(300), "k").url).toContain("q=" + "x".repeat(100) + "&");
  });
  test("no key or no query means no request", () => {
    expect(Stock.pexelsRequest("sun", "")).toBeNull();
    expect(Stock.pexelsRequest("   ", "k")).toBeNull();
    expect(Stock.pixabayRequest("sun", "  ")).toBeNull();
  });
});

describe("Stock Images plugin: results", () => {
  test("reads a Pexels answer, keeping the alt text as tags", () => {
    const hits = Stock.parsePexels(JSON.stringify({ photos: [{
      id: 42, width: 4000, height: 3000, url: "https://www.pexels.com/photo/42/", photographer: "Ann", photographer_url: "https://www.pexels.com/@ann",
      alt: "Sunrise over hills", src: { medium: "https://images.pexels.com/photos/42/m.jpeg", large2x: "https://images.pexels.com/photos/42/l2x.jpeg" } }] }));
    expect(hits).toHaveLength(1);
    expect(hits[0]).toMatchObject({ id: "pexels_42", provider: "Pexels", tags: "Sunrise over hills", author: "Ann", width: 4000,
      thumb: "https://images.pexels.com/photos/42/m.jpeg", full: "https://images.pexels.com/photos/42/l2x.jpeg" });
  });
  test("reads a Pixabay answer, keeping its tags", () => {
    const hits = Stock.parsePixabay({ hits: [{ id: 7, webformatURL: "https://pixabay.com/get/w.jpg", largeImageURL: "https://pixabay.com/get/l.jpg",
      imageWidth: 1920, imageHeight: 1080, user: "Bob", pageURL: "https://pixabay.com/photos/x-7/", tags: "church, window, light" }] });
    expect(hits[0]).toMatchObject({ id: "pixabay_7", provider: "Pixabay", tags: "church, window, light", author: "Bob" });
  });
  test("drops results whose pictures come from anywhere but the provider's own hosts", () => {
    const evil = { photos: [
      { id: 1, src: { medium: "https://evil.test/a.jpg", large2x: "https://images.pexels.com/x.jpg" } },
      { id: 2, src: { medium: "https://images.pexels.com/a.jpg", large2x: "http://images.pexels.com/x.jpg" } },
      { id: 3, src: { medium: "https://images.pexels.com.evil.test/a.jpg", large2x: "https://images.pexels.com/x.jpg" } },
      { id: 4, src: { medium: "https://user:pw@images.pexels.com/a.jpg", large2x: "https://images.pexels.com/x.jpg" } },
    ] };
    expect(Stock.parsePexels(evil)).toHaveLength(0);
    expect(Stock.parsePexels({})).toEqual([]);
    expect(() => Stock.parsePexels("not json")).toThrow();
  });
  test("strips control characters and markup-ish length from text", () => {
    expect(Stock.clean("a\u0000b\n\nc   d")).toBe("a b c d");
    expect(Stock.clean("x".repeat(500), 10)).toHaveLength(10);
  });
  test("interleaves providers so neither buries the other", () => {
    expect(Stock.interleave([[1, 2, 3], ["a"], []])).toEqual([1, "a", 2, 3]);
  });
  test("names a library item by keywords first, then credit", () => {
    expect(Stock.libraryName({ provider: "Pixabay", author: "Bob", tags: "church, window" })).toBe("church, window (Pixabay photo by Bob)");
    expect(Stock.libraryName({ provider: "Pexels", author: "Ann", tags: "" })).toBe("Pexels photo by Ann");
    expect(Stock.libraryName({ provider: "Pexels", author: "", tags: "t".repeat(80) })).toBe("t".repeat(60) + "… (Pexels photo)");
  });
  test("explains common failures", () => {
    expect(Stock.explainStatus("Pexels", 401)).toContain("did not accept that API key");
    expect(Stock.explainStatus("Pixabay", 429)).toContain("too many searches");
    expect(Stock.explainStatus("Pixabay", 400)).toContain("Check the API key");
    expect(Stock.explainStatus("Pixabay", 500)).toBe("Pixabay answered 500.");
  });
});
