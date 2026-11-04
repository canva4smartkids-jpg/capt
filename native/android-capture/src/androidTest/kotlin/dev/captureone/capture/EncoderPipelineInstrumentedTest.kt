package dev.captureone.capture

import android.graphics.Color
import android.graphics.Paint
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.HardwareAcceleration
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.engine.AudioEncoder
import dev.captureone.capture.engine.EncoderCatalog
import dev.captureone.capture.engine.MediaValidator
import dev.captureone.capture.engine.MuxerSink
import dev.captureone.capture.engine.VideoEncoder
import dev.captureone.capture.engine.VideoEncoderConfig
import dev.captureone.capture.media.PauseClock
import dev.captureone.capture.media.framesToUs
import dev.captureone.capture.presets.EncodingPresets
import dev.captureone.capture.service.CaptureService
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import kotlin.math.PI
import kotlin.math.sin

/**
 * The closest thing to on-device verification that needs no MediaProjection consent: drives the real
 * engine classes (VideoEncoder with a Surface input, AudioEncoder, MuxerSink, PauseClock,
 * MediaValidator) with synthetic frames drawn on the encoder's input Surface and synthetic PCM, then
 * validates the MP4 with MediaExtractor/MediaMetadataRetriever.
 *
 * Runs only on an emulator/device (CI job "emulator-smoke" in .github/workflows/android.yml).
 */
@RunWith(AndroidJUnit4::class)
class EncoderPipelineInstrumentedTest {
    private val context = InstrumentationRegistry.getInstrumentation().targetContext

    @Test
    fun presetsAssetIsPackaged() {
        val p: EncodingPresets = CaptureService.loadPresets(context)
        assertEquals(listOf(24, 30, 60), p.frameRates)
    }

    @Test
    fun capabilityReportIsHonestAndSerializable() {
        val r: CapabilityReport = DeviceFactsCollector.capabilityReport(context)
        assertTrue("an AVC encoder exists", r.encoders.any { it.codec == VideoCodec.H264 })
        assertTrue(r.isUsable(FeatureKey.H264))
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) assertTrue(!r.isUsable(FeatureKey.SYSTEM_AUDIO))
        val json = CaptureJson.encodeToString(CapabilityReport.serializer(), r)
        assertEquals(r, CaptureJson.decodeFromString(CapabilityReport.serializer(), json))
    }

    @Test
    fun surfaceEncoderWithPauseAndAacProducesValidMp4() {
        val candidate = EncoderCatalog.candidates(VideoCodec.H264, HardwareAcceleration.AUTO).first()
        val size = candidate.fit(Size(640, 360))
        assertNotNull(size)
        val file = File(context.cacheDir, "pipeline-test.mp4").apply { delete() }
        val muxer = MuxerSink(file, expectedTracks = 2)
        val origin = System.nanoTime()
        val clock = PauseClock(origin)
        val thread = HandlerThread("test-codec").apply { start() }
        var fatal: Throwable? = null
        val video = VideoEncoder(
            candidate, VideoEncoderConfig(VideoCodec.H264, size!!, 30, 2_000_000, 1.0),
            muxer, clock, Handler(thread.looper), onFatal = { fatal = it },
        )
        video.prepare()
        video.start()
        val audio = AudioEncoder("mixed", 48_000, 2, 128, muxer)

        val paint = Paint().apply { color = Color.WHITE; textSize = 48f }
        var audioFrames = 0L
        val frames = 90
        for (i in 0 until frames) {
            if (i == 30) clock.pause(System.nanoTime())
            if (i == 45) {
                clock.resume(System.nanoTime())
                video.requestSyncFrame()
            }
            val canvas = video.inputSurface.lockHardwareCanvas()
            canvas.drawColor(Color.rgb((i * 7) % 255, 40, 160))
            canvas.drawText("frame $i", 40f, 120f, paint)
            video.inputSurface.unlockCanvasAndPost(canvas)
            // ~33 ms of 440 Hz stereo tone per frame, stamped on the same clock.
            val n = 1600
            val pcm = ShortArray(n * 2) { s -> (sin(2 * PI * 440 * (audioFrames + s / 2) / 48_000) * 8000).toInt().toShort() }
            val pts = clock.map(origin + framesToUs(audioFrames, 48_000) * 1000)
            if (pts != null) audio.encode(pcm, pts)
            audioFrames += n
            Thread.sleep(33)
        }
        audio.finish(clock.map(System.nanoTime()) ?: 0)
        assertTrue("video drained", video.finish(5000))
        val finalized = muxer.finish()
        video.release()
        audio.release()
        thread.quitSafely()
        assertEquals(null, fatal)
        assertTrue("muxer finalized", finalized)

        val inspected = MediaValidator.inspectVideo(file, expectAudioTracks = 1, audioLabels = listOf("mixed"))
        assertTrue("validation: ${inspected.validation.issues}", inspected.validation.valid)
        assertEquals(size, inspected.size)
        assertEquals(VideoCodec.H264, inspected.videoCodec)
        assertEquals(1, inspected.audioTracks.size)
        assertEquals(48_000, inspected.audioTracks[0].sampleRate)
        assertEquals(2, inspected.audioTracks[0].channels)
        // 90 frames at ~33 ms with 15 frames (~0.5 s) paused: about 2.5 s of media (generous bounds for slow emulators).
        val d = inspected.durationMs ?: 0.0
        assertTrue("duration $d ms", d in 1500.0..9000.0)
        assertTrue("paused frames were dropped", video.framesDropped.get() > 0)
        file.delete()
    }
}
