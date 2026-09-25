package org.opensanctuary.tv

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.KeyEvent
import android.view.View
import android.view.WindowManager
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch

class MainActivity : AppCompatActivity() {

    private lateinit var webView: WebView
    private lateinit var statusOverlay: LinearLayout
    private lateinit var statusText: TextView
    private lateinit var statusHint: TextView
    private lateinit var btnSettings: Button
    private lateinit var remoteHintToast: TextView

    private val prefs by lazy { getSharedPreferences("open_sanctuary_tv", Context.MODE_PRIVATE) }
    private val mainHandler = Handler(Looper.getMainLooper())
    private var isConnected = false
    private var retryCount = 0

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        hideSystemUI()

        setContentView(R.layout.activity_main)

        webView = findViewById(R.id.liveWebView)
        statusOverlay = findViewById(R.id.statusOverlay)
        statusText = findViewById(R.id.statusText)
        statusHint = findViewById(R.id.statusHint)
        btnSettings = findViewById(R.id.btnOpenSettings)
        remoteHintToast = findViewById(R.id.remoteHintToast)

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
        }

        webView.webChromeClient = object : WebChromeClient() {}
    }

    private fun checkAndConnect() {
        var serverIp = prefs.getString("server_ip", "") ?: ""
        val port = prefs.getInt("server_port", 8080)
        val isStageMode = prefs.getBoolean("is_stage_mode", false)

        if (serverIp.isEmpty()) {
            serverIp = "192.168.1.251"
            prefs.edit().putString("server_ip", serverIp).apply()
        }
        loadLiveUrl(serverIp, port, isStageMode)
    }

    private fun loadLiveUrl(serverIp: String, port: Int, isStageMode: Boolean) {
        val endpoint = if (isStageMode) "stage.html" else "live.html"
        val fullUrl = "http://$serverIp:$port/$endpoint"

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
                val port = prefs.getInt("server_port", 8080)
                val isStageMode = prefs.getBoolean("is_stage_mode", false)
                if (serverIp.isNotEmpty()) {
                    loadLiveUrl(serverIp, port, isStageMode)
                }
            }
        }, 5000)
    }

    private fun showSettingsDialog() {
        SettingsDialog(this) { ip, port, stageMode ->
            loadLiveUrl(ip, port, stageMode)
        }.show()
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
