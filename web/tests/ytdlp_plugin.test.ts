import { test, expect, describe } from "bun:test";
// The plugin's pure parsing module (CommonJS, also loaded as a plain <script> in the sandbox).
// @ts-ignore
import YtDlp from "../plugins/ytdlp/ytdlp.js";

describe("Video Downloader plugin: progress lines", () => {
  test("reads percent, speed and ETA from a download line", () => {
    expect(YtDlp.parseProgressLine("[download]  45.2% of 12.34MiB at 3.45MiB/s ETA 00:02")).toEqual({
      percent: 45.2, speed: "3.45MiB/s", eta: "00:02", status: "Downloading: 45.2%" });
    expect(YtDlp.parseProgressLine("[download] 100% of 12.34MiB at 3.45MiB/s ETA 00:00")?.status).toBe("Merging video & audio format…");
    // Unknown speed/ETA are not shown as values.
    const early = YtDlp.parseProgressLine("[download]   0.0% of 1.00MiB at Unknown ETA Unknown");
    expect(early).toEqual({ percent: 0, status: "Downloading: 0.0%" });
  });

  test("recognises the post-processing stages and ignores noise", () => {
    expect(YtDlp.parseProgressLine("[SponsorBlock] Fetching SponsorBlock segments")?.status).toContain("SponsorBlock");
    for (const l of ["[ExtractAudio] Destination: x.mp3", "[Merger] Merging formats into x.webm", "[Fixup] Fixing something"]) {
      expect(YtDlp.parseProgressLine(l)?.status).toBe("Processing and finalizing media file…");
    }
    for (const l of ["", "   ", "[youtube] Extracting URL", "[download] Destination: x.webm", "WARNING: something"]) {
      expect(YtDlp.parseProgressLine(l)).toBeNull();
    }
  });
});

describe("Video Downloader plugin: the info line and the result file", () => {
  test("parses a --print-json line, preferring _filename", () => {
    const line = JSON.stringify({ id: "abc", title: "T", duration: 12.5, thumbnail: "https://i/t.jpg", _filename: "/job/T [abc].webm", filename: "/other", ext: "webm", formats: [{}] });
    expect(YtDlp.parseInfoLine(line)).toEqual({ id: "abc", title: "T", duration: 12.5, thumbnail: "https://i/t.jpg", filename: "/job/T [abc].webm", ext: "webm" });
    expect(YtDlp.parseInfoLine('{"id": 7}')).toMatchObject({ id: "7", title: null, duration: null });
    for (const bad of ["not json", "[1,2]", '{"broken":', "", "[download] 5%"]) expect(YtDlp.parseInfoLine(bad)).toBeNull();
  });

  const files = (...specs: Array<[string, number]>) => specs.map(([name, size]) => ({ name, size }));

  test("picks the file yt-dlp named, even when audio conversion changed its extension", () => {
    const info = { filename: "/job/Song [x1].webm" } as any;
    expect(YtDlp.chooseFile(files(["Song [x1].mp3", 100], ["Song [x1].webm.part", 900], ["notes.txt", 5]), info, true)?.name).toBe("Song [x1].mp3");
    expect(YtDlp.chooseFile(files(["Song [x1].webm", 100], ["other.mp4", 9999]), info, false)?.name).toBe("Song [x1].webm");
  });

  test("ignores leftovers and intermediates, and falls back to the largest media file", () => {
    expect(YtDlp.chooseFile(files(["a [1].f137.mp4", 5000], ["a [1].f251.webm", 3000], ["a [1].mp4", 100]), { filename: "/j/a [1].mp4" } as any, false)?.name).toBe("a [1].mp4");
    expect(YtDlp.chooseFile(files(["small.mp4", 10], ["big.mkv", 99]), null, false)?.name).toBe("big.mkv");
    expect(YtDlp.chooseFile(files(["x.mp3", 10]), null, false)).toBeNull(); // audio isn't a video result
    expect(YtDlp.chooseFile(files(["x.part", 10]), null, true)).toBeNull();
    expect(YtDlp.chooseFile([], null, false)).toBeNull();
    expect(YtDlp.chooseFile(undefined as any, null, false)).toBeNull();
  });
});

describe("Video Downloader plugin: cookies", () => {
  test("maps the choice to a browser the host will accept", () => {
    expect(YtDlp.cookieBrowser("auto")).toBe("chrome");
    expect(YtDlp.cookieBrowser("Firefox")).toBe("firefox");
    expect(YtDlp.cookieBrowser("none")).toBeNull();
    expect(YtDlp.cookieBrowser("")).toBeNull();
    expect(YtDlp.cookieBrowser("netscape")).toBeNull();
    expect(YtDlp.cookieBrowser("chrome; rm -rf /")).toBeNull();
  });
});
