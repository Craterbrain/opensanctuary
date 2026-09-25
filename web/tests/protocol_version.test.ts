import { describe, expect, test } from "bun:test";
import { validateSnapshotProtocol, CURRENT_PROTOCOL_VERSION } from "../src/core/protocol.ts";

describe("StateSnapshot Protocol Versioning", () => {
    test("accepts valid snapshot with CURRENT_PROTOCOL_VERSION", () => {
        const snapshot = {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            sequence_number: 42,
            timestamp_ms: 1789076846570,
            state: {
                is_blackout: false,
                is_clear_text: false,
                is_logo_override: false,
                live_item: null,
                live_slide_index: 0
            },
            schedule: {
                id: "sched_1",
                title: "Sunday Service",
                items: []
            }
        };

        const res = validateSnapshotProtocol(snapshot);
        expect(res.valid).toBe(true);
        expect(res.protocolVersion).toBe(CURRENT_PROTOCOL_VERSION);
    });

    test("a snapshot missing protocol_version entirely (pre-versioning client) is treated as version 1 and rejected as a mismatch against the current version", () => {
        const legacySnapshot = {
            sequence_number: 10,
            timestamp_ms: 1789076840000,
            state: { is_blackout: true },
            schedule: { items: [] }
        };

        const res = validateSnapshotProtocol(legacySnapshot);
        expect(res.valid).toBe(false);
        expect(res.protocolVersion).toBe(1);
    });

    test("rejects snapshot with mismatched protocol_version", () => {
        const futureSnapshot = {
            protocol_version: 99,
            sequence_number: 5,
            timestamp_ms: 1789076840000,
            state: {},
            schedule: {}
        };

        const res = validateSnapshotProtocol(futureSnapshot);
        expect(res.valid).toBe(false);
        expect(res.error).toContain("Protocol version mismatch");
        expect(res.protocolVersion).toBe(99);
    });

    test("rejects malformed snapshots", () => {
        expect(validateSnapshotProtocol(null).valid).toBe(false);
        expect(validateSnapshotProtocol(undefined).valid).toBe(false);
        expect(validateSnapshotProtocol("not a json object").valid).toBe(false);
        expect(validateSnapshotProtocol({ protocol_version: 1 }).valid).toBe(false); // missing sequence_number and state
    });
});
