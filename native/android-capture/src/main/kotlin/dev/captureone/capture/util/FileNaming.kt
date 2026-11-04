package dev.captureone.capture.util

import java.time.LocalDateTime
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.roundToLong

/** Port of packages/settings/src/filename.ts. */
object FileNaming {
    private val INVALID = Regex("[<>:\"/\\\\|?*\\u0000-\\u001f]")
    // JavaScript's \s set, spelled out: Android ICU regex rejects the inline Unicode-class flag, and plain \s differs
    // between ICU (Unicode) and the JVM (ASCII).
    private val WHITESPACE = Regex("[\\t\\n\\u000B\\f\\r \\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000\\uFEFF]+")
    private val TRAILING_DOTS_SPACES = Regex("[. ]+$")
    private val RESERVED = Regex("^(con|prn|aux|nul|com[0-9]|lpt[0-9])$", RegexOption.IGNORE_CASE)

    /** Expands `{date} {time} {source} {n}`; the result is filesystem-safe (no extension). */
    fun expand(template: String, date: LocalDateTime, source: String? = null, n: Int? = null): String {
        fun pad(v: Int) = v.toString().padStart(2, '0')
        val d = "${date.year}-${pad(date.monthValue)}-${pad(date.dayOfMonth)}"
        val t = "${pad(date.hour)}-${pad(date.minute)}-${pad(date.second)}"
        val out = template
            .replace("{date}", d)
            .replace("{time}", t)
            .replace("{source}", source ?: "Screen")
            .replace("{n}", (n ?: 1).toString())
        return sanitize(out)
    }

    /** Removes characters invalid on Windows/macOS/Linux/Android and trims to 120 chars. */
    fun sanitize(name: String): String {
        var s = name.replace(INVALID, "_").replace(WHITESPACE, " ").trim()
        s = s.replace(TRAILING_DOTS_SPACES, "")
        if (s.isEmpty()) s = "CaptureOne"
        if (RESERVED.matches(s)) s = "_$s"
        return s.take(120)
    }

    /**
     * Returns `base.ext`, or `base (2).ext`, `base (3).ext`… — the first name for which [exists] is false.
     */
    fun unique(base: String, extension: String, exists: (String) -> Boolean): String {
        val first = "$base.$extension"
        if (!exists(first)) return first
        var i = 2
        while (true) {
            val candidate = "$base ($i).$extension"
            if (!exists(candidate)) return candidate
            i++
        }
    }
}

/** Ports of `formatBytes` / `formatDuration` from packages/settings/src/presets.ts. */
object Format {
    fun bytes(bytes: Double): String {
        if (bytes.isNaN() || bytes.isInfinite() || bytes < 0) return "—"
        val units = listOf("B", "KB", "MB", "GB", "TB")
        var v = bytes
        var i = 0
        while (v >= 1024 && i < units.size - 1) {
            v /= 1024
            i++
        }
        val num = if (v >= 100 || i == 0) floor(v + 0.5).roundToLong().toString() else toFixed1(v)
        return "$num ${units[i]}"
    }

    fun bytes(bytes: Long): String = bytes(bytes.toDouble())

    /** JS `Number.prototype.toFixed(1)` for the non-negative values used here. */
    private fun toFixed1(v: Double): String = java.math.BigDecimal(v).setScale(1, java.math.RoundingMode.HALF_UP).toPlainString()

    fun duration(ms: Double): String {
        val total = max(0.0, floor(ms / 1000)).toLong()
        val h = total / 3600
        val m = (total % 3600) / 60
        val s = total % 60
        val mm = m.toString().padStart(2, '0')
        val ss = s.toString().padStart(2, '0')
        return if (h > 0) "$h:$mm:$ss" else "$mm:$ss"
    }

    fun duration(ms: Long): String = duration(ms.toDouble())
}
