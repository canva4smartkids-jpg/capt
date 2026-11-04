package dev.captureone.capture.engine

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.os.Build
import dev.captureone.capture.capability.CapabilityClassifier
import dev.captureone.capture.capability.EncoderFacts
import dev.captureone.capture.contract.HardwareAcceleration
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.presets.EncoderSizeLimits
import dev.captureone.capture.presets.fitToEncoder

/** A video encoder found on the device, with its size limits. */
class VideoEncoderCandidate(
    val codec: VideoCodec,
    val info: MediaCodecInfo,
    val hardware: Boolean?,
) {
    val name: String get() = info.name
    private val caps: MediaCodecInfo.VideoCapabilities? = info.getCapabilitiesForType(codec.mime).videoCapabilities

    val sizeLimits: EncoderSizeLimits? = caps?.let { c ->
        EncoderSizeLimits(
            widthAlignment = c.widthAlignment,
            heightAlignment = c.heightAlignment,
            minWidth = c.supportedWidths.lower,
            minHeight = c.supportedHeights.lower,
            maxWidth = c.supportedWidths.upper,
            maxHeight = c.supportedHeights.upper,
            isSupported = { w, h -> c.isSizeSupported(w, h) },
        )
    }

    fun supportsFrameRate(size: Size, fps: Int): Boolean =
        caps?.let { c -> runCatching { c.areSizeAndRateSupported(size.width, size.height, fps.toDouble()) }.getOrDefault(true) } ?: false

    /** Clamps a bitrate into the encoder's supported range (bps). */
    fun clampBitrate(bps: Int): Int = caps?.bitrateRange?.clamp(bps) ?: bps

    fun fit(desired: Size): Size? = sizeLimits?.let { fitToEncoder(desired, it) }

    fun supportsSurfaceInput(): Boolean =
        info.getCapabilitiesForType(codec.mime).colorFormats.contains(MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
}

/** Enumerates encoders with `MediaCodecList(REGULAR_CODECS)` and picks one for a recording. */
object EncoderCatalog {
    fun videoEncoders(): List<VideoEncoderCandidate> {
        val list = MediaCodecList(MediaCodecList.REGULAR_CODECS)
        val out = ArrayList<VideoEncoderCandidate>()
        for (info in list.codecInfos) {
            if (!info.isEncoder) continue
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && info.isAlias) continue
            for (codec in VideoCodec.entries) {
                if (info.supportedTypes.none { it.equals(codec.mime, ignoreCase = true) }) continue
                val hw = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) info.isHardwareAccelerated else null
                out += VideoEncoderCandidate(codec, info, hw)
            }
        }
        return out
    }

    fun hasAacEncoder(): Boolean = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos.any { info ->
        info.isEncoder && info.supportedTypes.any { it.equals("audio/mp4a-latm", ignoreCase = true) }
    }

    fun facts(verifiedNames: Set<String>): List<EncoderFacts> = videoEncoders().map { c ->
        EncoderFacts(
            codec = c.codec,
            name = c.name,
            hardware = c.hardware,
            maxWidth = c.sizeLimits?.maxWidth ?: 0,
            maxHeight = c.sizeLimits?.maxHeight ?: 0,
            verified = c.name in verifiedNames,
        )
    }

    private fun isHardware(c: VideoEncoderCandidate): Boolean = c.hardware ?: !CapabilityClassifier.looksLikeSoftwareCodec(c.name)

    /**
     * Ordered candidates for [codec] honouring the hardware preference. `MediaCodecList` order is the
     * vendor's preference order, which is kept within each group.
     */
    fun candidates(codec: VideoCodec, hw: HardwareAcceleration): List<VideoEncoderCandidate> {
        val all = videoEncoders().filter { it.codec == codec && it.supportsSurfaceInput() }
        return when (hw) {
            HardwareAcceleration.OFF -> all.filter { !isHardware(it) }.ifEmpty { all }
            HardwareAcceleration.PREFER, HardwareAcceleration.AUTO -> all.sortedByDescending { isHardware(it) }
        }
    }
}
