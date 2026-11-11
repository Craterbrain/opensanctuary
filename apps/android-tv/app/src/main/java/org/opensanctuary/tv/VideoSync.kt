package org.opensanctuary.tv

import kotlin.math.abs

/** Kotlin port of web/src/core/video_sync.ts's 3-tier sync decision engine --
 * same thresholds (75ms/500ms) and same deadband/slew/seek behavior, so the
 * Android TV app's ExoPlayer drift correction matches the web client's
 * bit-for-bit instead of being a separately-tuned approximation. */
enum class SyncTier { DEADBAND, SLEW, SEEK }

data class VideoSyncDecision(
    val tier: SyncTier,
    val targetTimeMs: Long? = null,
    val playbackRate: Float,
    val driftMs: Double,
)

/** A looping video's drift taken the short way round the loop, in [-duration/2, duration/2):
 * just after the expected position wraps to 0:00, a player still finishing the last loop is a
 * moment behind, not a whole loop ahead (wrapLoopDrift in video_sync.ts). */
fun wrapLoopDrift(driftMs: Long, loopDurationMs: Long): Long {
    val half = loopDurationMs / 2
    return Math.floorMod(driftMs + half, loopDurationMs) - half
}

/** `loopDurationMs` is the video's duration when it loops: the drift is then measured around the loop. */
fun computeVideoSyncDecision(expectedTimeMs: Long, currentTimeMs: Long, loopDurationMs: Long? = null): VideoSyncDecision {
    val rawDriftMs = expectedTimeMs - currentTimeMs
    val driftMs = (if (loopDurationMs != null && loopDurationMs > 0) wrapLoopDrift(rawDriftMs, loopDurationMs) else rawDriftMs).toDouble()
    val absDrift = abs(driftMs)

    return when {
        absDrift > 500 -> VideoSyncDecision(
            tier = SyncTier.SEEK,
            targetTimeMs = expectedTimeMs,
            playbackRate = 1.0f,
            driftMs = driftMs,
        )
        absDrift > 75 -> {
            val rateAdjust = (driftMs / 1000.0) * 0.1
            val newRate = (1.0 + rateAdjust).coerceIn(0.95, 1.05).toFloat()
            VideoSyncDecision(tier = SyncTier.SLEW, playbackRate = newRate, driftMs = driftMs)
        }
        else -> VideoSyncDecision(tier = SyncTier.DEADBAND, playbackRate = 1.0f, driftMs = driftMs)
    }
}
