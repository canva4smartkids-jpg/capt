package dev.captureone.android.logic

import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.PermissionKind
import dev.captureone.capture.contract.PermissionState
import dev.captureone.capture.contract.PermissionStatus
import dev.captureone.capture.contract.QualityPreset
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.ResolutionSetting
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.presets.EncodingPresets
import dev.captureone.capture.presets.resolveOutputSize
import dev.captureone.capture.settings.CaptureSettings
import dev.captureone.capture.util.Format

/*
 * Platform-neutral presentation logic for the phone/tablet UI. No android.* imports: unit-tested on
 * the JVM and shared by the Compose screens in dev.captureone.android.ui.
 */

data class PermissionCardModel(
    val kind: PermissionKind,
    val title: String,
    val explanation: String,
    val state: PermissionState,
    /** Button label, or null when nothing can be requested. */
    val actionLabel: String?,
    /** The button opens system settings instead of a runtime prompt. */
    val opensSettings: Boolean,
) {
    val satisfied: Boolean get() = state == PermissionState.GRANTED || state == PermissionState.PER_SESSION || state == PermissionState.NOT_APPLICABLE
}

object PermissionCards {
    private fun title(kind: PermissionKind) = when (kind) {
        PermissionKind.SCREEN_CAPTURE -> "Screen capture"
        PermissionKind.MICROPHONE -> "Microphone"
        PermissionKind.SYSTEM_AUDIO -> "Internal audio"
        PermissionKind.STORAGE -> "Storage"
        PermissionKind.NOTIFICATIONS -> "Notifications"
        PermissionKind.LOCAL_NETWORK -> "Local network"
    }

    private fun defaultExplanation(kind: PermissionKind) = when (kind) {
        PermissionKind.SCREEN_CAPTURE -> "Android shows a consent dialog every time you start a recording or screenshot. Apps that block capture (banking, DRM video) appear black."
        PermissionKind.MICROPHONE -> "Lets CaptureOne record your voice alongside the screen."
        PermissionKind.SYSTEM_AUDIO -> "Records sound from other apps (Android 10+). Apps can opt out; calls are never captured."
        PermissionKind.STORAGE -> "Needed on Android 9 and older to save into Movies and Pictures."
        PermissionKind.NOTIFICATIONS -> "Shows the timer and pause/stop controls while recording."
        PermissionKind.LOCAL_NETWORK -> "Used to reach a CaptureOne desktop hub on your network."
    }

    fun from(status: PermissionStatus): PermissionCardModel {
        val action = when (status.state) {
            PermissionState.PROMPT -> "Allow"
            PermissionState.DENIED -> if (status.requiresSystemSettings) "Open settings" else "Allow"
            else -> null
        }
        return PermissionCardModel(
            kind = status.kind,
            title = title(status.kind),
            explanation = status.message ?: defaultExplanation(status.kind),
            state = status.state,
            actionLabel = action,
            opensSettings = status.requiresSystemSettings,
        )
    }

    /** Cards worth showing on the Record screen, most important first. */
    fun forRecordScreen(statuses: List<PermissionStatus>): List<PermissionCardModel> {
        val order = listOf(PermissionKind.SCREEN_CAPTURE, PermissionKind.NOTIFICATIONS, PermissionKind.MICROPHONE, PermissionKind.SYSTEM_AUDIO, PermissionKind.STORAGE)
        return statuses.filter { it.kind in order && it.state != PermissionState.NOT_APPLICABLE }
            .sortedBy { order.indexOf(it.kind) }
            .map(::from)
    }
}

data class AudioToggleModel(val checked: Boolean, val available: Boolean, val note: String)

data class EstimateModel(val outputSize: Size, val videoKbps: Int, val bytesPerMinute: Long) {
    val summary: String
        get() = "${outputSize.width}×${outputSize.height} · ${"%.1f".format(videoKbps / 1000.0)} Mbps · ≈ ${Format.bytes(bytesPerMinute)}/min"
}

/** Video/audio option lists filtered by what this device can actually do. */
data class OptionsModel(
    val codecs: List<VideoCodec>,
    val resolutions: List<ResolutionSetting>,
    val frameRates: List<Int>,
    val qualities: List<QualityPreset>,
    val countdowns: List<Int>,
)

object RecordOptions {
    fun options(report: CapabilityReport?, display: Size?): OptionsModel {
        val codecs = buildList {
            if (report == null || report.isUsable(FeatureKey.H264)) add(VideoCodec.H264)
            if (report != null && report.isUsable(FeatureKey.H265)) add(VideoCodec.H265)
        }.ifEmpty { listOf(VideoCodec.H264) }
        // Contract semantics: "height" scales the source height (portrait phones get narrow outputs), never upscales.
        val heights = ResolutionSetting.Height.HEIGHTS.filter { display == null || it < display.height }
        val resolutions = listOf<ResolutionSetting>(ResolutionSetting.Source) + heights.map { ResolutionSetting.Height(it) }
        val fps = report?.frameRates?.takeIf { it.isNotEmpty() } ?: listOf(24, 30, 60)
        return OptionsModel(codecs, resolutions, fps, listOf(QualityPreset.LOW, QualityPreset.MEDIUM, QualityPreset.HIGH, QualityPreset.ULTRA), CaptureSettings.COUNTDOWN_CHOICES)
    }

    fun label(r: ResolutionSetting): String = when (r) {
        is ResolutionSetting.Source -> "Native"
        is ResolutionSetting.Height -> "${r.height}p"
        is ResolutionSetting.Custom -> "${r.width}×${r.height}"
    }

    /** Label including the resulting output size for this display, e.g. "1080 px tall · 486×1080". */
    fun label(r: ResolutionSetting, display: Size?): String {
        if (display == null) return label(r)
        val out = resolveOutputSize(display, r)
        return when (r) {
            is ResolutionSetting.Source -> "Native · ${out.width}×${out.height}"
            is ResolutionSetting.Height -> "${r.height} px tall · ${out.width}×${out.height}"
            is ResolutionSetting.Custom -> "Fit ${r.width}×${r.height} · ${out.width}×${out.height}"
        }
    }

    fun label(c: VideoCodec): String = if (c == VideoCodec.H264) "H.264" else "H.265 / HEVC"

    fun label(q: QualityPreset): String = q.name.lowercase().replaceFirstChar { it.uppercase() }

    fun mic(report: CapabilityReport?, settings: CaptureSettings, micPermission: PermissionState?): AudioToggleModel {
        val usable = report?.isUsable(FeatureKey.MICROPHONE_AUDIO) ?: true
        val note = when {
            !usable -> report?.features?.get(FeatureKey.MICROPHONE_AUDIO)?.reason ?: "Not available on this device."
            micPermission == PermissionState.GRANTED -> "Records your voice."
            else -> "Requires the microphone permission."
        }
        return AudioToggleModel(settings.audio.microphone.enabled && usable, usable, note)
    }

    fun system(report: CapabilityReport?, settings: CaptureSettings): AudioToggleModel {
        val f = report?.features?.get(FeatureKey.SYSTEM_AUDIO)
        val usable = report?.isUsable(FeatureKey.SYSTEM_AUDIO) ?: false
        val note = if (usable) "Only apps that allow capture (media/games). Calls and protected apps stay silent." else f?.reason ?: "Not available on this device."
        return AudioToggleModel(settings.audio.system.enabled && usable, usable, note)
    }

    /** Applies device limits to persisted settings (e.g. HEVC chosen on a device without HEVC). */
    fun sanitize(settings: CaptureSettings, report: CapabilityReport?): CaptureSettings {
        var s = settings
        if (s.video.codec == VideoCodec.H265 && report != null && !report.isUsable(FeatureKey.H265)) s = s.copy(video = s.video.copy(codec = VideoCodec.H264))
        if (s.audio.system.enabled && report != null && !report.isUsable(FeatureKey.SYSTEM_AUDIO)) s = s.copy(audio = s.audio.copy(system = s.audio.system.copy(enabled = false)))
        if (s.audio.microphone.enabled && report != null && !report.isUsable(FeatureKey.MICROPHONE_AUDIO)) s = s.copy(audio = s.audio.copy(microphone = s.audio.microphone.copy(enabled = false)))
        return s
    }

    fun estimate(settings: CaptureSettings, display: Size?, presets: EncodingPresets?): EstimateModel? {
        if (display == null || presets == null) return null
        val size = resolveOutputSize(display, settings.video.resolution)
        val kbps = presets.videoBitrateKbps(size, settings.video.fps, settings.video.codec, settings.video.quality, settings.video.bitrateKbps)
        val tracks = when {
            !settings.audio.microphone.enabled && !settings.audio.system.enabled -> 0
            settings.audio.trackMode == dev.captureone.capture.contract.AudioTrackMode.MIXED -> 1
            else -> listOf(settings.audio.microphone.enabled, settings.audio.system.enabled).count { it }
        }
        val perMin = presets.estimateFileSizeBytes(kbps.toDouble(), tracks * settings.audio.bitrateKbps.toDouble(), 60.0)
        return EstimateModel(size, kbps, perMin)
    }
}

/** Recording controls derived from a [RecordingStatus]. */
data class ControlsModel(
    val title: String,
    val timer: String,
    val detail: String,
    val canPause: Boolean,
    val canResume: Boolean,
    val canStop: Boolean,
    val canCancel: Boolean,
    val canScreenshot: Boolean,
    val countdown: Int?,
    val warnings: List<String>,
) {
    companion object {
        fun from(status: RecordingStatus?): ControlsModel? {
            if (status == null) return null
            val st = status.state
            val active = st == RecordingState.COUNTDOWN || st == RecordingState.STARTING || st.isActive || st == RecordingState.STOPPING || st == RecordingState.FINALIZING
            if (!active) return null
            val title = when (st) {
                RecordingState.COUNTDOWN -> "Starting in ${status.countdownRemainingSec ?: 0}…"
                RecordingState.STARTING -> "Starting…"
                RecordingState.RECORDING -> "Recording"
                RecordingState.PAUSED -> "Paused"
                RecordingState.STOPPING, RecordingState.FINALIZING -> "Saving…"
                else -> st.wireName
            }
            val detail = buildList {
                if (status.bytesWritten > 0) add(Format.bytes(status.bytesWritten))
                status.encoder?.let { add(it) }
                status.freeDiskBytes?.let { add("${Format.bytes(it)} free") }
            }.joinToString(" · ")
            return ControlsModel(
                title = title,
                timer = Format.duration(status.elapsedMs),
                detail = detail,
                canPause = st == RecordingState.RECORDING,
                canResume = st == RecordingState.PAUSED,
                canStop = st.isActive,
                canCancel = st == RecordingState.COUNTDOWN,
                canScreenshot = st.isActive,
                countdown = if (st == RecordingState.COUNTDOWN) status.countdownRemainingSec else null,
                warnings = status.warnings,
            )
        }
    }
}

/** One row of the library list. */
data class LibraryRowModel(
    val id: String,
    val title: String,
    val subtitle: String,
    val isVideo: Boolean,
    val shareable: Boolean,
    val validationNote: String?,
) {
    companion object {
        fun from(media: MediaMetadata, published: Boolean): LibraryRowModel {
            val parts = buildList {
                if (media.kind == MediaKind.VIDEO) media.durationMs?.let { add(Format.duration(it)) }
                add("${media.width}×${media.height}")
                add(Format.bytes(media.sizeBytes))
                if (media.kind == MediaKind.VIDEO) media.videoCodec?.let { add(RecordOptions.label(it)) }
                if (media.audioTracks.isNotEmpty()) add("${media.audioTracks.size} audio")
                add(media.createdAt.take(16).replace('T', ' '))
            }
            val note = when {
                media.validation?.valid == false -> "Validation failed: " + media.validation?.issues?.joinToString { it.message }
                !published -> "Kept in app storage (not in the shared gallery)"
                else -> null
            }
            return LibraryRowModel(media.id, media.title, parts.joinToString(" · "), media.kind == MediaKind.VIDEO, published, note)
        }
    }
}

data class StorageUsageModel(val text: String) {
    companion object {
        fun from(totalBytes: Long, videos: Int, screenshots: Int, freeBytes: Long) =
            StorageUsageModel("$videos recordings · $screenshots screenshots · ${Format.bytes(totalBytes)} used · ${Format.bytes(freeBytes)} free")
    }
}
