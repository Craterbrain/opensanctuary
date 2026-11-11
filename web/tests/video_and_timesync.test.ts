import { describe, expect, test } from "bun:test";
import { computeVideoSyncDecision, projectedMediaPosition, wrapLoopDrift } from "../src/core/video_sync.ts";
import { computeMedianOffset } from "../src/core/timesync.ts";

describe("VideoSyncController 3-Tier Drift Algorithm", () => {
    test("Tier 1 Deadband: drift within [-75ms, +75ms] keeps rate at 1.0x without seeking", () => {
        // Zero drift
        const d0 = computeVideoSyncDecision(10.0, 10.0);
        expect(d0.tier).toBe(1);
        expect(d0.action).toBe('deadband');
        expect(d0.playbackRate).toBe(1.0);
        expect(d0.targetTime).toBeUndefined();

        // +50ms drift (video slightly behind, but within imperceptible deadband)
        const d50 = computeVideoSyncDecision(10.050, 10.000);
        expect(d50.tier).toBe(1);
        expect(d50.action).toBe('deadband');
        expect(d50.playbackRate).toBe(1.0);

        // -70ms drift (video slightly ahead, within deadband)
        const dNeg70 = computeVideoSyncDecision(9.930, 10.000);
        expect(dNeg70.tier).toBe(1);
        expect(dNeg70.action).toBe('deadband');
        expect(dNeg70.playbackRate).toBe(1.0);
    });

    test("Tier 2 Micro-Slew: drift between 75ms and 500ms smoothly scales playback rate", () => {
        // Video 200ms behind -> drift is +200ms -> rate should increase above 1.0x
        const d200 = computeVideoSyncDecision(10.200, 10.000);
        expect(d200.tier).toBe(2);
        expect(d200.action).toBe('slew');
        expect(d200.playbackRate).toBeGreaterThan(1.0);
        expect(d200.playbackRate).toBeLessThanOrEqual(1.05);
        expect(d200.targetTime).toBeUndefined();

        // Video 200ms ahead -> drift is -200ms -> rate should decrease below 1.0x
        const dNeg200 = computeVideoSyncDecision(9.800, 10.000);
        expect(dNeg200.tier).toBe(2);
        expect(dNeg200.action).toBe('slew');
        expect(dNeg200.playbackRate).toBeLessThan(1.0);
        expect(dNeg200.playbackRate).toBeGreaterThanOrEqual(0.95);
    });

    test("Tier 2 Clamping: rate never exceeds 1.05x or drops below 0.95x to prevent pitch distortion", () => {
        // At 499ms drift
        const d499 = computeVideoSyncDecision(10.499, 10.000);
        expect(d499.tier).toBe(2);
        expect(d499.playbackRate).toBeLessThanOrEqual(1.05);

        // At -499ms drift
        const dNeg499 = computeVideoSyncDecision(9.501, 10.000);
        expect(dNeg499.tier).toBe(2);
        expect(dNeg499.playbackRate).toBeGreaterThanOrEqual(0.95);
    });

    test("Tier 3 Hard Seek: drift exceeding 500ms triggers immediate seek to target PTS", () => {
        // Video 1200ms behind -> hard seek
        const d1200 = computeVideoSyncDecision(11.200, 10.000);
        expect(d1200.tier).toBe(3);
        expect(d1200.action).toBe('seek');
        expect(d1200.targetTime).toBe(11.200);
        expect(d1200.playbackRate).toBe(1.0);

        // Video 2000ms ahead -> hard seek
        const dNeg2000 = computeVideoSyncDecision(8.000, 10.000);
        expect(dNeg2000.tier).toBe(3);
        expect(dNeg2000.action).toBe('seek');
        expect(dNeg2000.targetTime).toBe(8.000);
        expect(dNeg2000.playbackRate).toBe(1.0);
    });
});

describe("NTP TimeSync Median Offset Calculation", () => {
    test("empty samples return zero offset", () => {
        expect(computeMedianOffset([])).toBe(0);
    });

    test("single sample returns exact offset", () => {
        expect(computeMedianOffset([42])).toBe(42);
    });

    test("odd count returns true median sample", () => {
        // [10, 12, 14, 50, 120] -> sorted median is 14
        const samples = [50, 10, 120, 12, 14];
        expect(computeMedianOffset(samples)).toBe(14);
    });

    test("filters out high-latency network spikes and outliers", () => {
        // Typical NTP ping series with one severe network spike (+950ms)
        const pings = [18, 19, 950, 21, 17];
        const median = computeMedianOffset(pings);
        expect(median).toBe(19);
    });

    test("handles negative offsets when local clock is ahead of server", () => {
        const pings = [-45, -50, -42, -500, -48];
        const median = computeMedianOffset(pings);
        expect(median).toBe(-48);
    });
});

describe("Looping video drift around the wrap", () => {
    test("just past the wrap, a player finishing the last loop is slightly behind, not a loop ahead", () => {
        // Expected 0.1s into the next loop of a 30s video; the player is at 29.95s.
        const d = computeVideoSyncDecision(0.1, 29.95, 1.0, 30);
        expect(d.tier).toBe(2);
        expect(d.driftMs).toBeCloseTo(150, 5);
        // The other way round: the player already wrapped, the clock hasn't.
        expect(computeVideoSyncDecision(29.98, 0.02, 1.0, 30).driftMs).toBeCloseTo(-40, 5);
    });

    test("without a loop duration the drift is the plain difference", () => {
        expect(computeVideoSyncDecision(0.1, 29.95).tier).toBe(3);
    });

    test("wrapLoopDrift maps any drift into half a loop either side", () => {
        expect(wrapLoopDrift(-29.85, 30)).toBeCloseTo(0.15, 9);
        expect(wrapLoopDrift(10, 30)).toBeCloseTo(10, 9);
        expect(wrapLoopDrift(-10, 30)).toBeCloseTo(-10, 9);
        expect(wrapLoopDrift(61, 30)).toBeCloseTo(1, 9);
    });
});

describe("Where dedicated media should be now", () => {
    const base = { is_playing: true, current_time: 10, timestamp_ms: 1_000_000, start_at_epoch_ms: 1_000_000, is_looping: false };
    test("a playing track has moved on from where Play was pressed", () => {
        expect(projectedMediaPosition(base, 1_060_000)).toBe(70);
    });
    test("a paused track is where it was paused", () => {
        expect(projectedMediaPosition({ ...base, is_playing: false }, 1_060_000)).toBe(10);
    });
    test("a start in the future hasn't begun", () => {
        expect(projectedMediaPosition({ ...base, start_at_epoch_ms: 2_000_000 }, 1_060_000)).toBe(10);
    });
    test("a looping track wraps at its length", () => {
        expect(projectedMediaPosition({ ...base, is_looping: true }, 1_060_000, 30)).toBe(10);
    });
});

