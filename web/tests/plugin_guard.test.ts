import { test, expect, describe } from "bun:test";
import { previousSessionHung, markPluginsStarting, clearPluginMarker, silentFrames, SILENT_LIMIT_MS, type GuardStorage } from "../src/core/plugin_guard.ts";

function memory(): GuardStorage & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return { data, getItem: (k) => data.get(k) ?? null, setItem: (k, v) => void data.set(k, v), removeItem: (k) => void data.delete(k) };
}

describe("plugin guard: safe mode marker", () => {
  test("a clean start has no marker", () => {
    expect(previousSessionHung(Date.now(), memory())).toBe(false);
  });
  test("a marker left behind means the last session hung", () => {
    const s = memory();
    markPluginsStarting(1_000_000, s);
    expect(previousSessionHung(1_005_000, s)).toBe(true);
  });
  test("a cleared marker (clean unload, or healthy for a while) is not a hang", () => {
    const s = memory();
    markPluginsStarting(1_000_000, s);
    clearPluginMarker(s);
    expect(previousSessionHung(1_005_000, s)).toBe(false);
  });
  test("an old marker is stale and ignored", () => {
    const s = memory();
    markPluginsStarting(1_000_000, s);
    expect(previousSessionHung(1_000_000 + 16 * 60 * 1000, s)).toBe(false);
  });
  test("garbage, a future time, and blocked storage never throw or pause plugins", () => {
    const s = memory();
    s.setItem("os.pluginGuard.pending", "not a number");
    expect(previousSessionHung(1, s)).toBe(false);
    markPluginsStarting(9_000_000, s);
    expect(previousSessionHung(1_000, s)).toBe(false);
    const broken: GuardStorage = { getItem() { throw new Error("blocked"); }, setItem() { throw new Error("blocked"); }, removeItem() { throw new Error("blocked"); } };
    expect(previousSessionHung(1, broken)).toBe(false);
    expect(() => { markPluginsStarting(1, broken); clearPluginMarker(broken); }).not.toThrow();
    expect(previousSessionHung(1, null)).toBe(false);
  });
});

describe("plugin guard: watchdog", () => {
  const f = (id: string, alive: number, everAlive: boolean) => ({ id, alive, everAlive });
  test("only a frame that answered before and then went quiet is flagged", () => {
    const now = 100_000;
    const out = silentFrames([f("quiet", now - SILENT_LIMIT_MS - 1, true), f("fine", now - 1000, true), f("never_answered", 0, false)], now, false);
    expect(out.map((x) => x.id)).toEqual(["quiet"]);
  });
  test("nobody is blamed while the console itself was stalled", () => {
    expect(silentFrames([f("quiet", 0, true)], 100_000, true)).toEqual([]);
  });
});
