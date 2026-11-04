package dev.captureone.capture

import dev.captureone.capture.contract.AudioTrackMode
import dev.captureone.capture.contract.HardwareAcceleration
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.QualityPreset
import dev.captureone.capture.contract.ResolutionSetting
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.settings.CaptureSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Defaults mirror `AppSettings` in packages/settings/src/app-settings.ts. */
class CaptureSettingsTest {
    @Test
    fun defaultsMatchAppSettings() {
        val s = CaptureSettings()
        assertEquals("CaptureOne {date} {time}", s.fileNameTemplate)
        assertEquals(VideoCodec.H264, s.video.codec)
        assertEquals(ResolutionSetting.Source, s.video.resolution)
        assertEquals(30, s.video.fps)
        assertEquals(QualityPreset.MEDIUM, s.video.quality)
        assertEquals(2.0, s.video.keyframeIntervalSec, 0.0)
        assertEquals(HardwareAcceleration.AUTO, s.video.hardwareAcceleration)
        assertEquals(false, s.audio.microphone.enabled)
        assertEquals(false, s.audio.system.enabled)
        assertEquals(AudioTrackMode.MIXED, s.audio.trackMode)
        assertEquals(48000, s.audio.sampleRate)
        assertEquals(160, s.audio.bitrateKbps)
        assertEquals(ImageFormat.PNG, s.screenshotFormat)
        assertEquals(90, s.jpegQuality)
        assertEquals(3, s.countdownSec)
        assertEquals(500, s.minFreeDiskMb)
        assertNull(s.maxDurationSec)
    }

    @Test
    fun roundTripAndFallback() {
        val s = CaptureSettings(countdownSec = 10, maxDurationSec = 120, video = CaptureSettings().video.copy(codec = VideoCodec.H265, resolution = ResolutionSetting.Height(720)))
        assertEquals(s, CaptureSettings.fromJson(s.toJson()))
        assertEquals(CaptureSettings(), CaptureSettings.fromJson("{not json"))
        assertEquals(CaptureSettings(), CaptureSettings.fromJson(null))
        assertEquals(CaptureSettings(), CaptureSettings.fromJson("""{"countdownSec":7}"""))
    }

    @Test
    fun toRecordingConfigCarriesLimits() {
        val c = CaptureSettings(maxDurationSec = 60, maxFileSizeMb = 100, minFreeDiskMb = 200).toRecordingConfig("/tmp/x")
        assertEquals(60, c.limits.maxDurationSec)
        assertEquals(100, c.limits.maxFileSizeMb)
        assertEquals(200, c.limits.minFreeDiskMb)
        assertEquals("/tmp/x", c.output.directory)
        assertNull(c.validate())
    }
}
