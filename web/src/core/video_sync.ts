import { clock } from "./timesync.ts";

/**
 * 3-Tier Video Synchronization Algorithm (Syncplay/timingsrc model)
 * Ensures HTML5 <video> elements play in exact lockstep across physical devices.
 */
export class VideoSyncController {
    private video: HTMLVideoElement;
    private masterStartTimeMs: number = 0;
    private syncInterval: number | null = null;
    private isFOH: boolean = false; // Is this the Front-Of-House main audio egress?

    constructor(videoElement: HTMLVideoElement, isFOH: boolean = false) {
        this.video = videoElement;
        this.isFOH = isFOH;

        // Enforce Single Audio Egress
        if (!this.isFOH) {
            this.video.muted = true;
        }
    }

    /**
     * Start playback synchronized to a specific server epoch time.
     * @param serverEpochMs The exact server time the video SHOULD have started playing from 0:00.
     */
    playSynchronized(serverEpochMs: number) {
        this.masterStartTimeMs = serverEpochMs;
        this.video.play().catch(e => console.warn("Video auto-play blocked", e));
        
        if (this.syncInterval) clearInterval(this.syncInterval);
        
        // Run the 3-tier correction algorithm every 500ms
        this.syncInterval = window.setInterval(() => this.correctDrift(), 500);
        
        // Initial correction
        this.correctDrift();
    }

    stop() {
        if (this.syncInterval) {
            clearInterval(this.syncInterval);
            this.syncInterval = null;
        }
        this.video.pause();
    }

    private correctDrift() {
        if (this.video.paused || this.video.readyState < 2) return;

        const currentServerTime = clock.getServerTimeMs();
        
        // How long the video SHOULD have been playing
        let expectedVideoTime = (currentServerTime - this.masterStartTimeMs) / 1000.0;
        
        // Handle looping videos
        if (this.video.loop && this.video.duration > 0) {
            expectedVideoTime = expectedVideoTime % this.video.duration;
        }

        const actualVideoTime = this.video.currentTime;
        const loopDuration = this.video.loop && this.video.duration > 0 ? this.video.duration : undefined;
        const decision = computeVideoSyncDecision(expectedVideoTime, actualVideoTime, this.video.playbackRate, loopDuration);

        if (decision.tier === 3) {
            console.warn(`[VideoSync] Hard Seek applied. Drift: ${decision.driftMs.toFixed(1)}ms`);
            this.video.currentTime = decision.targetTime!;
            this.video.playbackRate = decision.playbackRate;
        } else if (decision.tier === 2) {
            if (Math.abs(this.video.playbackRate - decision.playbackRate) > 0.01) {
                this.video.playbackRate = decision.playbackRate;
            }
        } else {
            if (this.video.playbackRate !== 1.0) {
                this.video.playbackRate = 1.0;
            }
        }
    }
}

export interface VideoSyncDecision {
    tier: 1 | 2 | 3;
    action: 'deadband' | 'slew' | 'seek';
    targetTime?: number;
    playbackRate: number;
    driftMs: number;
}

/**
 * A looping video's drift taken the short way round the loop, in [-duration/2, duration/2).
 * Just after the expected position wraps to 0:00 a player still finishing the last loop is
 * a moment behind, not a whole loop ahead: a straight subtraction would hard-seek every
 * screen at every wrap.
 */
export function wrapLoopDrift(driftSeconds: number, loopDurationSec: number): number {
    const d = loopDurationSec;
    return ((((driftSeconds + d / 2) % d) + d) % d) - d / 2;
}

/**
 * Pure 3-Tier Video Synchronization Decision Engine. `loopDurationSec` is the video's
 * duration when it loops: the drift is then measured around the loop (see `wrapLoopDrift`).
 */
export function computeVideoSyncDecision(
    expectedTime: number,
    currentTime: number,
    _currentRate: number = 1.0,
    loopDurationSec?: number
): VideoSyncDecision {
    const rawDrift = expectedTime - currentTime;
    const driftSeconds = loopDurationSec && loopDurationSec > 0 ? wrapLoopDrift(rawDrift, loopDurationSec) : rawDrift;
    const driftMs = driftSeconds * 1000.0;
    const absDrift = Math.abs(driftMs);

    if (absDrift > 500) {
        // Tier 3: Drift > 500ms -> Trigger a hard seek
        return {
            tier: 3,
            action: 'seek',
            targetTime: expectedTime,
            playbackRate: 1.0,
            driftMs
        };
    } else if (absDrift > 75) {
        // Tier 2: Drift 75ms - 500ms -> Apply progressive playbackRate scaling (0.95x - 1.05x)
        const rateAdjust = driftSeconds * 0.1;
        const newRate = Math.max(0.95, Math.min(1.05, 1.0 + rateAdjust));
        return {
            tier: 2,
            action: 'slew',
            playbackRate: newRate,
            driftMs
        };
    } else {
        // Tier 1: Drift <= 75ms -> Deadband (snap to 1.0x)
        return {
            tier: 1,
            action: 'deadband',
            playbackRate: 1.0,
            driftMs
        };
    }
}

/**
 * Where dedicated media should be now, in seconds. `current_time` is the position when it was last
 * played, paused or sought, not a live position: a playing item has moved on since its start anchor.
 * A start scheduled in the future hasn't begun yet. A looping item wraps at `durationSec` when known.
 */
export function projectedMediaPosition(
    mp: { is_playing: boolean; current_time: number; timestamp_ms: number; start_at_epoch_ms?: number | null; is_looping: boolean },
    nowMs: number,
    durationSec?: number
): number {
    if (!mp.is_playing) return mp.current_time;
    const anchor = mp.start_at_epoch_ms || mp.timestamp_ms;
    const position = mp.current_time + Math.max(0, nowMs - anchor) / 1000;
    return mp.is_looping && durationSec && durationSec > 0 ? position % durationSec : position;
}
