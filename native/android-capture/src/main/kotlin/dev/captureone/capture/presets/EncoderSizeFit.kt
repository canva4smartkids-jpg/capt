package dev.captureone.capture.presets

import dev.captureone.capture.contract.Size
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/**
 * Size constraints of a concrete encoder (from `MediaCodecInfo.VideoCapabilities` on device).
 * Pure data so the fitting logic is unit-testable on the JVM.
 */
class EncoderSizeLimits(
    val widthAlignment: Int,
    val heightAlignment: Int,
    val minWidth: Int,
    val minHeight: Int,
    val maxWidth: Int,
    val maxHeight: Int,
    /** `VideoCapabilities.isSizeSupported` (accounts for block-count / macroblock-rate limits). */
    val isSupported: (width: Int, height: Int) -> Boolean,
)

private tailrec fun gcd(a: Int, b: Int): Int = if (b == 0) a else gcd(b, a % b)
private fun lcm(a: Int, b: Int): Int = a / gcd(a, b) * b
private fun floorTo(v: Double, step: Int): Int = (floor(v / step) * step).toInt()

/**
 * Shrinks [desired] (already even, from [resolveOutputSize]) until the encoder accepts it, keeping the
 * aspect ratio as closely as the encoder's alignment allows. Never upscales. Returns null when no
 * acceptable size exists (caller reports ENCODER_UNAVAILABLE).
 */
fun fitToEncoder(desired: Size, limits: EncoderSizeLimits): Size? {
    val wa = lcm(2, max(1, limits.widthAlignment))
    val ha = lcm(2, max(1, limits.heightAlignment))
    if (desired.width % wa == 0 && desired.height % ha == 0 &&
        desired.width in limits.minWidth..limits.maxWidth && desired.height in limits.minHeight..limits.maxHeight &&
        limits.isSupported(desired.width, desired.height)
    ) {
        return desired
    }
    var scale = min(1.0, min(limits.maxWidth.toDouble() / desired.width, limits.maxHeight.toDouble() / desired.height))
    repeat(80) {
        val w = max(wa, floorTo(desired.width * scale, wa))
        val h = max(ha, floorTo(desired.height * scale, ha))
        if (w < limits.minWidth || h < limits.minHeight) return null
        if (w <= limits.maxWidth && h <= limits.maxHeight && limits.isSupported(w, h)) return Size(w, h)
        scale *= 0.96
    }
    return null
}
