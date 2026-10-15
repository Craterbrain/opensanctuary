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

import { LoadShedder, LAG_MS, type ShedAction } from "../src/core/plugin_guard.ts";

/** Simulates a console whose lag depends on which plugins are running. */
function simulate(plugins: string[], culprits: string[], seconds: number) {
  const s = new LoadShedder();
  const running = [...plugins];
  const log: ShedAction[] = [];
  for (let t = 0; t < seconds; t++) {
    const lagged = running.some((p) => culprits.includes(p));
    for (const a of s.tick(lagged ? LAG_MS * 3 : 5, [...running])) {
      log.push(a);
      if (a.kind === "suspend") running.splice(running.indexOf(a.id), 1);
      if (a.kind === "restore") for (const id of a.ids) running.push(id);
    }
  }
  return { running, log };
}

describe("plugin guard: load shedding", () => {
  test("a healthy console never switches anything off", () => {
    const { running, log } = simulate(["a", "b", "c"], [], 120);
    expect(log).toEqual([]);
    expect(running.sort()).toEqual(["a", "b", "c"]);
  });
  test("the plugin whose removal clears the lag is blamed; the others are started again", () => {
    const { running, log } = simulate(["good1", "bad", "good2"], ["bad"], 60);
    expect(log.filter((a) => a.kind === "blame")).toEqual([{ kind: "blame", id: "bad" }]);
    expect(running.sort()).toEqual(["good1", "good2"]);
    // Every innocent that was tested comes back, and nothing is blamed twice.
    const restored = log.filter((a): a is Extract<ShedAction, { kind: "restore" }> => a.kind === "restore").flatMap((a) => a.ids);
    expect(restored.every((id) => id !== "bad")).toBe(true);
  });
  test("a lone culprit is found", () => {
    const { running, log } = simulate(["bad"], ["bad"], 30);
    expect(running).toEqual([]);
    expect(log.map((a) => a.kind)).toEqual(["suspend", "blame"]);
  });
  test("two culprits are both found, one after the other", () => {
    const { running } = simulate(["bad1", "ok", "bad2"], ["bad1", "bad2"], 200);
    expect(running).toEqual(["ok"]);
  });
  test("lag the plugins didn't cause: everything is put back and left alone for a while", () => {
    const s = new LoadShedder();
    const running = ["a", "b"];
    let suspendedNow: string[] = [];
    const actions: ShedAction[] = [];
    for (let t = 0; t < 200; t++) {
      for (const a of s.tick(LAG_MS * 3, running.filter((p) => !suspendedNow.includes(p)))) {
        actions.push(a);
        if (a.kind === "suspend") suspendedNow.push(a.id);
        if (a.kind === "restore") suspendedNow = suspendedNow.filter((id) => !a.ids.includes(id));
      }
    }
    expect(actions.some((a) => a.kind === "blame")).toBe(false);
    expect(suspendedNow).toEqual([]); // not left switched off
    // Not a restart loop: a few rounds in 200 s, not one every few seconds.
    expect(actions.filter((a) => a.kind === "suspend").length).toBeLessThanOrEqual(8);
  });
  test("a single slow tick is not lag", () => {
    const s = new LoadShedder();
    const acts = [LAG_MS * 5, 5, 5, 5, LAG_MS * 5, 5, 5, 5].flatMap((d) => s.tick(d, ["a"]));
    expect(acts).toEqual([]);
  });
});
