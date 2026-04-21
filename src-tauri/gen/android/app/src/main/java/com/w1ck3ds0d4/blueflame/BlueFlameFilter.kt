package com.w1ck3ds0d4.blueflame

import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * Built-in filter list and live stats for the Android WebView interception path.
 *
 * Mirrors the default regex set in src-tauri/src/proxy.rs so desktop and
 * mobile block the same things with zero filter-list loading. A follow-up PR
 * will replace this with a shared Rust filter engine invoked through JNI.
 */
object BlueFlameFilter {
    private val patterns: List<Regex> = listOf(
        Regex("^https?://[^/]*doubleclick\\.net/"),
        Regex("^https?://[^/]*google-analytics\\.com/"),
        Regex("^https?://[^/]*googletagmanager\\.com/"),
        Regex("^https?://[^/]*facebook\\.com/tr"),
        Regex("^https?://[^/]*hotjar\\.com/"),
        Regex("^https?://[^/]*mixpanel\\.com/"),
        Regex("^https?://[^/]*segment\\.(io|com)/"),
        Regex("^https?://[^/]*amplitude\\.com/"),
    )

    val enabled = AtomicBoolean(true)
    val requestsTotal = AtomicLong(0)
    val requestsBlocked = AtomicLong(0)
    val bytesSaved = AtomicLong(0)

    /** True when the URL matches any filter pattern and filtering is on. */
    fun shouldBlock(url: String): Boolean {
        requestsTotal.incrementAndGet()
        if (!enabled.get()) return false
        if (patterns.any { it.containsMatchIn(url) }) {
            requestsBlocked.incrementAndGet()
            bytesSaved.addAndGet(2048) // rough estimate; actual savings need response inspection
            return true
        }
        return false
    }

    fun snapshot(): Stats = Stats(
        requestsTotal = requestsTotal.get(),
        requestsBlocked = requestsBlocked.get(),
        bytesSaved = bytesSaved.get(),
    )

    data class Stats(
        val requestsTotal: Long,
        val requestsBlocked: Long,
        val bytesSaved: Long,
    )
}
