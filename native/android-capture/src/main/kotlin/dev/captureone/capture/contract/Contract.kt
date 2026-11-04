@file:OptIn(ExperimentalSerializationApi::class)

package dev.captureone.capture.contract

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonClassDiscriminator
import kotlinx.serialization.json.JsonObject

/*
 * Kotlin mirror of packages/capture-contracts/src (all .ts modules). Field names, enum spellings and
 * discriminators match the JSON shapes exactly; contract tests validate encoded JSON against
 * packages/capture-contracts/schema (*.schema.json files).
 *
 * Mapping rules: zod `number().int()` -> Int/Long, zod `number()` -> Double, optional -> nullable
 * with default null (omitted when encoding, see CaptureJson), zod defaults -> Kotlin defaults.
 */

// ---------------------------------------------------------------- platform.ts

@Serializable
enum class Platform {
    @SerialName("windows") WINDOWS,
    @SerialName("macos") MACOS,
    @SerialName("linux") LINUX,
    @SerialName("android") ANDROID,
    @SerialName("android-tv") ANDROID_TV,
    @SerialName("google-tv") GOOGLE_TV,
    @SerialName("tizen") TIZEN,
    @SerialName("webos") WEBOS,
    @SerialName("other-tv") OTHER_TV,
}

@Serializable
enum class SupportLevel {
    @SerialName("native") NATIVE,
    @SerialName("partial") PARTIAL,
    @SerialName("companion-only") COMPANION_ONLY,
    @SerialName("external-hardware") EXTERNAL_HARDWARE,
    @SerialName("unsupported") UNSUPPORTED,
}

@Serializable
enum class FeatureKey {
    @SerialName("screenshot") SCREENSHOT,
    @SerialName("displayCapture") DISPLAY_CAPTURE,
    @SerialName("windowCapture") WINDOW_CAPTURE,
    @SerialName("regionCapture") REGION_CAPTURE,
    @SerialName("multiDisplay") MULTI_DISPLAY,
    @SerialName("videoRecording") VIDEO_RECORDING,
    @SerialName("pauseResume") PAUSE_RESUME,
    @SerialName("cursorCapture") CURSOR_CAPTURE,
    @SerialName("microphoneAudio") MICROPHONE_AUDIO,
    @SerialName("systemAudio") SYSTEM_AUDIO,
    @SerialName("separateAudioTracks") SEPARATE_AUDIO_TRACKS,
    @SerialName("h264") H264,
    @SerialName("h265") H265,
    @SerialName("hardwareEncoding") HARDWARE_ENCODING,
    @SerialName("globalShortcuts") GLOBAL_SHORTCUTS,
    @SerialName("recordingHistory") RECORDING_HISTORY,
    @SerialName("share") SHARE,
    @SerialName("hdmiCaptureDevice") HDMI_CAPTURE_DEVICE,
    @SerialName("remoteControl") REMOTE_CONTROL,
    @SerialName("deviceDiscovery") DEVICE_DISCOVERY,
    @SerialName("tvNativeScreenshot") TV_NATIVE_SCREENSHOT,
    @SerialName("fileTransfer") FILE_TRANSFER,
}

@Serializable
data class FeatureSupport(
    val level: SupportLevel,
    val reason: String? = null,
    val requires: List<PermissionKind>? = null,
    val minOsVersion: String? = null,
)

@Serializable
enum class VideoCodec(val mime: String) {
    @SerialName("h264") H264("video/avc"),
    @SerialName("h265") H265("video/hevc"),
}

@Serializable
data class EncoderInfo(
    val codec: VideoCodec,
    val name: String,
    val hardware: Boolean,
    val verified: Boolean,
)

/** Contract `FrameRate`: 24 | 30 | 60. Encoded as a plain number. */
object FrameRates {
    val ALL: List<Int> = listOf(24, 30, 60)
    fun isValid(fps: Int): Boolean = fps in ALL
}

@Serializable
enum class ImageFormat(val mime: String, val extension: String) {
    @SerialName("png") PNG("image/png", "png"),
    @SerialName("jpeg") JPEG("image/jpeg", "jpg"),
}

@Serializable
data class Size(val width: Int, val height: Int) {
    override fun toString(): String = "${width}x$height"
}

@Serializable
data class CapabilityReport(
    val schemaVersion: Int = 1,
    val platform: Platform,
    val osName: String,
    val osVersion: String,
    val deviceModel: String? = null,
    val appVersion: String,
    val generatedAt: String,
    /** Partial: absent keys are treated as `unsupported`. */
    val features: Map<FeatureKey, FeatureSupport>,
    val encoders: List<EncoderInfo>,
    val frameRates: List<Int>,
    val screenshotFormats: List<ImageFormat>,
    val maxRecordingResolution: Size? = null,
    val notes: List<String> = emptyList(),
) {
    fun level(key: FeatureKey): SupportLevel = features[key]?.level ?: SupportLevel.UNSUPPORTED

    /** True when the feature can be used directly on this device (native or partial). */
    fun isUsable(key: FeatureKey): Boolean = level(key).let { it == SupportLevel.NATIVE || it == SupportLevel.PARTIAL }
}

// ---------------------------------------------------------------- permissions.ts

@Serializable
enum class PermissionKind {
    @SerialName("screenCapture") SCREEN_CAPTURE,
    @SerialName("microphone") MICROPHONE,
    @SerialName("systemAudio") SYSTEM_AUDIO,
    @SerialName("storage") STORAGE,
    @SerialName("notifications") NOTIFICATIONS,
    @SerialName("localNetwork") LOCAL_NETWORK,
}

@Serializable
enum class PermissionState {
    @SerialName("granted") GRANTED,
    @SerialName("denied") DENIED,
    @SerialName("prompt") PROMPT,
    @SerialName("restricted") RESTRICTED,
    @SerialName("not-applicable") NOT_APPLICABLE,
    @SerialName("per-session") PER_SESSION,
}

@Serializable
data class PermissionStatus(
    val kind: PermissionKind,
    val state: PermissionState,
    val requiresSystemSettings: Boolean = false,
    val message: String? = null,
)

// ---------------------------------------------------------------- sources.ts

@Serializable
enum class SourceKind {
    @SerialName("display") DISPLAY,
    @SerialName("window") WINDOW,
    @SerialName("device") DEVICE,
}

@Serializable
data class Rect(val x: Int, val y: Int, val width: Int, val height: Int)

@Serializable
data class CaptureSource(
    val id: String,
    val kind: SourceKind,
    val name: String,
    val width: Int,
    val height: Int,
    val x: Int? = null,
    val y: Int? = null,
    val scaleFactor: Double? = null,
    val isPrimary: Boolean? = null,
    val appName: String? = null,
    val deviceDescriptor: String? = null,
)

@Serializable
@JsonClassDiscriminator("kind")
sealed interface CaptureTarget {
    @Serializable @SerialName("display")
    data class Display(val displayId: String) : CaptureTarget

    @Serializable @SerialName("window")
    data class Window(val windowId: String) : CaptureTarget

    @Serializable @SerialName("region")
    data class Region(val displayId: String, val rect: Rect) : CaptureTarget

    @Serializable @SerialName("device")
    data class Device(val deviceId: String) : CaptureTarget
}

// ---------------------------------------------------------------- errors.ts

@Serializable
enum class CaptureErrorCode(val userMessage: String) {
    PERMISSION_DENIED("Permission was denied. Grant access in system settings and try again."),
    UNSUPPORTED("This feature is not supported on this device."),
    SOURCE_NOT_FOUND("The selected screen or window is no longer available."),
    SOURCE_CLOSED("The captured screen or window was closed. The recording was saved up to that point."),
    ENCODER_UNAVAILABLE("No suitable video encoder is available. Try H.264 or a lower resolution."),
    ENCODER_FAILED("The video encoder failed. Try a lower resolution or frame rate."),
    STORAGE_FULL("Not enough free disk space. Free up space or choose another output folder."),
    IO_ERROR("A file could not be read or written."),
    INVALID_CONFIG("The recording settings are invalid."),
    INVALID_STATE("That action is not available right now."),
    DRM_PROTECTED("Protected content cannot be captured."),
    INTERRUPTED("The recording was interrupted. The partial recording was saved when possible."),
    DEVICE_BUSY("The capture device is in use by another application."),
    NETWORK("Network error while communicating with the device."),
    INTERNAL("An unexpected error occurred."),
}

@Serializable
data class CaptureError(
    val code: CaptureErrorCode,
    val message: String,
    val recoverable: Boolean,
    val details: JsonObject? = null,
)

// ---------------------------------------------------------------- recording.ts

@Serializable
@JsonClassDiscriminator("mode")
sealed interface ResolutionSetting {
    /** Record at the source's native pixel size. */
    @Serializable @SerialName("source")
    data object Source : ResolutionSetting

    /** Scale to the given height (2160|1440|1080|720|480), preserving aspect ratio. Never upscales. */
    @Serializable @SerialName("height")
    data class Height(val height: Int) : ResolutionSetting {
        init { require(height in HEIGHTS) { "height must be one of $HEIGHTS" } }
        companion object { val HEIGHTS = listOf(2160, 1440, 1080, 720, 480) }
    }

    /** Exact output box (letterboxed to preserve aspect). Never upscales. */
    @Serializable @SerialName("custom")
    data class Custom(val width: Int, val height: Int) : ResolutionSetting {
        init { require(width in 16..7680 && height in 16..4320) { "custom size out of range" } }
    }
}

@Serializable
enum class QualityPreset {
    @SerialName("low") LOW,
    @SerialName("medium") MEDIUM,
    @SerialName("high") HIGH,
    @SerialName("ultra") ULTRA,
    @SerialName("custom") CUSTOM,
}

@Serializable
enum class HardwareAcceleration {
    @SerialName("auto") AUTO,
    @SerialName("prefer") PREFER,
    @SerialName("off") OFF,
}

@Serializable
data class VideoSettings(
    val codec: VideoCodec = VideoCodec.H264,
    val resolution: ResolutionSetting = ResolutionSetting.Source,
    val fps: Int = 30,
    val quality: QualityPreset = QualityPreset.MEDIUM,
    /** Required when quality == CUSTOM. 250..200000. */
    val bitrateKbps: Int? = null,
    val keyframeIntervalSec: Double = 2.0,
    val hardwareAcceleration: HardwareAcceleration = HardwareAcceleration.AUTO,
    val showCursor: Boolean = true,
)

@Serializable
data class AudioSourceSettings(
    val enabled: Boolean,
    val deviceId: String? = null,
    /** Linear gain, 0..2. */
    val gain: Double = 1.0,
)

@Serializable
enum class AudioTrackMode {
    @SerialName("mixed") MIXED,
    @SerialName("separate") SEPARATE,
}

@Serializable
data class AudioSettings(
    val microphone: AudioSourceSettings = AudioSourceSettings(enabled = false),
    val system: AudioSourceSettings = AudioSourceSettings(enabled = false),
    val trackMode: AudioTrackMode = AudioTrackMode.MIXED,
    /** 44100 | 48000 */
    val sampleRate: Int = 48000,
    /** 96 | 128 | 160 | 192 | 256 */
    val bitrateKbps: Int = 160,
)

@Serializable
data class OutputSettings(
    val directory: String,
    val fileNameTemplate: String = "CaptureOne {date} {time}",
    val container: String = "mp4",
)

@Serializable
data class RecordingLimits(
    val maxDurationSec: Int? = null,
    val maxFileSizeMb: Int? = null,
    val minFreeDiskMb: Int = 500,
)

@Serializable
data class RecordingConfig(
    val target: CaptureTarget,
    val video: VideoSettings,
    val audio: AudioSettings,
    val output: OutputSettings,
    val limits: RecordingLimits = RecordingLimits(),
    /** 0 | 3 | 5 | 10 */
    val countdownSec: Int = 3,
) {
    /** Mirrors the zod refinements of RecordingConfig; returns the first violation or null. */
    fun validate(): CaptureError? {
        fun invalid(msg: String) = CaptureError(CaptureErrorCode.INVALID_CONFIG, msg, recoverable = false)
        if (!FrameRates.isValid(video.fps)) return invalid("fps must be one of ${FrameRates.ALL}")
        if (video.quality == QualityPreset.CUSTOM && video.bitrateKbps == null) {
            return invalid("bitrateKbps is required when quality is 'custom'")
        }
        video.bitrateKbps?.let { if (it !in 250..200_000) return invalid("bitrateKbps must be 250..200000") }
        if (video.keyframeIntervalSec !in 0.5..10.0) return invalid("keyframeIntervalSec must be 0.5..10")
        if (audio.sampleRate != 44100 && audio.sampleRate != 48000) return invalid("sampleRate must be 44100 or 48000")
        if (audio.bitrateKbps !in listOf(96, 128, 160, 192, 256)) return invalid("audio bitrateKbps must be 96|128|160|192|256")
        if (audio.microphone.gain !in 0.0..2.0 || audio.system.gain !in 0.0..2.0) return invalid("gain must be 0..2")
        if (countdownSec !in listOf(0, 3, 5, 10)) return invalid("countdownSec must be 0, 3, 5 or 10")
        if (output.directory.isEmpty()) return invalid("output.directory must not be empty")
        if (output.fileNameTemplate.isEmpty()) return invalid("output.fileNameTemplate must not be empty")
        if (output.container != "mp4") return invalid("only the mp4 container is supported")
        limits.maxDurationSec?.let { if (it <= 0) return invalid("maxDurationSec must be positive") }
        limits.maxFileSizeMb?.let { if (it <= 0) return invalid("maxFileSizeMb must be positive") }
        if (limits.minFreeDiskMb < 0) return invalid("minFreeDiskMb must be >= 0")
        return null
    }
}

@Serializable
enum class RecordingState {
    @SerialName("idle") IDLE,
    @SerialName("countdown") COUNTDOWN,
    @SerialName("starting") STARTING,
    @SerialName("recording") RECORDING,
    @SerialName("paused") PAUSED,
    @SerialName("stopping") STOPPING,
    @SerialName("finalizing") FINALIZING,
    @SerialName("completed") COMPLETED,
    @SerialName("failed") FAILED;

    val wireName: String get() = name.lowercase()
    val isActive: Boolean get() = this == RECORDING || this == PAUSED
}

@Serializable
data class RecordingStatus(
    val sessionId: String,
    val state: RecordingState,
    /** Recorded (non-paused) time. */
    val elapsedMs: Double,
    val countdownRemainingSec: Int? = null,
    val bytesWritten: Long,
    val framesEncoded: Long,
    val framesDropped: Long,
    val estimatedBytesPerMinute: Double? = null,
    val freeDiskBytes: Long? = null,
    val outputPath: String? = null,
    val encoder: String? = null,
    val warnings: List<String> = emptyList(),
    val error: CaptureError? = null,
)

@Serializable
enum class StopReason {
    @SerialName("user") USER,
    @SerialName("max-duration") MAX_DURATION,
    @SerialName("max-file-size") MAX_FILE_SIZE,
    @SerialName("low-disk") LOW_DISK,
    @SerialName("source-closed") SOURCE_CLOSED,
    @SerialName("error") ERROR,
    @SerialName("interrupted") INTERRUPTED,
}

// ---------------------------------------------------------------- media.ts

@Serializable
data class AudioTrackInfo(
    val index: Int,
    val codec: String,
    val channels: Int,
    val sampleRate: Int,
    val label: String? = null,
    val durationMs: Double? = null,
)

@Serializable
enum class IssueSeverity {
    @SerialName("error") ERROR,
    @SerialName("warning") WARNING,
}

@Serializable
data class ValidationIssue(val severity: IssueSeverity, val code: String, val message: String)

@Serializable
data class MediaValidation(
    /** True only if the file was inspected after writing and had no errors. */
    val valid: Boolean,
    val checkedAt: String,
    val tool: String,
    val issues: List<ValidationIssue>,
)

@Serializable
enum class MediaKind {
    @SerialName("video") VIDEO,
    @SerialName("screenshot") SCREENSHOT,
}

@Serializable
data class MediaMetadata(
    val id: String,
    val kind: MediaKind,
    val path: String,
    val fileName: String,
    val title: String,
    val createdAt: String,
    val sizeBytes: Long,
    val width: Int,
    val height: Int,
    val durationMs: Double? = null,
    val fps: Double? = null,
    val videoCodec: VideoCodec? = null,
    val videoBitrateKbps: Double? = null,
    val audioTracks: List<AudioTrackInfo> = emptyList(),
    val imageFormat: ImageFormat? = null,
    /** "mp4" | "png" | "jpeg" */
    val container: String,
    val sourceName: String? = null,
    val platform: String? = null,
    val validation: MediaValidation? = null,
    val thumbnailPath: String? = null,
)

// ---------------------------------------------------------------- screenshot.ts

@Serializable
data class ScreenshotRequest(
    val target: CaptureTarget,
    val format: ImageFormat = ImageFormat.PNG,
    val jpegQuality: Int = 90,
    val outputDirectory: String? = null,
    val fileNameTemplate: String? = null,
    val showCursor: Boolean = true,
    val delaySec: Int = 0,
)

// ---------------------------------------------------------------- events.ts

@Serializable
@JsonClassDiscriminator("type")
sealed interface CaptureEvent {
    @Serializable @SerialName("state-changed")
    data class StateChanged(val sessionId: String, val from: RecordingState, val to: RecordingState, val at: String) : CaptureEvent

    @Serializable @SerialName("status")
    data class Status(val status: RecordingStatus) : CaptureEvent

    @Serializable @SerialName("countdown")
    data class Countdown(val sessionId: String, val remainingSec: Int) : CaptureEvent

    @Serializable @SerialName("warning")
    data class Warning(val sessionId: String? = null, val message: String) : CaptureEvent

    @Serializable @SerialName("error")
    data class Error(val sessionId: String? = null, val error: CaptureError) : CaptureEvent

    @Serializable @SerialName("completed")
    data class Completed(val sessionId: String, val reason: StopReason, val media: MediaMetadata) : CaptureEvent

    @Serializable @SerialName("screenshot")
    data class Screenshot(val media: MediaMetadata) : CaptureEvent
}
