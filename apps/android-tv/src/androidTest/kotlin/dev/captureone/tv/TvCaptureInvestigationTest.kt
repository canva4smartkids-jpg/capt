package dev.captureone.tv

import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.ResolutionSetting
import dev.captureone.capture.contract.SupportLevel
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.service.CaptureController
import dev.captureone.capture.settings.CaptureSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

/**
 * Investigation (always passes unless the app's own behaviour is inconsistent): does MediaProjection work
 * on this Android TV image? Evidence is logged under tag CaptureOneTV (lines prefixed TVFACT).
 */
@RunWith(AndroidJUnit4::class)
class TvCaptureInvestigationTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext

    private fun shell(cmd: String): String =
        instrumentation.uiAutomation.executeShellCommand(cmd).use { pfd ->
            android.os.ParcelFileDescriptor.AutoCloseInputStream(pfd).bufferedReader().readText()
        }

    private fun fact(s: String) = Log.i(TAG, "TVFACT $s")

    private fun consent(then: (Activity, Int, Intent) -> Unit): Int {
        val f = CompletableFuture<Int>()
        TvConsentProxyActivity.pending = f
        TvConsentProxyActivity.onResult = then
        context.startActivity(Intent(context, TvConsentProxyActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        return try {
            f.get(20, TimeUnit.SECONDS)
        } catch (e: java.util.concurrent.TimeoutException) {
            Activity.RESULT_CANCELED
        } finally {
            TvConsentProxyActivity.onResult = null
        }
    }

    private fun awaitState(vararg states: RecordingState, timeoutMs: Long = 20_000) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            val s = CaptureController.status.value
            if (s != null && s.state in states) return
            if (s?.state == RecordingState.FAILED) throw AssertionError("session failed: ${CaptureController.lastError.value}")
            Thread.sleep(50)
        }
        throw AssertionError("timed out waiting for ${states.toList()}; status=${CaptureController.status.value}; error=${CaptureController.lastError.value}")
    }

    private fun awaitMedia(kind: MediaKind, previous: MediaMetadata?, timeoutMs: Long = 30_000): MediaMetadata {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            val m = CaptureController.lastMedia.value
            if (m != null && m != previous && m.kind == kind) return m
            CaptureController.lastError.value?.let { throw AssertionError("capture error: $it") }
            Thread.sleep(100)
        }
        throw AssertionError("no $kind result; status=${CaptureController.status.value}; error=${CaptureController.lastError.value}")
    }

    @Test
    fun investigateMediaProjectionOnTv() {
        try { investigate() } catch (t: Throwable) { fact("RESULT=TEST_EXCEPTION $t"); throw t }
    }

    private fun investigate() {
        val pkg = context.packageName
        val pm = context.packageManager
        val report: CapabilityReport = DeviceFactsCollector.capabilityReport(context)
        fact("capabilityReport=" + CaptureJson.encodeToString(CapabilityReport.serializer(), report))

        val intent = DeviceFactsCollector.screenCaptureIntent(context)
        val resolved = pm.resolveActivity(intent, 0)
        val component = resolved?.activityInfo?.let { "${it.packageName}/${it.name}" }
        val leanback = pm.hasSystemFeature(PackageManager.FEATURE_LEANBACK)
        fact("api=${Build.VERSION.SDK_INT} release=${Build.VERSION.RELEASE} model=${Build.MODEL} device=${Build.DEVICE} product=${Build.PRODUCT} fingerprint=${Build.FINGERPRINT}")
        fact("FEATURE_LEANBACK=$leanback FEATURE_TELEVISION=${pm.hasSystemFeature("android.hardware.type.television")} touchscreen=${pm.hasSystemFeature(PackageManager.FEATURE_TOUCHSCREEN)}")
        fact("createScreenCaptureIntent.resolves=${resolved != null} component=$component")
        fact("queryIntentActivities=" + pm.queryIntentActivities(intent, 0).joinToString { "${it.activityInfo.packageName}/${it.activityInfo.name}" })
        fact("collector.consentActivityAvailable=${DeviceFactsCollector.consentActivityAvailable(context)}")

        if (resolved == null) {
            fact("RESULT=NO_CONSENT_ACTIVITY -> expecting capture unsupported + companion mode")
            for (k in listOf(FeatureKey.VIDEO_RECORDING, FeatureKey.SCREENSHOT)) {
                fact("feature $k level=${report.level(k)}")
                assertEquals("$k must be unsupported without a consent activity", SupportLevel.UNSUPPORTED, report.level(k))
            }
            return
        }

        // Consent activity exists: do the real thing, like the phone test.
        shell("appops set $pkg PROJECT_MEDIA allow")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) shell("pm grant $pkg android.permission.POST_NOTIFICATIONS")
        CaptureController.clearError()
        fact("feature VIDEO_RECORDING level=${report.level(FeatureKey.VIDEO_RECORDING)} SCREENSHOT level=${report.level(FeatureKey.SCREENSHOT)}")

        val dir = File(context.getExternalFilesDir(null) ?: context.filesDir, "tv-e2e-tmp")
        val config = CaptureSettings(countdownSec = 0, fileNameTemplate = "CaptureOne tv {date} {time}").let { s ->
            s.copy(video = s.video.copy(codec = VideoCodec.H264, resolution = ResolutionSetting.Height(720), fps = 30))
        }.toRecordingConfig(dir.absolutePath)
        val before = CaptureController.lastMedia.value
        val code = consent { activity, rc, data -> CaptureController.startRecording(activity, rc, data, config) }
        fact("consent resultCode=$code (RESULT_OK=${Activity.RESULT_OK})")
        if (code != Activity.RESULT_OK) {
            fact("RESULT=CONSENT_ACTIVITY_PRESENT_BUT_NOT_GRANTED (dialog waiting for a human or refused)")
            return
        }
        try {
            awaitState(RecordingState.RECORDING)
            Thread.sleep(3000)
            CaptureController.stop(context)
            val media = awaitMedia(MediaKind.VIDEO, before)
            fact("RECORDING " + CaptureJson.encodeToString(MediaMetadata.serializer(), media))
            fact("RESULT=RECORDING valid=${media.validation?.valid} issues=${media.validation?.issues} ${media.width}x${media.height} durationMs=${media.durationMs} codec=${media.videoCodec} path=${media.path}")
            assertTrue("validated: ${media.validation?.issues}", media.validation?.valid == true)
        } catch (e: AssertionError) {
            fact("RESULT=RECORDING_FAILED ${e.message}")
            throw e
        }

        val beforeShot = CaptureController.lastMedia.value
        val sc = consent { activity, rc, data ->
            CaptureController.takeScreenshot(activity, rc, data, ImageFormat.PNG, 90, "CaptureOne tv shot {date} {time}")
        }
        if (sc == Activity.RESULT_OK) {
            val shot = awaitMedia(MediaKind.SCREENSHOT, beforeShot)
            fact("SCREENSHOT " + CaptureJson.encodeToString(MediaMetadata.serializer(), shot))
            assertTrue("screenshot validated: ${shot.validation?.issues}", shot.validation?.valid == true)
        } else fact("screenshot consent resultCode=$sc")
    }

    private companion object {
        const val TAG = "CaptureOneTV"
    }
}
