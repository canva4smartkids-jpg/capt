package dev.captureone.capture

import android.app.Activity
import android.content.Intent
import android.os.Build
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.ResolutionSetting
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.service.CaptureController
import dev.captureone.capture.settings.CaptureSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.concurrent.CompletableFuture
import java.util.concurrent.TimeUnit

/**
 * Full engine path on an emulator/device: MediaProjection consent → CaptureService (FGS type
 * mediaProjection) → VirtualDisplay → MediaCodec → MediaMuxer → pause/resume → finalize →
 * MediaExtractor validation → MediaStore publish, plus a standalone ImageReader screenshot.
 *
 * Consent is pre-authorised with `appops set <pkg> PROJECT_MEDIA allow` (the SystemUI consent activity
 * then returns RESULT_OK without UI). If a device still shows the dialog the tests are skipped, not
 * faked. Only runnable in CI (emulator job) or on a device.
 */
@RunWith(AndroidJUnit4::class)
class MediaProjectionEndToEndTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context = instrumentation.targetContext

    private fun shell(cmd: String): String =
        instrumentation.uiAutomation.executeShellCommand(cmd).use { pfd ->
            android.os.ParcelFileDescriptor.AutoCloseInputStream(pfd).bufferedReader().readText()
        }

    @Before
    fun grant() {
        val pkg = context.packageName
        shell("appops set $pkg PROJECT_MEDIA allow")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) shell("pm grant $pkg android.permission.POST_NOTIFICATIONS")
        CaptureController.clearError()
    }

    /** Launches the consent activity; [then] runs in its onActivityResult with a fresh token. */
    private fun consent(then: (Activity, Int, Intent) -> Unit): Int {
        val f = CompletableFuture<Int>()
        ConsentProxyActivity.pending = f
        ConsentProxyActivity.onResult = then
        context.startActivity(Intent(context, ConsentProxyActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        return try {
            f.get(20, TimeUnit.SECONDS)
        } catch (e: java.util.concurrent.TimeoutException) {
            Activity.RESULT_CANCELED // the consent dialog is waiting for a human
        } finally {
            ConsentProxyActivity.onResult = null
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

    private fun log(tag: String, m: MediaMetadata) {
        Log.i(TAG, "$tag ${CaptureJson.encodeToString(MediaMetadata.serializer(), m)}")
    }

    @Test
    fun recordsTheScreenWithPauseResumeAndPublishesAValidatedMp4() {
        val report = DeviceFactsCollector.capabilityReport(context)
        assumeTrue("MediaProjection consent activity present", report.isUsable(FeatureKey.VIDEO_RECORDING))
        val dir = File(context.getExternalFilesDir(null) ?: context.filesDir, "e2e-tmp")
        val config = CaptureSettings(countdownSec = 0, fileNameTemplate = "CaptureOne e2e {date} {time}").let { s ->
            s.copy(video = s.video.copy(codec = VideoCodec.H264, resolution = ResolutionSetting.Height(720), fps = 30))
        }.toRecordingConfig(dir.absolutePath)
        val before = CaptureController.lastMedia.value

        val code = consent { activity, resultCode, data -> CaptureController.startRecording(activity, resultCode, data, config) }
        assumeTrue("consent was granted without UI (appops PROJECT_MEDIA)", code == Activity.RESULT_OK)

        awaitState(RecordingState.RECORDING)
        Thread.sleep(1500)
        CaptureController.pause(context)
        awaitState(RecordingState.PAUSED)
        Thread.sleep(1500)
        CaptureController.resume(context)
        awaitState(RecordingState.RECORDING)
        Thread.sleep(1500)
        val status = CaptureController.status.value
        Log.i(TAG, "status before stop: $status")
        CaptureController.stop(context)

        val media = awaitMedia(MediaKind.VIDEO, before)
        log("RECORDING", media)
        val v = media.validation
        assertTrue("validated: ${v?.issues}", v?.valid == true)
        assertEquals(VideoCodec.H264, media.videoCodec)
        assertEquals("mp4", media.container)
        assertTrue("published to MediaStore: ${media.path}", media.path.startsWith("content://"))
        assertTrue("height ${media.height} <= 720 (or native when smaller)", media.height <= 720 || media.height <= report.maxRecordingResolution!!.height)
        // ~3 s recorded + 1.5 s paused: the paused time must not be in the file.
        val d = media.durationMs ?: 0.0
        assertTrue("duration $d ms excludes the 1.5 s pause", d in 2000.0..4300.0)
        assertTrue("frames encoded", (status?.framesEncoded ?: 0) > 0)
    }

    @Test
    fun takesAValidatedScreenshot() {
        val report = DeviceFactsCollector.capabilityReport(context)
        assumeTrue(report.isUsable(FeatureKey.SCREENSHOT))
        val before = CaptureController.lastMedia.value
        val code = consent { activity, resultCode, data ->
            CaptureController.takeScreenshot(activity, resultCode, data, ImageFormat.PNG, 90, "CaptureOne e2e shot {date} {time}")
        }
        assumeTrue("consent was granted without UI (appops PROJECT_MEDIA)", code == Activity.RESULT_OK)
        val media = awaitMedia(MediaKind.SCREENSHOT, before)
        log("SCREENSHOT", media)
        assertTrue("validated: ${media.validation?.issues}", media.validation?.valid == true)
        assertEquals(ImageFormat.PNG, media.imageFormat)
        val display = report.maxRecordingResolution
        assertTrue("non-trivial size ${media.width}x${media.height}", media.width >= 320 && media.height >= 320)
        if (display != null) assertTrue(media.width * media.height >= display.width * display.height / 4)
    }

    private companion object {
        const val TAG = "CaptureOneTest"
    }
}
