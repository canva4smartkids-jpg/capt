package dev.captureone.capture.settings

import dev.captureone.capture.contract.AudioSettings
import dev.captureone.capture.contract.AudioSourceSettings
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.CaptureTarget
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.OutputSettings
import dev.captureone.capture.contract.RecordingConfig
import dev.captureone.capture.contract.RecordingLimits
import dev.captureone.capture.contract.VideoSettings
import kotlinx.serialization.Serializable

/**
 * Persisted user defaults. Field names and defaults follow `AppSettings` in
 * packages/settings/src/app-settings.ts (the subset that is meaningful on Android).
 */
@Serializable
data class CaptureSettings(
    val schemaVersion: Int = 1,
    val fileNameTemplate: String = "CaptureOne {date} {time}",
    val video: VideoSettings = VideoSettings(),
    val audio: AudioSettings = AudioSettings(
        microphone = AudioSourceSettings(enabled = false),
        system = AudioSourceSettings(enabled = false),
    ),
    val screenshotFormat: ImageFormat = ImageFormat.PNG,
    val jpegQuality: Int = 90,
    val countdownSec: Int = 3,
    val maxDurationSec: Int? = null,
    val maxFileSizeMb: Int? = null,
    val minFreeDiskMb: Int = 500,
) {
    /** Builds the recording config for the default display. [outputDirectory] is the temp/working dir. */
    fun toRecordingConfig(outputDirectory: String): RecordingConfig = RecordingConfig(
        target = CaptureTarget.Display(DEFAULT_DISPLAY_ID),
        video = video,
        audio = audio,
        output = OutputSettings(directory = outputDirectory, fileNameTemplate = fileNameTemplate),
        limits = RecordingLimits(maxDurationSec = maxDurationSec, maxFileSizeMb = maxFileSizeMb, minFreeDiskMb = minFreeDiskMb),
        countdownSec = countdownSec,
    )

    fun toJson(): String = CaptureJson.encodeToString(serializer(), this)

    companion object {
        /** `Display.DEFAULT_DISPLAY` as a stable string id. MediaProjection mirrors only this display. */
        const val DEFAULT_DISPLAY_ID = "0"
        val COUNTDOWN_CHOICES = listOf(0, 3, 5, 10)

        /** Parses persisted settings, falling back to defaults for invalid content (mirror of `loadSettings`). */
        fun fromJson(text: String?): CaptureSettings {
            if (text.isNullOrBlank()) return CaptureSettings()
            return try {
                val s = CaptureJson.decodeFromString(serializer(), text)
                if (s.countdownSec !in COUNTDOWN_CHOICES || s.jpegQuality !in 1..100) CaptureSettings() else s
            } catch (_: Exception) {
                CaptureSettings()
            }
        }
    }
}
