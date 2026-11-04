package dev.captureone.capture.capability

import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.EncoderInfo
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.FeatureSupport
import dev.captureone.capture.contract.FrameRates
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.PermissionKind
import dev.captureone.capture.contract.Platform
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.SupportLevel
import dev.captureone.capture.contract.SupportLevel.COMPANION_ONLY
import dev.captureone.capture.contract.SupportLevel.EXTERNAL_HARDWARE
import dev.captureone.capture.contract.SupportLevel.NATIVE
import dev.captureone.capture.contract.SupportLevel.PARTIAL
import dev.captureone.capture.contract.SupportLevel.UNSUPPORTED
import dev.captureone.capture.contract.VideoCodec

enum class FormFactor { PHONE, TABLET, TV }

/** One encoder as found in `MediaCodecList` on the device. */
data class EncoderFacts(
    val codec: VideoCodec,
    val name: String,
    /** `MediaCodecInfo.isHardwareAccelerated` (API 29+); null when the OS does not report it. */
    val hardware: Boolean?,
    val maxWidth: Int,
    val maxHeight: Int,
    /** True once a recording produced with this encoder was validated on this device. */
    val verified: Boolean,
)

/** Everything the classifier needs, collected on device by [DeviceFactsCollector]. */
data class DeviceFacts(
    val sdkInt: Int,
    val osRelease: String,
    val manufacturer: String,
    val model: String,
    val appVersion: String,
    val formFactor: FormFactor,
    val isGoogleTv: Boolean,
    /** `createScreenCaptureIntent()` resolves to an activity (many TV builds lack SystemUI's consent dialog). */
    val consentActivityAvailable: Boolean,
    val hasMicrophone: Boolean,
    val encoders: List<EncoderFacts>,
    val displaySize: Size?,
    /** Largest size the best AVC encoder accepts for this display (aspect preserved). */
    val maxRecordingSize: Size?,
    val generatedAt: String,
)

/** Tiers used by the TV capability screen. */
enum class NativeCaptureTier { SUPPORTED, PARTIAL, NOT_AVAILABLE }

data class NativeCaptureAssessment(val tier: NativeCaptureTier, val reasons: List<String>)

/**
 * Honest, pure classification of what this device can do. Never reports `native` for something the
 * OS does not expose to ordinary apps.
 */
object CapabilityClassifier {
    const val MIN_SDK_PLAYBACK_CAPTURE = 29
    const val MIN_SDK_HW_FLAG = 29
    const val MIN_SDK_SINGLE_APP_CAPTURE = 34
    const val MIN_SDK_MAX_FPS_TO_ENCODER = 29

    const val DRM_NOTE =
        "Protected content (apps using FLAG_SECURE, DRM/HDCP video) is blanked to black by Android. CaptureOne does not and cannot bypass this."

    fun hasAvc(f: DeviceFacts) = f.encoders.any { it.codec == VideoCodec.H264 }
    fun hasHevc(f: DeviceFacts) = f.encoders.any { it.codec == VideoCodec.H265 }
    fun canCaptureScreen(f: DeviceFacts) = f.consentActivityAvailable
    fun canRecordVideo(f: DeviceFacts) = f.consentActivityAvailable && hasAvc(f)
    fun canCaptureSystemAudio(f: DeviceFacts) = canRecordVideo(f) && f.sdkInt >= MIN_SDK_PLAYBACK_CAPTURE

    /** Best-effort guess when the OS does not report the hardware flag (API < 29). */
    fun looksLikeSoftwareCodec(name: String): Boolean {
        val n = name.lowercase()
        return n.startsWith("omx.google.") || n.startsWith("c2.android.") || n.contains(".sw.") || n.endsWith(".sw")
    }

    fun nativeCaptureAssessment(f: DeviceFacts): NativeCaptureAssessment {
        val reasons = mutableListOf<String>()
        if (!f.consentActivityAvailable) {
            reasons += "This device has no MediaProjection consent screen (createScreenCaptureIntent does not resolve to an activity), so no app can capture the screen."
        }
        if (!hasAvc(f)) reasons += "No H.264 video encoder was found."
        if (reasons.isNotEmpty()) {
            reasons += "Use companion mode (control a CaptureOne desktop hub) or record this device's HDMI output with a capture card."
            return NativeCaptureAssessment(NativeCaptureTier.NOT_AVAILABLE, reasons)
        }
        if (!canCaptureSystemAudio(f)) {
            reasons += "Internal audio capture needs Android 10 (API 29); this device runs API ${f.sdkInt}. Video only (plus microphone if present)."
        } else {
            reasons += "Internal audio: only apps that allow playback capture (media/game usage) are recorded."
        }
        if (!f.hasMicrophone) reasons += "No microphone on this device."
        reasons += DRM_NOTE
        val tier = if (canCaptureSystemAudio(f)) NativeCaptureTier.SUPPORTED else NativeCaptureTier.PARTIAL
        return NativeCaptureAssessment(tier, reasons)
    }

    fun platform(f: DeviceFacts): Platform = when {
        f.formFactor != FormFactor.TV -> Platform.ANDROID
        f.isGoogleTv -> Platform.GOOGLE_TV
        else -> Platform.ANDROID_TV
    }

    fun report(f: DeviceFacts): CapabilityReport {
        val tv = f.formFactor == FormFactor.TV
        val features = linkedMapOf<FeatureKey, FeatureSupport>()
        val screen = listOf(PermissionKind.SCREEN_CAPTURE)
        val screenAndNotif = listOf(PermissionKind.SCREEN_CAPTURE, PermissionKind.NOTIFICATIONS)
        val noConsent = "No MediaProjection consent activity on this device; record the HDMI output with a capture card on the CaptureOne desktop app, or use companion mode."

        // Screen capture (screenshot / display / video)
        if (canCaptureScreen(f)) {
            val level = if (tv) PARTIAL else NATIVE
            val reason = if (tv) "MediaProjection works, but DRM/HDCP-protected video (most streaming apps) is captured as black." else null
            features[FeatureKey.SCREENSHOT] = FeatureSupport(level, reason ?: "Requires per-session screen-capture consent.", screen, "Android 5.0")
            features[FeatureKey.DISPLAY_CAPTURE] = FeatureSupport(level, reason, screenAndNotif, "Android 5.0")
        } else {
            val level = if (tv) EXTERNAL_HARDWARE else UNSUPPORTED
            features[FeatureKey.SCREENSHOT] = FeatureSupport(level, noConsent)
            features[FeatureKey.DISPLAY_CAPTURE] = FeatureSupport(level, noConsent)
        }
        features[FeatureKey.VIDEO_RECORDING] = when {
            canRecordVideo(f) -> features.getValue(FeatureKey.DISPLAY_CAPTURE)
            !f.consentActivityAvailable -> features.getValue(FeatureKey.DISPLAY_CAPTURE)
            else -> FeatureSupport(UNSUPPORTED, "No H.264 encoder available.")
        }
        val video = canRecordVideo(f)

        features[FeatureKey.WINDOW_CAPTURE] = if (video && !tv && f.sdkInt >= MIN_SDK_SINGLE_APP_CAPTURE) {
            FeatureSupport(PARTIAL, "Android 14+: the user can choose a single app in the system consent dialog; apps cannot pick a window programmatically.", screen, "Android 14")
        } else {
            FeatureSupport(UNSUPPORTED, "Single-app capture requires Android 14 on phones/tablets.", minOsVersion = "Android 14")
        }
        features[FeatureKey.REGION_CAPTURE] = FeatureSupport(UNSUPPORTED, "Not implemented on Android; the full display is recorded.")
        features[FeatureKey.MULTI_DISPLAY] = FeatureSupport(UNSUPPORTED, "MediaProjection mirrors the default display only.")
        features[FeatureKey.PAUSE_RESUME] = if (video) FeatureSupport(NATIVE) else FeatureSupport(UNSUPPORTED, "Requires video recording.")
        features[FeatureKey.CURSOR_CAPTURE] = FeatureSupport(
            UNSUPPORTED,
            "Android offers no API to control the pointer/touch overlay. Enable Developer options › Show taps to make touches visible.",
        )

        // Audio
        features[FeatureKey.MICROPHONE_AUDIO] = when {
            !video -> FeatureSupport(UNSUPPORTED, "Requires video recording.")
            f.hasMicrophone -> FeatureSupport(NATIVE, requires = listOf(PermissionKind.MICROPHONE))
            else -> FeatureSupport(UNSUPPORTED, "This device has no microphone.")
        }
        features[FeatureKey.SYSTEM_AUDIO] = if (canCaptureSystemAudio(f)) {
            FeatureSupport(
                PARTIAL,
                "AudioPlaybackCapture: only media, game and unknown usages; apps can opt out (allowAudioPlaybackCapture=false, or by default when targeting < API 29); calls are never captured. Needs the screen-capture session and the microphone permission.",
                listOf(PermissionKind.SCREEN_CAPTURE, PermissionKind.MICROPHONE),
                "Android 10",
            )
        } else {
            FeatureSupport(UNSUPPORTED, if (video) "Internal audio capture requires Android 10 (API 29)." else "Requires video recording.", minOsVersion = "Android 10")
        }
        features[FeatureKey.SEPARATE_AUDIO_TRACKS] = if (canCaptureSystemAudio(f) && f.hasMicrophone) {
            FeatureSupport(PARTIAL, "One AAC track per source (microphone, internal). Many players only play the first audio track.")
        } else {
            FeatureSupport(UNSUPPORTED, "Requires both microphone and internal audio.")
        }

        // Codecs
        features[FeatureKey.H264] = if (hasAvc(f)) FeatureSupport(NATIVE) else FeatureSupport(UNSUPPORTED, "No H.264 encoder.")
        features[FeatureKey.H265] = if (hasHevc(f)) FeatureSupport(NATIVE) else FeatureSupport(UNSUPPORTED, "No HEVC encoder on this device.")
        val hwKnown = f.encoders.filter { it.hardware != null }
        features[FeatureKey.HARDWARE_ENCODING] = when {
            hwKnown.any { it.hardware == true } -> FeatureSupport(NATIVE)
            f.encoders.any { it.hardware == null && !looksLikeSoftwareCodec(it.name) } ->
                FeatureSupport(PARTIAL, "Android < 10 does not report whether a codec is hardware accelerated; inferred from the codec name.", minOsVersion = "Android 10")
            else -> FeatureSupport(UNSUPPORTED, "Only software encoders were found.")
        }

        // Product features
        features[FeatureKey.GLOBAL_SHORTCUTS] = FeatureSupport(
            UNSUPPORTED,
            if (tv) "Android TV has no global shortcuts; use the CaptureOne app or notification." else "Android has no global keyboard shortcuts; use the Quick Settings tile or the notification actions.",
        )
        features[FeatureKey.RECORDING_HISTORY] = FeatureSupport(NATIVE)
        features[FeatureKey.SHARE] = if (tv) FeatureSupport(PARTIAL, "Android TV devices rarely have share targets.") else FeatureSupport(NATIVE)
        features[FeatureKey.HDMI_CAPTURE_DEVICE] = if (tv) {
            FeatureSupport(EXTERNAL_HARDWARE, "Connect the TV's HDMI output to a capture card attached to the CaptureOne desktop app.")
        } else {
            FeatureSupport(UNSUPPORTED, "USB/UVC capture cards are not supported by the Android app.")
        }
        features[FeatureKey.REMOTE_CONTROL] = if (tv) {
            FeatureSupport(NATIVE, "Companion mode: controls a paired CaptureOne desktop hub (start/pause/resume/stop/screenshot).", listOf(PermissionKind.LOCAL_NETWORK))
        } else {
            FeatureSupport(UNSUPPORTED, "Companion mode is available in the TV app.")
        }
        features[FeatureKey.DEVICE_DISCOVERY] = FeatureSupport(UNSUPPORTED, "Hubs are added by IP address; SSDP/mDNS discovery is not implemented on Android.")
        if (tv) {
            features[FeatureKey.TV_NATIVE_SCREENSHOT] = if (canCaptureScreen(f)) {
                FeatureSupport(PARTIAL, "Via MediaProjection; DRM/HDCP-protected video is black.", screen)
            } else {
                FeatureSupport(UNSUPPORTED, noConsent)
            }
            features[FeatureKey.FILE_TRANSFER] = FeatureSupport(PARTIAL, "Recordings on a paired desktop hub can be streamed for playback; no download to the TV.")
        } else {
            features[FeatureKey.TV_NATIVE_SCREENSHOT] = FeatureSupport(UNSUPPORTED, "Not a TV.")
            features[FeatureKey.FILE_TRANSFER] = FeatureSupport(UNSUPPORTED, "Not implemented in the phone app.")
        }

        val encoders = f.encoders.map {
            EncoderInfo(codec = it.codec, name = it.name, hardware = it.hardware ?: !looksLikeSoftwareCodec(it.name), verified = it.verified)
        }
        val notes = buildList {
            add(DRM_NOTE)
            if (video && f.sdkInt < MIN_SDK_MAX_FPS_TO_ENCODER) {
                add("Android < 10 cannot cap the encoder frame rate (KEY_MAX_FPS_TO_ENCODER); frames arrive as the screen updates, up to the display refresh rate.")
            }
            if (f.encoders.any { it.hardware == null }) add("Encoder 'hardware' flags are inferred from codec names on Android < 10.")
            add("An encoder is marked verified only after a recording made with it was validated (MediaExtractor) on this device.")
            if (!f.consentActivityAvailable) add(noConsent)
        }
        return CapabilityReport(
            schemaVersion = 1,
            platform = platform(f),
            osName = "Android",
            osVersion = "${f.osRelease} (API ${f.sdkInt})",
            deviceModel = "${f.manufacturer} ${f.model}".trim(),
            appVersion = f.appVersion,
            generatedAt = f.generatedAt,
            features = features,
            encoders = encoders,
            frameRates = if (video) FrameRates.ALL else emptyList(),
            screenshotFormats = if (canCaptureScreen(f)) listOf(ImageFormat.PNG, ImageFormat.JPEG) else emptyList(),
            maxRecordingResolution = if (video) f.maxRecordingSize else null,
            notes = notes,
        )
    }

    /** Companion device kind to announce when pairing with a hub. */
    fun deviceKind(f: DeviceFacts): dev.captureone.capture.contract.DeviceKind = when (f.formFactor) {
        FormFactor.PHONE -> dev.captureone.capture.contract.DeviceKind.ANDROID_PHONE
        FormFactor.TABLET -> dev.captureone.capture.contract.DeviceKind.ANDROID_TABLET
        FormFactor.TV -> if (f.isGoogleTv) dev.captureone.capture.contract.DeviceKind.GOOGLE_TV else dev.captureone.capture.contract.DeviceKind.ANDROID_TV
    }
}
