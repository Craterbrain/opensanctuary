package org.opensanctuary.tv

import android.annotation.SuppressLint
import android.content.Context
import android.content.Intent
import android.net.http.SslError
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.KeyEvent
import android.view.View
import android.view.WindowManager
import android.webkit.JavascriptInterface
import android.webkit.SslErrorHandler
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ProgressBar
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.PlayerView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.math.max

class MainActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private lateinit var videoPlayerView: PlayerView
    private lateinit var statusOverlay: LinearLayout
    private lateinit var statusText: TextView
    private lateinit var statusHint: TextView
    private lateinit var btnSettings: Button
    private lateinit var remoteHintToast: TextView
    private lateinit var pairingQrImage: ImageView
    private lateinit var progressBar: ProgressBar

    private lateinit var pairingManager: PairingManager
    private lateinit var tvClock: TvClock
    private var exoPlayer: ExoPlayer? = null
    private val prefs by lazy { getSharedPreferences("open_sanctuary_tv", Context.MODE_PRIVATE) }
    private val mainHandler = Handler(Looper.getMainLooper())
    private val activityScope = CoroutineScope(Dispatchers.Main)
    private var pairingPollJob: Job? = null
    private var isConnected = false
    private var retryCount = 0

    // --- Native background-video playback (AndroidTVBridge below) ---
    // Hardware-decodes via ExoPlayer/MediaCodec instead of relying on the
    // WebView's own Chromium build supporting WebCodecs, which isn't
    // guaranteed on OEM Android TV boxes -- see
    // web/src/core/webcodecs_player.ts's three-tier renderer selection,
    // where this is tier 1 (highest priority, checked first).
    private var driftCorrectionJob: Job? = null
    private var bgVideoUrl: String? = null
    private var bgStartAnchorMs: Long = 0
    private var bgLoop: Boolean = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        hideSystemUI()

        setContentView(R.layout.activity_main)

        webView = findViewById(R.id.liveWebView)
        videoPlayerView = findViewById(R.id.videoPlayerView)
        statusOverlay = findViewById(R.id.statusOverlay)
        statusText = findViewById(R.id.statusText)
        statusHint = findViewById(R.id.statusHint)
        btnSettings = findViewById(R.id.btnOpenSettings)
        remoteHintToast = findViewById(R.id.remoteHintToast)
        pairingQrImage = findViewById(R.id.pairingQrImage)
        progressBar = findViewById(R.id.progressBar)
        pairingManager = PairingManager(this)
        tvClock = TvClock(pairingManager)

        exoPlayer = ExoPlayer.Builder(this).build().also { videoPlayerView.player = it }

        setupWebView()

        btnSettings.setOnClickListener {
            showSettingsDialog()
        }

        // Hide the remote hint toast after 8 seconds
        mainHandler.postDelayed({
            remoteHintToast.animate().alpha(0f).setDuration(1000).withEndAction {
                remoteHintToast.visibility = View.GONE
            }
        }, 8000)

        handleLaunchIntentExtras(intent)
    }

    /// `launchMode="singleTask"` means a second ADB-driven provisioning
    /// launch while this app is already running arrives here, not `onCreate`
    /// -- without this override, re-running "Install via ADB" against an
    /// already-installed instance would silently do nothing.
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleLaunchIntentExtras(intent)
    }

    /// Reads the extras `adb shell am start` supplies (see
    /// `src/network/adb.rs::provision` on the console side): persists
    /// `server_ip`/`server_port`/`is_stage_mode` the same way the manual
    /// Settings dialog does, pins the console's own certificate fingerprint
    /// (`PairingManager.setPinnedCertFingerprint` -- delivered over ADB, not
    /// fetched over the network, so this app's very first HTTPS connection
    /// below is already a real verification instead of trust-on-first-use),
    /// then -- if a `pairing_session_token` came along -- self-authorizes
    /// with it (`PairingManager`) instead of waiting for a phone to scan a
    /// second QR. Either way, `checkAndConnect()` is the single place that
    /// decides what to actually show next.
    private fun handleLaunchIntentExtras(intent: Intent) {
        val extras = intent.extras
        val ip = extras?.getString("server_ip")
        if (!ip.isNullOrEmpty()) {
            val port = if (extras.containsKey("server_port")) extras.getInt("server_port") else prefs.getInt("server_port", 8443)
            val stageMode = extras.getBoolean("is_stage_mode", prefs.getBoolean("is_stage_mode", false))
            prefs.edit()
                .putString("server_ip", ip)
                .putInt("server_port", port)
                .putBoolean("is_stage_mode", stageMode)
                .apply()

            val certFingerprint = extras.getString("cert_fingerprint")
            if (!certFingerprint.isNullOrEmpty()) {
                pairingManager.setPinnedCertFingerprint(certFingerprint)
            }

            val sessionToken = extras.getString("pairing_session_token")
            if (!sessionToken.isNullOrEmpty()) {
                val deviceName = extras.getString("console_name")?.takeIf { it.isNotBlank() } ?: "Sanctuary TV"
                statusOverlay.visibility = View.VISIBLE
                statusText.text = "Pairing with console…"
                statusHint.text = ""
                activityScope.launch {
                    val result = pairingManager.selfAuthorize(ip, port, sessionToken, deviceName)
                    result.onFailure {
                        // Not fatal -- checkAndConnect() below falls back to
                        // the manual QR-and-poll bridge when there's still
                        // no stored device_token, so a transient failure
                        // here doesn't strand the install.
                        statusText.text = "Automatic pairing failed"
                        statusHint.text = it.message ?: "Unknown error"
                    }
                    checkAndConnect()
                }
                return
            }
        }
        checkAndConnect()
    }

    @SuppressLint("SetJavaScriptEnabled")
    private fun setupWebView() {
        webView.setLayerType(View.LAYER_TYPE_HARDWARE, null)
        webView.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            databaseEnabled = true
            mediaPlaybackRequiresUserGesture = false
            loadWithOverviewMode = true
            useWideViewPort = true
            mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
            cacheMode = WebSettings.LOAD_DEFAULT
            allowFileAccess = true
            allowContentAccess = true
        }

        webView.webViewClient = object : WebViewClient() {
            override fun onPageFinished(view: WebView?, url: String?) {
                super.onPageFinished(view, url)
                if (!url.isNullOrEmpty() && !url.startsWith("data:") && !url.startsWith("about:blank")) {
                    isConnected = true
                    retryCount = 0
                    statusOverlay.visibility = View.GONE
                }
            }

            override fun onReceivedError(
                view: WebView?,
                request: WebResourceRequest?,
                error: WebResourceError?
            ) {
                super.onReceivedError(view, request, error)
                if (request?.isForMainFrame == true) {
                    onConnectionFailed("Unable to reach OpenSanctuary server")
                }
            }

            // The console serves HTTPS with a self-signed certificate
            // (src/network/tls.rs -- there's no public CA on a church LAN),
            // which WebView rejects by default with no built-in way to
            // prompt-and-proceed. Rather than blindly proceeding past any
            // cert warning, this checks the presented certificate against
            // the same pin PairingManager's OkHttp client already
            // establishes (set from the ADB launch's cert_fingerprint
            // extra, or trust-on-first-use via the manual pairing path) --
            // by the time the WebView ever navigates here, pairing has
            // already happened, so a pin should always already exist. A
            // mismatch is refused, not silently accepted.
            override fun onReceivedSslError(view: WebView?, handler: SslErrorHandler?, error: SslError?) {
                val cert = error?.certificate?.let { PairingManager.extractX509(it) }
                val pinned = pairingManager.getPinnedCertFingerprint()
                if (cert != null && pinned != null && PairingManager.sha256Fingerprint(cert) == pinned) {
                    handler?.proceed()
                } else {
                    handler?.cancel()
                    onConnectionFailed("Server certificate does not match this console's pinned certificate")
                }
            }
        }

        webView.webChromeClient = object : WebChromeClient() {}

        // Tier 1 of web/src/core/webcodecs_player.ts's renderer selection --
        // when this bridge exists, the page delegates background-video
        // playback here instead of decoding it itself.
        webView.addJavascriptInterface(AndroidTvBridge(), "AndroidTV")
    }

    /** Methods are invoked by the WebView on a non-UI thread, so every body
     * hops to `mainHandler` before touching `exoPlayer`/views. */
    private inner class AndroidTvBridge {
        @JavascriptInterface
        fun playBackgroundVideo(url: String, startAtEpochMs: Double, loop: Boolean, muted: Boolean) {
            mainHandler.post { startBackgroundVideo(url, startAtEpochMs.toLong(), loop, muted) }
        }

        @JavascriptInterface
        fun stopBackgroundVideo() {
            mainHandler.post { stopBackgroundVideoInternal() }
        }

        @JavascriptInterface
        fun setVolume(volume: Double) {
            mainHandler.post { exoPlayer?.volume = volume.toFloat() }
        }
    }

    private fun startBackgroundVideo(url: String, startAtEpochMs: Long, loop: Boolean, muted: Boolean) {
        val player = exoPlayer ?: return
        bgStartAnchorMs = startAtEpochMs
        bgLoop = loop
        player.volume = if (muted) 0f else 1f
        player.repeatMode = if (loop) Player.REPEAT_MODE_ONE else Player.REPEAT_MODE_OFF

        if (bgVideoUrl != url) {
            bgVideoUrl = url
            player.setMediaItem(MediaItem.fromUri(url))
            player.prepare()
        }
        videoPlayerView.visibility = View.VISIBLE

        activityScope.launch {
            // Sync this device's own clock to the console before computing
            // where playback should start -- otherwise "startAtEpochMs" means
            // nothing (see TvClock, the Kotlin port of timesync.ts's NTP
            // median technique).
            if (!tvClock.isSynced) {
                val serverIp = prefs.getString("server_ip", "") ?: ""
                val port = prefs.getInt("server_port", 8443)
                tvClock.synchronize(serverIp, port)
            }
            seekToAnchorWhenReady(player)
        }
        startDriftCorrectionLoop(player)
    }

    private fun stopBackgroundVideoInternal() {
        driftCorrectionJob?.cancel()
        driftCorrectionJob = null
        bgVideoUrl = null
        exoPlayer?.stop()
        videoPlayerView.visibility = View.GONE
    }

    private fun expectedBgPositionMs(durationMs: Long): Long {
        val elapsed = max(0L, tvClock.nowMs() - bgStartAnchorMs)
        return if (bgLoop && durationMs > 0) elapsed % durationMs else elapsed.coerceAtMost(durationMs)
    }

    private fun seekToAnchorWhenReady(player: ExoPlayer) {
        if (player.playbackState == Player.STATE_READY && player.duration != C.TIME_UNSET) {
            player.seekTo(expectedBgPositionMs(player.duration))
            player.playWhenReady = true
            return
        }
        player.addListener(object : Player.Listener {
            override fun onPlaybackStateChanged(state: Int) {
                if (state == Player.STATE_READY && player.duration != C.TIME_UNSET) {
                    player.removeListener(this)
                    player.seekTo(expectedBgPositionMs(player.duration))
                    player.playWhenReady = true
                }
            }
        })
    }

    /** Ports web/src/core/video_sync.ts's 3-tier correction (VideoSync.kt) to
     * ExoPlayer: deadband (do nothing), slew (nudge PlaybackParameters speed),
     * or hard seek -- same 75ms/500ms thresholds as the web client. */
    private fun startDriftCorrectionLoop(player: ExoPlayer) {
        driftCorrectionJob?.cancel()
        driftCorrectionJob = activityScope.launch {
            while (isActive) {
                delay(500)
                val duration = player.duration
                if (player.playbackState != Player.STATE_READY || duration == C.TIME_UNSET || duration <= 0) continue

                val expectedMs = expectedBgPositionMs(duration)
                val decision = computeVideoSyncDecision(expectedMs, player.currentPosition)
                when (decision.tier) {
                    SyncTier.SEEK -> {
                        player.seekTo(decision.targetTimeMs ?: expectedMs)
                        player.playbackParameters = PlaybackParameters(1.0f)
                    }
                    SyncTier.SLEW -> {
                        if (abs(player.playbackParameters.speed - decision.playbackRate) > 0.01f) {
                            player.playbackParameters = PlaybackParameters(decision.playbackRate)
                        }
                    }
                    SyncTier.DEADBAND -> {
                        if (player.playbackParameters.speed != 1.0f) {
                            player.playbackParameters = PlaybackParameters(1.0f)
                        }
                    }
                }
            }
        }
    }

    /// Single source of truth for what the app shows next: no stored
    /// pairing credential yet -> the manual QR-and-poll bridge
    /// (docs/CLIENT_PAIRING.md); a stored credential whose console
    /// `instance_id` no longer matches -> the "different console" screen
    /// (the actual hijack-prevention the pairing design exists for); a
    /// matching (or unreachable-to-check) credential -> load the live view.
    private fun checkAndConnect() {
        pairingPollJob?.cancel()

        var serverIp = prefs.getString("server_ip", "") ?: ""
        val port = prefs.getInt("server_port", 8443)
        val isStageMode = prefs.getBoolean("is_stage_mode", false)

        if (serverIp.isEmpty()) {
            serverIp = "192.168.1.251"
            prefs.edit().putString("server_ip", serverIp).apply()
        }

        if (pairingManager.getStoredDeviceToken() == null) {
            showManualPairingScreen(serverIp, port, isStageMode)
            return
        }

        activityScope.launch {
            when (val check = pairingManager.checkInstance(serverIp, port)) {
                is PairingManager.InstanceCheckResult.Mismatch -> showWrongConsoleScreen()
                is PairingManager.InstanceCheckResult.Unreachable,
                is PairingManager.InstanceCheckResult.Match -> loadLiveUrl(serverIp, port, isStageMode)
            }
        }
    }

    private fun showManualPairingScreen(serverIp: String, port: Int, isStageMode: Boolean) {
        statusOverlay.visibility = View.VISIBLE
        progressBar.visibility = View.GONE
        pairingQrImage.visibility = View.VISIBLE
        statusText.text = "Scan to pair this display"
        statusHint.text = "Open the console's \"Pair TV App\" QR and scan this screen with your phone"
        pairingQrImage.setImageBitmap(pairingManager.buildPairingQrBitmap(android.os.Build.MODEL ?: "Android TV"))

        pairingPollJob = activityScope.launch {
            while (isActive) {
                val token = pairingManager.pollPairingStatus(serverIp, port).getOrNull()
                if (!token.isNullOrEmpty()) {
                    pairingQrImage.visibility = View.GONE
                    progressBar.visibility = View.VISIBLE
                    loadLiveUrl(serverIp, port, isStageMode)
                    break
                }
                delay(3000)
            }
        }
    }

    private fun showWrongConsoleScreen() {
        pairingQrImage.visibility = View.GONE
        progressBar.visibility = View.GONE
        statusOverlay.visibility = View.VISIBLE
        statusText.text = "Different Console Detected"
        statusHint.text = getString(R.string.pairing_wrong_console)
    }

    private fun loadLiveUrl(serverIp: String, port: Int, isStageMode: Boolean) {
        pairingQrImage.visibility = View.GONE
        progressBar.visibility = View.VISIBLE
        val endpoint = if (isStageMode) "stage.html" else "live.html"
        val fullUrl = "https://$serverIp:$port/$endpoint"

        statusOverlay.visibility = View.VISIBLE
        statusText.text = "Connecting to $fullUrl…"
        statusHint.text = getString(R.string.offline_msg)

        webView.loadUrl(fullUrl)
    }

    private fun onConnectionFailed(reason: String) {
        isConnected = false
        statusOverlay.visibility = View.VISIBLE
        statusText.text = "Connection Failed: $reason"
        statusHint.text = "Retrying in 5 seconds…\nPress [MENU] or [BACK] on remote for settings."

        retryCount++
        mainHandler.postDelayed({
            if (!isConnected) {
                val serverIp = prefs.getString("server_ip", "") ?: ""
                val port = prefs.getInt("server_port", 8443)
                val isStageMode = prefs.getBoolean("is_stage_mode", false)
                if (serverIp.isNotEmpty()) {
                    loadLiveUrl(serverIp, port, isStageMode)
                }
            }
        }, 5000)
    }

    private fun showSettingsDialog() {
        SettingsDialog(
            context = this,
            pairingManager = pairingManager,
            onSave = { ip, port, stageMode -> loadLiveUrl(ip, port, stageMode) },
            onResetPairing = { checkAndConnect() },
        ).show()
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent?): Boolean {
        when (keyCode) {
            KeyEvent.KEYCODE_MENU, KeyEvent.KEYCODE_SETTINGS, KeyEvent.KEYCODE_INFO -> {
                showSettingsDialog()
                return true
            }
            KeyEvent.KEYCODE_BACK -> {
                if (event?.isLongPress == true) {
                    showSettingsDialog()
                    return true
                }
            }
            KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE, KeyEvent.KEYCODE_PROG_RED -> {
                webView.reload()
                return true
            }
        }
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyLongPress(keyCode: Int, event: KeyEvent?): Boolean {
        if (keyCode == KeyEvent.KEYCODE_BACK || keyCode == KeyEvent.KEYCODE_DPAD_CENTER) {
            showSettingsDialog()
            return true
        }
        return super.onKeyLongPress(keyCode, event)
    }

    override fun onResume() {
        super.onResume()
        hideSystemUI()
    }

    override fun onDestroy() {
        super.onDestroy()
        pairingPollJob?.cancel()
        driftCorrectionJob?.cancel()
        exoPlayer?.release()
        exoPlayer = null
    }

    private fun hideSystemUI() {
        window.decorView.systemUiVisibility = (
            View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                or View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
                or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                or View.SYSTEM_UI_FLAG_FULLSCREEN
            )
    }
}
