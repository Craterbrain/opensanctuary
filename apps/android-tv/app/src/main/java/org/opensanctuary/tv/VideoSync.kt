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

fun computeVideoSyncDecision(expectedTimeMs: Long, currentTimeMs: Long): VideoSyncDecision {
    val driftMs = (expectedTimeMs - currentTimeMs).toDouble()
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
