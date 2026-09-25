/**
 * OpenSanctuary / OS-Next Protocol Specification & Validation
 */

// Must track src/core/engine.rs's CURRENT_PROTOCOL_VERSION on the backend — they're
// serialized/deserialized from the exact same StateSnapshot shape.
export const CURRENT_PROTOCOL_VERSION = 2;

export interface SnapshotValidationResult {
    valid: boolean;
    error?: string;
    protocolVersion: number;
}

/**
 * Validates incoming WebSocket or REST state snapshots against the expected engine protocol.
 */
export function validateSnapshotProtocol(snapshot: any, expectedVersion: number = CURRENT_PROTOCOL_VERSION): SnapshotValidationResult {
    if (!snapshot || typeof snapshot !== 'object') {
        return {
            valid: false,
            error: 'Invalid snapshot: payload is null or not an object',
            protocolVersion: 0
        };
    }

    const version = typeof snapshot.protocol_version === 'number' ? snapshot.protocol_version : 1;

    if (version !== expectedVersion) {
        return {
            valid: false,
            error: `Protocol version mismatch: expected ${expectedVersion}, got ${version}`,
            protocolVersion: version
        };
    }

    if (typeof snapshot.sequence_number !== 'number') {
        return {
            valid: false,
            error: 'Invalid snapshot: missing or invalid sequence_number',
            protocolVersion: version
        };
    }

    if (!snapshot.state || typeof snapshot.state !== 'object') {
        return {
            valid: false,
            error: 'Invalid snapshot: missing or invalid state object',
            protocolVersion: version
        };
    }

    return {
        valid: true,
        protocolVersion: version
    };
}
