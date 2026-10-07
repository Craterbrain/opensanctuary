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
        const decision = computeVideoSyncDecision(expectedVideoTime, actualVideoTime, this.video.playbackRate);

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
 * Pure 3-Tier Video Synchronization Decision Engine
 */
export function computeVideoSyncDecision(
    expectedTime: number,
    currentTime: number,
    currentRate: number = 1.0
): VideoSyncDecision {
    const driftSeconds = expectedTime - currentTime;
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

