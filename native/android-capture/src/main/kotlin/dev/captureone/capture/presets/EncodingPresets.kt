package dev.captureone.capture.presets

import dev.captureone.capture.contract.QualityPreset
import dev.captureone.capture.contract.ResolutionSetting
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.VideoCodec
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min

/**
 * Kotlin port of packages/settings/src/presets.ts, driven by the same packages/settings/presets.json
 * (copied into the library's assets at build time by the `copyEncodingPresets` task — never by hand).
 * Results must match tests/contract/fixtures/presets-cases.json exactly.
 */
@Serializable
data class EncodingPresets(
    val version: Int,
    val bitsPerPixel: Map<String, Map<String, Double>>,
    val minBitrateKbps: Double,
    val maxBitrateKbps: Double,
    val containerOverhead: Double,
    val resolutionHeights: List<Int>,
    val frameRates: List<Int>,
    val audioBitratesKbps: List<Int>,
    val defaultAudioBitrateKbps: Int,
) {
    fun bitsPerPixel(codec: VideoCodec, quality: QualityPreset): Double {
        require(quality != QualityPreset.CUSTOM) { "custom quality has no bits-per-pixel" }
        val codecKey = if (codec == VideoCodec.H264) "h264" else "h265"
        val qualityKey = quality.name.lowercase()
        return bitsPerPixel[codecKey]?.get(qualityKey) ?: error("presets.json lacks $codecKey.$qualityKey")
    }

    /** Target video bitrate in kbps for a preset (clamped to min/max), or the custom value. */
    fun videoBitrateKbps(size: Size, fps: Int, codec: VideoCodec, quality: QualityPreset, customKbps: Int? = null): Int {
        if (quality == QualityPreset.CUSTOM) {
            requireNotNull(customKbps) { "customKbps required for custom quality" }
            return customKbps
        }
        val bpp = bitsPerPixel(codec, quality)
        // Same evaluation order as the TS implementation: ((w * h) * fps) * bpp / 1000.
        val kbps = size.width.toDouble() * size.height.toDouble() * fps.toDouble() * bpp / 1000.0
        return jsRound(min(maxBitrateKbps, max(minBitrateKbps, kbps))).toInt()
    }

    /** Estimated output size in bytes for a duration, including container overhead. */
    fun estimateFileSizeBytes(videoKbps: Double, audioKbpsTotal: Double, durationSec: Double): Long {
        require(durationSec >= 0) { "duration must be >= 0" }
        val bytes = ((videoKbps + audioKbpsTotal) * 1000.0 * durationSec) / 8.0
        return jsRound(bytes * (1 + containerOverhead)).toLong()
    }

    companion object {
        private val json = Json { ignoreUnknownKeys = true }

        fun parse(text: String): EncodingPresets = json.decodeFromString(serializer(), text)

        /** Name of the asset the Gradle task copies presets.json to. */
        const val ASSET_NAME = "captureone/presets.json"
    }
}

/** JavaScript `Math.round` (round half towards +∞), so results equal the TS/Rust implementations. */
internal fun jsRound(v: Double): Double = floor(v + 0.5)

private fun even(n: Double): Int = max(2.0, floor(n / 2.0) * 2.0).toInt()

/**
 * Output size for a source and a resolution setting. Never upscales, preserves aspect ratio and
 * always returns even dimensions (required by 4:2:0 encoders). Port of `resolveOutputSize`.
 */
fun resolveOutputSize(source: Size, setting: ResolutionSetting): Size {
    require(source.width > 0 && source.height > 0) { "source size must be positive" }
    val sw = source.width.toDouble()
    val sh = source.height.toDouble()
    return when (setting) {
        is ResolutionSetting.Source -> Size(even(sw), even(sh))
        is ResolutionSetting.Height -> {
            val h = setting.height.toDouble()
            if (h >= sh) {
                Size(even(sw), even(sh))
            } else {
                val scale = h / sh
                Size(even(jsRound(sw * scale)), even(h))
            }
        }
        is ResolutionSetting.Custom -> {
            val scale = min(min(setting.width / sw, setting.height / sh), 1.0)
            Size(even(jsRound(sw * scale)), even(jsRound(sh * scale)))
        }
    }
}
