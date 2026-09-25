import { test, expect, describe } from "bun:test";
import { formatMediaTime, debounce, copyToClipboard, readClipboard, showToast } from "../src/core/ui_utils";

describe("Core UI Utilities: formatMediaTime", () => {
  test("formats 0 seconds as 0:00", () => {
    expect(formatMediaTime(0)).toBe("0:00");
  });

  test("formats seconds under one minute with leading zero", () => {
    expect(formatMediaTime(5)).toBe("0:05");
    expect(formatMediaTime(42)).toBe("0:42");
    expect(formatMediaTime(59)).toBe("0:59");
  });

  test("formats multi-minute timestamps accurately", () => {
    expect(formatMediaTime(60)).toBe("1:00");
    expect(formatMediaTime(65)).toBe("1:05");
    expect(formatMediaTime(125)).toBe("2:05");
    expect(formatMediaTime(754)).toBe("12:34");
    expect(formatMediaTime(3600)).toBe("60:00");
  });

  test("handles negative numbers, NaN, null, and undefined safely", () => {
    expect(formatMediaTime(-10)).toBe("0:00");
    expect(formatMediaTime(NaN)).toBe("0:00");
    expect(formatMediaTime(null)).toBe("0:00");
    expect(formatMediaTime(undefined)).toBe("0:00");
  });
});

describe("Core UI Utilities: debounce", () => {
  test("delays function execution until delayMs elapses", async () => {
    let callCount = 0;
    let lastArg = "";
    const fn = debounce((arg: string) => {
      callCount++;
      lastArg = arg;
    }, 50);

    fn("a");
    expect(callCount).toBe(0);

    await new Promise(r => setTimeout(r, 70));
    expect(callCount).toBe(1);
    expect(lastArg).toBe("a");
  });

  test("batches rapid consecutive invocations into single trailing execution", async () => {
    let callCount = 0;
    let lastVal = 0;
    const fn = debounce((v: number) => {
      callCount++;
      lastVal = v;
    }, 50);

    fn(1);
    fn(2);
    fn(3);
    expect(callCount).toBe(0);

    await new Promise(r => setTimeout(r, 70));
    expect(callCount).toBe(1);
    expect(lastVal).toBe(3);
  });

  test("cancel() prevents pending invocation", async () => {
    let called = false;
    const fn = debounce(() => {
      called = true;
    }, 50);

    fn();
    fn.cancel();

    await new Promise(r => setTimeout(r, 70));
    expect(called).toBe(false);
  });
});

describe("Core UI Utilities: copyToClipboard & readClipboard", () => {
  test("handles environment without window/document gracefully", async () => {
    const result = await copyToClipboard("test content");
    expect(typeof result).toBe("boolean");
    const read = await readClipboard();
    expect(typeof read).toBe("string");
  });
});

describe("Core UI Utilities: showToast", () => {
  test("returns null safely when running outside DOM environment", () => {
    if (typeof document === "undefined") {
      const toast = showToast("Test message", "info");
      expect(toast).toBeNull();
    }
  });
});
