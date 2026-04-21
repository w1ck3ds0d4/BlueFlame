package com.w1ck3ds0d4.blueflame

import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
    }

    /**
     * Hook provided by Tauri's Android activity. Called after the webview is
     * constructed but before any URL is loaded. We attach our custom
     * [BlueFlameWebViewClient] so every sub-request goes through the filter.
     */
    override fun onWebViewCreate(webView: WebView) {
        super.onWebViewCreate(webView)
        webView.webViewClient = BlueFlameWebViewClient()
    }
}
