package org.opensanctuary.tv

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import com.google.zxing.BarcodeFormat
import com.google.zxing.qrcode.QRCodeWriter
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.cert.CertificateException
import java.security.cert.X509Certificate
import java.util.UUID
import java.util.concurrent.TimeUnit
import javax.net.ssl.HostnameVerifier
import javax.net.ssl.SSLContext
import javax.net.ssl.X509TrustManager

/**
 * Implements the pairing half of docs/CLIENT_PAIRING.md's two-way QR bridge
 * on the TV side, plus the instance-hijack check the doc describes but this
 * app never actually enforced before. Two ways a device gets its
 * `device_token`:
 *
 * 1. **Self-authorize** (ADB-provisioned): the console handed this launch a
 *    `pairing_session_token` directly (see MainActivity's intent-extra
 *    handling) -- no phone needed, this app calls `POST /api/pairing/authorize`
 *    itself with that token.
 * 2. **Manual QR** (installed some other way): this app renders its own
 *    pairing QR (`{"device_id","name","platform"}`) and polls
 *    `GET /api/pairing/status` until an operator's phone scans it and
 *    authorizes it, exactly as the doc's two-way bridge describes.
 *
 * Either way, once paired, every subsequent connect compares
 * `GET /api/server-info`'s `instance_id` against the one stored at pairing
 * time -- the actual hijack prevention the doc's design is for, not
 * implemented at all before this.
 */
class PairingManager(context: Context) {
    private val prefs = context.getSharedPreferences("open_sanctuary_tv", Context.MODE_PRIVATE)

    companion object {
        /** SHA-256 of a cert's raw DER bytes, as lowercase hex -- must match
         * `src/network/tls.rs::cert_fingerprint_sha256` byte-for-byte
         * (`X509Certificate.getEncoded()` returns the same raw DER encoding
         * `rustls_pemfile`/`CertificateDer` does, so hashing it directly
         * here produces an identical string with no format/encoding
         * mismatch to worry about). Also used by MainActivity's
         * `WebViewClient.onReceivedSslError`, so it's a top-level helper
         * rather than private to the trust manager below.
         */
        fun sha256Fingerprint(cert: X509Certificate): String {
            val digest = MessageDigest.getInstance("SHA-256").digest(cert.encoded)
            return digest.joinToString("") { "%02x".format(it) }
        }

        /** Converts a WebView `android.net.http.SslCertificate` (all
         * `MainActivity`'s `WebViewClient.onReceivedSslError` is ever
         * handed) into a real `java.security.cert.X509Certificate` so
         * `sha256Fingerprint` above can hash it. `SslCertificate` grew a
         * direct `.x509Certificate` getter, but only on API 29+ -- this
         * app's `minSdk` is 24, so this goes through the older
         * `saveState()`/Bundle round-trip instead, which has worked since
         * much earlier Android versions and covers the full supported
         * range uniformly (no reason to branch on API level for this). */
        fun extractX509(sslCert: android.net.http.SslCertificate): X509Certificate? {
            val bundle = android.net.http.SslCertificate.saveState(sslCert)
            val derBytes = bundle.getByteArray("x509-certificate") ?: return null
            return java.security.cert.CertificateFactory.getInstance("X.509")
                .generateCertificate(derBytes.inputStream()) as? X509Certificate
        }
    }

    fun getPinnedCertFingerprint(): String? = prefs.getString("pinned_cert_fingerprint", null)

    /** Pins a fingerprint delivered through an already-trusted channel --
     * the ADB launch intent (`src/network/adb.rs::provision`'s
     * `cert_fingerprint` extra, see MainActivity) -- so the very first
     * HTTPS connection in the ADB-provisioned path is a real verification,
     * not trust-on-first-use. Overwrites any existing pin: this is only
     * ever called with a value the console itself just vouched for. */
    fun setPinnedCertFingerprint(fingerprint: String) {
        prefs.edit().putString("pinned_cert_fingerprint", fingerprint).apply()
    }

    /** Called when the operator manually changes `server_ip`/`server_port`
     * in Settings (`SettingsDialog`) -- a pin established for whatever was
     * at the old address has no bearing on whatever's at the new one, and
     * leaving it in place would make the pinning check reject a perfectly
     * legitimate, different console's certificate as if it were a
     * man-in-the-middle. Deliberately narrower than `clearPairing()`: the
     * device_token/paired_instance_id stay put, since `checkInstance()`'s
     * own instance_id comparison already handles "this is actually a
     * different console" at the application layer -- this is purely the
     * lower-level TLS trust decision resetting alongside it. */
    fun clearPinnedCertFingerprint() {
        prefs.edit().remove("pinned_cert_fingerprint").apply()
    }

    /** The console serves HTTPS with a self-signed certificate (no public
     * CA on a church LAN -- src/network/tls.rs). Rather than trusting any
     * such certificate unconditionally, this pins the specific certificate
     * this app has actually talked to:
     *
     * - If a fingerprint was already pinned (via `setPinnedCertFingerprint`
     *   above, the ADB path; or by a prior connection, below), the
     *   presented certificate must match it exactly -- a mismatch means
     *   either a real man-in-the-middle, or the console's cert genuinely
     *   changed (e.g. an operator ran "Regenerate certificate"), and either
     *   way this app should not silently proceed.
     * - If nothing is pinned yet (first-ever connection via the manual QR /
     *   manually-typed-IP path, which has no side channel to deliver a
     *   fingerprint through), trust-on-first-use: accept this certificate
     *   and pin it, the same model SSH's `known_hosts` uses. Every
     *   connection after this one is a real check, not a repeat of this
     *   one-time trust decision.
     */
    private val pinningTrustManager = object : X509TrustManager {
        override fun checkClientTrusted(chain: Array<out X509Certificate>?, authType: String?) {}

        override fun checkServerTrusted(chain: Array<out X509Certificate>?, authType: String?) {
            val leaf = chain?.firstOrNull()
                ?: throw CertificateException("No certificate presented by server")
            val presented = sha256Fingerprint(leaf)
            val pinned = getPinnedCertFingerprint()
            if (pinned == null) {
                setPinnedCertFingerprint(presented)
                return
            }
            if (presented != pinned) {
                throw CertificateException(
                    "Server certificate does not match the pinned certificate for this console " +
                        "(expected $pinned, got $presented) -- refusing to connect"
                )
            }
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    private val client = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(5, TimeUnit.SECONDS)
        .sslSocketFactory(
            SSLContext.getInstance("TLS").apply { init(null, arrayOf(pinningTrustManager), SecureRandom()) }.socketFactory,
            pinningTrustManager,
        )
        // Hostname verification is meaningless for a self-signed cert
        // issued to a LAN IP -- the pinningTrustManager above is the actual
        // identity check (the exact certificate, not a CA-validated name),
        // so this is safe to skip rather than a blanket "don't verify
        // anything" shortcut.
        .hostnameVerifier(HostnameVerifier { _, _ -> true })
        .build()

    fun getOrCreateDeviceId(): String {
        prefs.getString("device_id", null)?.let { return it }
        val fresh = UUID.randomUUID().toString()
        prefs.edit().putString("device_id", fresh).apply()
        return fresh
    }

    fun getStoredDeviceToken(): String? = prefs.getString("device_token", null)
    fun getStoredInstanceId(): String? = prefs.getString("paired_instance_id", null)

    /** "Reset Pairing" -- deliberate and explicit only, never automatic (see
     * docs/CLIENT_PAIRING.md's "Resetting a pairing"). Leaves `device_id`
     * alone; resets the credential, the console it was issued for, and the
     * pinned certificate together -- all three describe trust in "this one
     * console," so a reset has to clear all of them or the next pairing
     * attempt would be rejected by a pin left over from the old one. */
    fun clearPairing() {
        prefs.edit().remove("device_token").remove("paired_instance_id").remove("pinned_cert_fingerprint").apply()
    }

    private fun baseUrl(serverIp: String, serverPort: Int) = "https://$serverIp:$serverPort"

    suspend fun fetchInstanceId(serverIp: String, serverPort: Int): Result<String> = withContext(Dispatchers.IO) {
        try {
            val req = Request.Builder().url("${baseUrl(serverIp, serverPort)}/api/server-info").build()
            client.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) return@withContext Result.failure(Exception("HTTP ${resp.code}"))
                val body = resp.body?.string() ?: return@withContext Result.failure(Exception("empty response"))
                Result.success(JSONObject(body).getString("instance_id"))
            }
        } catch (e: Exception) {
            Result.failure(e)
        }
    }

    /** Backs `TvClock`'s NTP-style sync (Kotlin port of
     * web/src/core/timesync.ts) -- reuses this class's pinned HTTPS client
     * rather than TvClock needing its own trust decision for the same
     * self-signed console certificate. */
    suspend fun fetchServerTimeMs(serverIp: String, serverPort: Int): Result<Long> = withContext(Dispatchers.IO) {
        try {
            val req = Request.Builder().url("${baseUrl(serverIp, serverPort)}/api/time").build()
            client.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) return@withContext Result.failure(Exception("HTTP ${resp.code}"))
                val body = resp.body?.string() ?: return@withContext Result.failure(Exception("empty response"))
                Result.success(JSONObject(body).getLong("server_time_ms"))
            }
        } catch (e: Exception) {
            Result.failure(e)
        }
    }

    sealed class InstanceCheckResult {
        data class Match(val instanceId: String) : InstanceCheckResult()
        data class Mismatch(val expected: String, val actual: String) : InstanceCheckResult()
        data class Unreachable(val reason: String) : InstanceCheckResult()
    }

    suspend fun checkInstance(serverIp: String, serverPort: Int): InstanceCheckResult {
        val result = fetchInstanceId(serverIp, serverPort)
        val actual = result.getOrNull()
            ?: return InstanceCheckResult.Unreachable(result.exceptionOrNull()?.message ?: "unknown error")
        val expected = getStoredInstanceId()
        return if (expected == null || expected == actual) {
            InstanceCheckResult.Match(actual)
        } else {
            InstanceCheckResult.Mismatch(expected, actual)
        }
    }

    /** Self-authorize path -- see class doc comment. Stores the resulting
     * `device_token` and the console's `instance_id` on success. */
    suspend fun selfAuthorize(serverIp: String, serverPort: Int, sessionToken: String, deviceName: String): Result<String> =
        withContext(Dispatchers.IO) {
            try {
                val payload = JSONObject().apply {
                    put("session_token", sessionToken)
                    put("device_id", getOrCreateDeviceId())
                    put("name", deviceName)
                    put("platform", "android-tv")
                }
                val body = payload.toString().toRequestBody("application/json".toMediaType())
                val req = Request.Builder().url("${baseUrl(serverIp, serverPort)}/api/pairing/authorize").post(body).build()
                client.newCall(req).execute().use { resp ->
                    val respText = resp.body?.string() ?: ""
                    if (!resp.isSuccessful) return@withContext Result.failure(Exception("HTTP ${resp.code}: $respText"))
                    val json = JSONObject(respText)
                    if (!json.optBoolean("success", false)) return@withContext Result.failure(Exception("Console rejected the pairing session"))
                    val token = json.getString("token")
                    persistPairing(token, serverIp, serverPort)
                    Result.success(token)
                }
            } catch (e: Exception) {
                Result.failure(e)
            }
        }

    /** Manual QR path -- renders `{"device_id","name","platform"}` for a
     * phone to scan, matching the payload `docs/CLIENT_PAIRING.md` specifies. */
    fun buildPairingQrBitmap(deviceName: String, sizePx: Int = 512): Bitmap {
        val payload = JSONObject().apply {
            put("device_id", getOrCreateDeviceId())
            put("name", deviceName)
            put("platform", "android-tv")
        }.toString()
        val bits = QRCodeWriter().encode(payload, BarcodeFormat.QR_CODE, sizePx, sizePx)
        val bmp = Bitmap.createBitmap(sizePx, sizePx, Bitmap.Config.RGB_565)
        for (x in 0 until sizePx) {
            for (y in 0 until sizePx) {
                bmp.setPixel(x, y, if (bits.get(x, y)) Color.BLACK else Color.WHITE)
            }
        }
        return bmp
    }

    /** One poll of `GET /api/pairing/status` -- returns the device_token the
     * moment an operator's phone has authorized this device, `null` while
     * still waiting. Caller (`MainActivity`) is responsible for the polling
     * loop/interval. */
    suspend fun pollPairingStatus(serverIp: String, serverPort: Int): Result<String?> = withContext(Dispatchers.IO) {
        try {
            val deviceId = getOrCreateDeviceId()
            val req = Request.Builder()
                .url("${baseUrl(serverIp, serverPort)}/api/pairing/status?device_id=$deviceId")
                .build()
            client.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) return@withContext Result.failure(Exception("HTTP ${resp.code}"))
                val json = JSONObject(resp.body?.string() ?: "{}")
                val hasToken = json.optBoolean("paired", false) && json.has("token") && !json.isNull("token")
                if (!hasToken) return@withContext Result.success(null)
                val token = json.getString("token")
                persistPairing(token, serverIp, serverPort)
                Result.success(token)
            }
        } catch (e: Exception) {
            Result.failure(e)
        }
    }

    private suspend fun persistPairing(token: String, serverIp: String, serverPort: Int) {
        // Best-effort -- if this particular call fails, the very next
        // checkInstance() call (which MainActivity always makes before
        // trusting a stored token) will simply see no stored instance_id yet
        // and treat it as a fresh match rather than a mismatch, so a
        // transient failure here doesn't strand the device.
        val instanceId = fetchInstanceId(serverIp, serverPort).getOrNull()
        prefs.edit().apply {
            putString("device_token", token)
            if (instanceId != null) putString("paired_instance_id", instanceId)
            apply()
        }
    }
}
