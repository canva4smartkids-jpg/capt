package dev.captureone.capture.capability

import android.Manifest
import android.app.Activity
import android.app.NotificationManager
import android.app.UiModeManager
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.media.projection.MediaProjectionManager
import android.os.Build
import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.PermissionKind
import dev.captureone.capture.contract.PermissionState
import dev.captureone.capture.contract.PermissionStatus
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.engine.DisplayInfo
import dev.captureone.capture.engine.EncoderCatalog
import dev.captureone.capture.settings.SettingsStore
import java.time.Instant

/** Collects [DeviceFacts] on a real device (no guessing: every value comes from an OS API). */
object DeviceFactsCollector {
    /** Google TV launcher; its presence distinguishes Google TV from plain Android TV (heuristic). */
    private const val GOOGLE_TV_LAUNCHER = "com.google.android.apps.tv.launcherx"

    fun isTelevision(context: Context): Boolean {
        val ui = context.getSystemService(UiModeManager::class.java)
        return ui?.currentModeType == Configuration.UI_MODE_TYPE_TELEVISION ||
            context.packageManager.hasSystemFeature(PackageManager.FEATURE_LEANBACK)
    }

    fun formFactor(context: Context): FormFactor = when {
        isTelevision(context) -> FormFactor.TV
        context.resources.configuration.smallestScreenWidthDp >= 600 -> FormFactor.TABLET
        else -> FormFactor.PHONE
    }

    private fun isGoogleTv(context: Context): Boolean = try {
        context.packageManager.getPackageInfo(GOOGLE_TV_LAUNCHER, 0)
        true
    } catch (e: PackageManager.NameNotFoundException) {
        false
    }

    /** The intent the UI launches for consent (exposed so the check and the launch use the same intent). */
    fun screenCaptureIntent(context: Context): Intent =
        context.getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent()

    /** Whether `createScreenCaptureIntent()` resolves to an activity on this device. */
    fun consentActivityAvailable(context: Context): Boolean = try {
        screenCaptureIntent(context).resolveActivity(context.packageManager) != null
    } catch (e: Exception) {
        false
    }

    fun collect(context: Context, appVersion: String): DeviceFacts {
        val verified = SettingsStore(context).verifiedEncoders()
        val encoders = EncoderCatalog.facts(verified)
        val display = runCatching { DisplayInfo.of(context) }.getOrNull()
        val maxSize = display?.let { d ->
            EncoderCatalog.videoEncoders().filter { it.codec == VideoCodec.H264 }
                .mapNotNull { it.fit(d.size.let { s -> Size(s.width / 2 * 2, s.height / 2 * 2) }) }
                .maxByOrNull { it.width.toLong() * it.height }
        }
        return DeviceFacts(
            sdkInt = Build.VERSION.SDK_INT,
            osRelease = Build.VERSION.RELEASE ?: "",
            manufacturer = Build.MANUFACTURER ?: "",
            model = Build.MODEL ?: "",
            appVersion = appVersion,
            formFactor = formFactor(context),
            isGoogleTv = isTelevision(context) && isGoogleTv(context),
            consentActivityAvailable = consentActivityAvailable(context),
            hasMicrophone = context.packageManager.hasSystemFeature(PackageManager.FEATURE_MICROPHONE),
            encoders = encoders,
            displaySize = display?.size,
            maxRecordingSize = maxSize,
            generatedAt = Instant.now().toString(),
        )
    }

    fun appVersion(context: Context): String = try {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "0"
    } catch (e: PackageManager.NameNotFoundException) {
        "0"
    }

    /** Honest per-device capability report (contract `CapabilityReport`). Call off the main thread. */
    fun capabilityReport(context: Context): CapabilityReport = CapabilityClassifier.report(collect(context, appVersion(context)))
}

/** Runtime permission states in contract terms. */
object PermissionInspector {
    private fun granted(context: Context, p: String) = context.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    private fun runtime(context: Context, activity: Activity?, kind: PermissionKind, permission: String, explanation: String): PermissionStatus {
        if (granted(context, permission)) return PermissionStatus(kind, PermissionState.GRANTED)
        val asked = SettingsStore(context).wasRequested(permission)
        val rationale = activity?.shouldShowRequestPermissionRationale(permission) == true
        return when {
            !asked -> PermissionStatus(kind, PermissionState.PROMPT, message = explanation)
            rationale -> PermissionStatus(kind, PermissionState.DENIED, message = explanation)
            // Asked before and no rationale: "don't ask again" – only system settings can change it.
            else -> PermissionStatus(kind, PermissionState.DENIED, requiresSystemSettings = true, message = "$explanation Enable it in system settings.")
        }
    }

    fun status(context: Context, kind: PermissionKind, activity: Activity? = null): PermissionStatus = when (kind) {
        PermissionKind.SCREEN_CAPTURE -> if (DeviceFactsCollector.consentActivityAvailable(context)) {
            PermissionStatus(kind, PermissionState.PER_SESSION, message = "Android asks for consent each time a recording or screenshot starts.")
        } else {
            PermissionStatus(kind, PermissionState.RESTRICTED, message = "This device has no screen-capture consent screen; native capture is not possible.")
        }
        PermissionKind.MICROPHONE -> if (!context.packageManager.hasSystemFeature(PackageManager.FEATURE_MICROPHONE)) {
            PermissionStatus(kind, PermissionState.NOT_APPLICABLE, message = "No microphone on this device.")
        } else {
            runtime(context, activity, kind, Manifest.permission.RECORD_AUDIO, "Needed to record your voice.")
        }
        PermissionKind.SYSTEM_AUDIO -> if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            PermissionStatus(kind, PermissionState.NOT_APPLICABLE, message = "Internal audio capture requires Android 10.")
        } else {
            // Playback capture = RECORD_AUDIO + the per-session MediaProjection consent.
            val mic = runtime(context, activity, kind, Manifest.permission.RECORD_AUDIO, "Internal audio capture uses the microphone permission (RECORD_AUDIO) together with screen-capture consent.")
            if (mic.state == PermissionState.GRANTED) PermissionStatus(kind, PermissionState.PER_SESSION, message = "Granted with each screen-capture consent. Apps that opt out stay silent.") else mic
        }
        PermissionKind.STORAGE -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            PermissionStatus(kind, PermissionState.NOT_APPLICABLE, message = "Android 10+ saves to Movies/Pictures without a storage permission.")
        } else {
            @Suppress("DEPRECATION")
            runtime(context, activity, kind, Manifest.permission.WRITE_EXTERNAL_STORAGE, "Needed on Android 9 and older to save into Movies/Pictures.")
        }
        PermissionKind.NOTIFICATIONS -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            runtime(context, activity, kind, Manifest.permission.POST_NOTIFICATIONS, "Shows the recording controls (timer, pause, stop) in the notification shade.")
        } else {
            val enabled = context.getSystemService(NotificationManager::class.java).areNotificationsEnabled()
            if (enabled) PermissionStatus(kind, PermissionState.GRANTED) else PermissionStatus(kind, PermissionState.DENIED, true, "Notifications are disabled for CaptureOne in system settings.")
        }
        PermissionKind.LOCAL_NETWORK -> PermissionStatus(kind, PermissionState.NOT_APPLICABLE, message = "Plain LAN connections need no runtime permission on Android.")
    }

    /** The Android permission string to request for [kind], or null if nothing can be requested. */
    fun requestablePermission(kind: PermissionKind): String? = when (kind) {
        PermissionKind.MICROPHONE, PermissionKind.SYSTEM_AUDIO -> Manifest.permission.RECORD_AUDIO
        PermissionKind.NOTIFICATIONS -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) Manifest.permission.POST_NOTIFICATIONS else null
        @Suppress("DEPRECATION")
        PermissionKind.STORAGE -> if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) Manifest.permission.WRITE_EXTERNAL_STORAGE else null
        else -> null
    }
}
