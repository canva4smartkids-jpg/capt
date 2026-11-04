package dev.captureone.android.logic

import dev.captureone.capture.capability.CapabilityClassifier
import dev.captureone.capture.capability.DeviceFacts
import dev.captureone.capture.capability.EncoderFacts
import dev.captureone.capture.capability.FormFactor
import dev.captureone.capture.contract.AudioTrackMode
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.MediaValidation
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
import dev.captureone.capture.settings.CaptureSettings
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class RecordModelsTest {
    private val repo = File(System.getProperty("captureone.repoRoot") ?: "../../..")
    private val presets = EncodingPresets.parse(File(repo, "packages/settings/presets.json").readText())

    private fun report(sdk: Int, hevc: Boolean, mic: Boolean = true) = CapabilityClassifier.report(
        DeviceFacts(
            sdk, "x", "Acme", "Phone", "1.0", FormFactor.PHONE, false, true, mic,
            listOfNotNull(
                EncoderFacts(VideoCodec.H264, "c2.vendor.avc.encoder", true, 4096, 4096, false),
                if (hevc) EncoderFacts(VideoCodec.H265, "c2.vendor.hevc.encoder", true, 4096, 4096, false) else null,
            ),
            Size(1080, 2400), Size(1080, 2400), "t",
        ),
    )

    @Test
    fun optionsFollowTheDevice() {
        val o = RecordOptions.options(report(34, hevc = false), Size(1080, 2400))
        assertEquals(listOf(VideoCodec.H264), o.codecs)
        assertEquals(listOf(24, 30, 60), o.frameRates)
        assertEquals(ResolutionSetting.Source, o.resolutions.first())
        assertEquals(listOf(2160, 1440, 1080, 720, 480), o.resolutions.drop(1).map { (it as ResolutionSetting.Height).height })
        val withHevc = RecordOptions.options(report(34, hevc = true), Size(1280, 720))
        assertEquals(listOf(VideoCodec.H264, VideoCodec.H265), withHevc.codecs)
        assertEquals("only heights below the 720 px source", listOf(480), withHevc.resolutions.drop(1).map { (it as ResolutionSetting.Height).height })
    }

    @Test
    fun resolutionLabelsShowTheRealOutputSize() {
        assertEquals("1080 px tall · 486×1080", RecordOptions.label(ResolutionSetting.Height(1080), Size(1080, 2400)))
        assertEquals("Native · 1080×2400", RecordOptions.label(ResolutionSetting.Source, Size(1080, 2400)))
    }

    @Test
    fun sanitizeDropsUnavailableChoices() {
        val s = CaptureSettings().let { it.copy(video = it.video.copy(codec = VideoCodec.H265), audio = it.audio.copy(system = it.audio.system.copy(enabled = true))) }
        val fixed = RecordOptions.sanitize(s, report(28, hevc = false))
        assertEquals(VideoCodec.H264, fixed.video.codec)
        assertFalse(fixed.audio.system.enabled)
        assertEquals(s, RecordOptions.sanitize(s, report(34, hevc = true)))
    }

    @Test
    fun audioTogglesAreHonest() {
        val old = report(28, hevc = false)
        val sys = RecordOptions.system(old, CaptureSettings())
        assertFalse(sys.available)
        assertTrue(sys.note.contains("Android 10"))
        val noMic = RecordOptions.mic(report(34, hevc = false, mic = false), CaptureSettings(), null)
        assertFalse(noMic.available)
    }

    @Test
    fun estimateUsesPresets() {
        val s = CaptureSettings().let { it.copy(video = it.video.copy(resolution = ResolutionSetting.Height(720), fps = 30, quality = QualityPreset.MEDIUM)) }
        val e = RecordOptions.estimate(s, Size(1280, 720), presets)!!
        assertEquals(Size(1280, 720), e.outputSize)
        assertEquals(presets.videoBitrateKbps(Size(1280, 720), 30, VideoCodec.H264, QualityPreset.MEDIUM), e.videoKbps)
        assertEquals(presets.estimateFileSizeBytes(e.videoKbps.toDouble(), 0.0, 60.0), e.bytesPerMinute)
        val withAudio = s.copy(audio = s.audio.copy(microphone = s.audio.microphone.copy(enabled = true), system = s.audio.system.copy(enabled = true), trackMode = AudioTrackMode.SEPARATE))
        assertEquals(presets.estimateFileSizeBytes(e.videoKbps.toDouble(), 320.0, 60.0), RecordOptions.estimate(withAudio, Size(1280, 720), presets)!!.bytesPerMinute)
        assertNull(RecordOptions.estimate(s, null, presets))
    }

    @Test
    fun permissionCards() {
        val cards = PermissionCards.forRecordScreen(
            listOf(
                PermissionStatus(PermissionKind.MICROPHONE, PermissionState.DENIED, requiresSystemSettings = true),
                PermissionStatus(PermissionKind.SCREEN_CAPTURE, PermissionState.PER_SESSION),
                PermissionStatus(PermissionKind.STORAGE, PermissionState.NOT_APPLICABLE),
                PermissionStatus(PermissionKind.NOTIFICATIONS, PermissionState.PROMPT),
            ),
        )
        assertEquals(listOf(PermissionKind.SCREEN_CAPTURE, PermissionKind.NOTIFICATIONS, PermissionKind.MICROPHONE), cards.map { it.kind })
        assertNull(cards[0].actionLabel)
        assertTrue(cards[0].satisfied)
        assertEquals("Allow", cards[1].actionLabel)
        assertEquals("Open settings", cards[2].actionLabel)
        assertTrue(cards[2].opensSettings)
    }

    @Test
    fun controlsFollowRecordingState() {
        fun st(s: RecordingState, cd: Int? = null) = RecordingStatus("id", s, 65_400.0, cd, 2048, 10, 0)
        assertNull(ControlsModel.from(null))
        assertNull(ControlsModel.from(st(RecordingState.COMPLETED)))
        assertNull(ControlsModel.from(st(RecordingState.IDLE)))
        val rec = ControlsModel.from(st(RecordingState.RECORDING))!!
        assertTrue(rec.canPause && rec.canStop && !rec.canResume && rec.canScreenshot)
        assertEquals("01:05", rec.timer)
        val paused = ControlsModel.from(st(RecordingState.PAUSED))!!
        assertTrue(paused.canResume && paused.canStop && !paused.canPause)
        val cd = ControlsModel.from(st(RecordingState.COUNTDOWN, 3))!!
        assertTrue(cd.canCancel)
        assertEquals(3, cd.countdown)
        assertFalse(ControlsModel.from(st(RecordingState.FINALIZING))!!.canStop)
    }

    @Test
    fun libraryRows() {
        val m = MediaMetadata(
            "1", MediaKind.VIDEO, "content://x", "a.mp4", "My clip", "2026-11-04T10:20:30Z", 3_145_728, 1080, 2400,
            durationMs = 65_000.0, videoCodec = VideoCodec.H264, container = "mp4", validation = MediaValidation(true, "t", "x", emptyList()),
        )
        val row = LibraryRowModel.from(m, published = true)
        assertEquals("01:05 · 1080×2400 · 3.0 MB · H.264 · 2026-11-04 10:20", row.subtitle)
        assertTrue(row.shareable)
        assertNull(row.validationNote)
        assertTrue(LibraryRowModel.from(m, published = false).validationNote!!.startsWith("Kept"))
        assertEquals("2 recordings · 1 screenshots · 1.5 KB used · 1.0 GB free", StorageUsageModel.from(1536, 2, 1, 1L shl 30).text)
    }
}
