package org.opensanctuary.tv

/**
 * NTP-style clock sync -- Kotlin port of web/src/core/timesync.ts's TimeSync.
 * Keeps this app's notion of "now" aligned with the console's clock so the
 * ExoPlayer drift correction in MainActivity (VideoSync.kt's
 * computeVideoSyncDecision) agrees with the web clients it's supposed to
 * stay in lockstep with -- both sides anchor playback position to the same
 * `start_at_epoch_ms` the console hands out, so they only actually match if
 * both sides' idea of "now" matches too.
 *
 * Network calls go through `PairingManager.fetchServerTimeMs`, which reuses
 * that class's already-pinned HTTPS client for the console's self-signed
 * certificate rather than this class needing its own trust decision.
 */
class TvClock(private val pairingManager: PairingManager) {
    @Volatile
    private var offsetMs: Long = 0

    @Volatile
    var isSynced: Boolean = false
        private set

    suspend fun synchronize(serverIp: String, serverPort: Int, samples: Int = 5) {
        val offsets = mutableListOf<Long>()
        repeat(samples) {
            val startNanos = System.nanoTime()
            val serverTimeMs = pairingManager.fetchServerTimeMs(serverIp, serverPort).getOrNull() ?: return@repeat
            val endNanos = System.nanoTime()

            val rttMs = (endNanos - startNanos) / 1_000_000
            val oneWayLatencyMs = rttMs / 2
            val localNowAtServerTime = System.currentTimeMillis() - oneWayLatencyMs
            offsets.add(serverTimeMs - localNowAtServerTime)
        }
        if (offsets.isNotEmpty()) {
            offsets.sort()
            offsetMs = offsets[offsets.size / 2]
            isSynced = true
        }
    }

    /** Mirrors timesync.ts's `getServerTimeMs()`/`now()`. */
    fun nowMs(): Long = System.currentTimeMillis() + offsetMs
}
