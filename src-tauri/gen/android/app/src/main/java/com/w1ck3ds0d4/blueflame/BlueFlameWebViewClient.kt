package com.w1ck3ds0d4.blueflame

import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import java.io.ByteArrayInputStream

/**
 * WebViewClient that routes every sub-request through [BlueFlameFilter].
 *
 * Android gives us the right hook for free: `shouldInterceptRequest` is
 * called for every resource request (HTML, CSS, JS, XHR, images, fonts)
 * before the network stack is touched, so we can block or return a synthetic
 * 204 without ever touching TLS - no MITM proxy, no CA trust needed.
 */
class BlueFlameWebViewClient : WebViewClient() {
    override fun shouldInterceptRequest(
        view: WebView,
        request: WebResourceRequest,
    ): WebResourceResponse? {
        val url = request.url.toString()
        if (BlueFlameFilter.shouldBlock(url)) {
            return WebResourceResponse(
                "text/plain",
                "utf-8",
                204,
                "No Content",
                emptyMap(),
                ByteArrayInputStream(ByteArray(0)),
            )
        }
        return super.shouldInterceptRequest(view, request)
    }
}
