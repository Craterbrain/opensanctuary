import { test, expect, describe } from "bun:test";
import {
  isSupportedScheduleFile,
  isBinaryScheduleFile,
  fileToBase64
} from "../src/core/schedule_drop";

describe("Schedule Drop Utilities", () => {
  test("isSupportedScheduleFile correctly identifies valid schedule formats", () => {
    expect(isSupportedScheduleFile("service.ewsx")).toBe(true);
    expect(isSupportedScheduleFile("SERVICE.EWSX")).toBe(true);
    expect(isSupportedScheduleFile("legacy.ews")).toBe(true);
    expect(isSupportedScheduleFile("service.osj")).toBe(true);
    expect(isSupportedScheduleFile("bundle.osz")).toBe(true);
    expect(isSupportedScheduleFile("BUNDLE.OSZ")).toBe(true);
    expect(isSupportedScheduleFile("export.json")).toBe(true);

    expect(isSupportedScheduleFile("image.png")).toBe(false);
    expect(isSupportedScheduleFile("song.txt")).toBe(false);
    expect(isSupportedScheduleFile("video.mp4")).toBe(false);
  });

  test("isBinaryScheduleFile correctly discriminates binary archives", () => {
    expect(isBinaryScheduleFile("service.ewsx")).toBe(true);
    expect(isBinaryScheduleFile("legacy.ews")).toBe(true);
    expect(isBinaryScheduleFile("bundle.osz")).toBe(true);

    expect(isBinaryScheduleFile("service.osj")).toBe(false);
    expect(isBinaryScheduleFile("export.json")).toBe(false);
  });

  test("fileToBase64 converts binary bytes into valid Base64 string", async () => {
    const rawData = new Uint8Array([72, 101, 108, 108, 111, 32, 87, 111, 114, 108, 100]); // "Hello World"
    const file = new File([rawData], "test.ewsx", { type: "application/octet-stream" });

    const b64 = await fileToBase64(file);
    expect(b64).toBe(btoa("Hello World"));
  });
});
