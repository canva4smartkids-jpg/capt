package dev.captureone.capture

import com.fasterxml.jackson.databind.ObjectMapper
import com.networknt.schema.JsonSchema
import com.networknt.schema.JsonSchemaFactory
import com.networknt.schema.SpecVersion
import dev.captureone.capture.capability.CapabilityClassifier
import dev.captureone.capture.capability.DeviceFacts
import dev.captureone.capture.capability.EncoderFacts
import dev.captureone.capture.capability.FormFactor
import dev.captureone.capture.contract.AudioSettings
import dev.captureone.capture.contract.AudioSourceSettings
import dev.captureone.capture.contract.AudioTrackInfo
import dev.captureone.capture.contract.AudioTrackMode
import dev.captureone.capture.contract.CaptureError
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureEvent
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.CaptureSource
import dev.captureone.capture.contract.CaptureTarget
import dev.captureone.capture.contract.ControlProtocol
import dev.captureone.capture.contract.DeviceKind
import dev.captureone.capture.contract.DiscoveredDevice
import dev.captureone.capture.contract.DiscoveryMethod
import dev.captureone.capture.contract.HardwareAcceleration
import dev.captureone.capture.contract.HubEvent
import dev.captureone.capture.contract.HubInfo
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.IssueSeverity
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.MediaValidation
import dev.captureone.capture.contract.OutputSettings
import dev.captureone.capture.contract.PairRequest
import dev.captureone.capture.contract.PairResponse
import dev.captureone.capture.contract.PermissionKind
import dev.captureone.capture.contract.PermissionState
import dev.captureone.capture.contract.PermissionStatus
import dev.captureone.capture.contract.Platform
import dev.captureone.capture.contract.QualityPreset
import dev.captureone.capture.contract.RecordingConfig
import dev.captureone.capture.contract.RecordingLimits
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.Rect
import dev.captureone.capture.contract.RemoteCommand
import dev.captureone.capture.contract.ResolutionSetting
import dev.captureone.capture.contract.ScreenshotRequest
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.SourceKind
import dev.captureone.capture.contract.StartPreset
import dev.captureone.capture.contract.StopReason
import dev.captureone.capture.contract.SupportLevel
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.ValidationIssue
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.contract.VideoSettings
import dev.captureone.capture.settings.CaptureSettings
import kotlinx.serialization.KSerializer
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * JSON produced by the Kotlin contract types must validate against the shared JSON Schemas in
 * packages/capture-contracts/schema, and decode back to equal objects.
 */
class SchemaContractTest {
    private val factory = JsonSchemaFactory.getInstance(SpecVersion.VersionFlag.V202012)
    private val mapper = ObjectMapper()
    private val schemas = HashMap<String, JsonSchema>()

    private fun schema(name: String): JsonSchema = schemas.getOrPut(name) {
        factory.getSchema(RepoFiles.text("packages/capture-contracts/schema/$name.schema.json"))
    }

    private fun <T> assertValid(name: String, serializer: KSerializer<T>, value: T) {
        val json = CaptureJson.encodeToString(serializer, value)
        val errors = schema(name).validate(mapper.readTree(json))
        assertTrue("$name invalid: $errors\n$json", errors.isEmpty())
        assertEquals("$name round trip", value, CaptureJson.decodeFromString(serializer, json))
    }

    private fun assertSchemaRejects(name: String, json: String) {
        assertTrue("$name should reject $json", schema(name).validate(mapper.readTree(json)).isNotEmpty())
    }

    private val config = RecordingConfig(
        target = CaptureTarget.Display("0"),
        video = VideoSettings(VideoCodec.H265, ResolutionSetting.Height(1080), 60, QualityPreset.HIGH, keyframeIntervalSec = 1.5, hardwareAcceleration = HardwareAcceleration.PREFER),
        audio = AudioSettings(AudioSourceSettings(true, "12", 1.5), AudioSourceSettings(true), AudioTrackMode.SEPARATE, 44100, 192),
        output = OutputSettings("/data/user/0/dev.captureone.android/files/tmp"),
        limits = RecordingLimits(maxDurationSec = 600, maxFileSizeMb = 2048, minFreeDiskMb = 300),
        countdownSec = 5,
    )

    private val validation = MediaValidation(true, "2026-11-04T12:00:00Z", "android.media.MediaExtractor+MediaMetadataRetriever", listOf(ValidationIssue(IssueSeverity.WARNING, "short", "x")))

    private val video = MediaMetadata(
        id = "8a0f", kind = MediaKind.VIDEO, path = "content://media/external/video/media/42", fileName = "CaptureOne 2026-11-04 12-00-00.mp4",
        title = "CaptureOne 2026-11-04 12-00-00", createdAt = "2026-11-04T12:00:00Z", sizeBytes = 1_234_567, width = 1080, height = 2400,
        durationMs = 12_345.6, fps = 29.97, videoCodec = VideoCodec.H264, videoBitrateKbps = 4200.5,
        audioTracks = listOf(AudioTrackInfo(0, "aac", 2, 48000, "mixed", 12_300.0)), container = "mp4", sourceName = "Screen",
        platform = "android", validation = validation, thumbnailPath = "/data/thumb.jpg",
    )

    private val shot = MediaMetadata(
        id = "s1", kind = MediaKind.SCREENSHOT, path = "/sdcard/Pictures/CaptureOne/a.png", fileName = "a.png", title = "a",
        createdAt = "2026-11-04T12:00:00Z", sizeBytes = 10, width = 1080, height = 2400, imageFormat = ImageFormat.PNG, container = "png",
    )

    private fun facts(form: FormFactor, sdk: Int, consent: Boolean) = DeviceFacts(
        sdkInt = sdk, osRelease = "14", manufacturer = "Google", model = "Pixel", appVersion = "0.1.0", formFactor = form,
        isGoogleTv = false, consentActivityAvailable = consent, hasMicrophone = form != FormFactor.TV,
        encoders = listOf(
            EncoderFacts(VideoCodec.H264, "c2.qti.avc.encoder", true, 4096, 2176, true),
            EncoderFacts(VideoCodec.H265, "c2.android.hevc.encoder", false, 1920, 1088, false),
        ),
        displaySize = Size(1080, 2400), maxRecordingSize = Size(1080, 2400), generatedAt = "2026-11-04T12:00:00Z",
    )

    @Test fun recordingConfig() {
        assertValid("RecordingConfig", RecordingConfig.serializer(), config)
        assertValid("RecordingConfig", RecordingConfig.serializer(), config.copy(target = CaptureTarget.Region("0", Rect(0, 0, 640, 480)), video = config.video.copy(resolution = ResolutionSetting.Custom(1280, 720), quality = QualityPreset.CUSTOM, bitrateKbps = 9000)))
        assertValid("RecordingConfig", RecordingConfig.serializer(), CaptureSettings().toRecordingConfig("/tmp/out"))
        assertEquals(null, config.validate())
    }

    @Test fun recordingConfigDefaultsMatchZodDefaults() {
        // Minimal input as the TS side would send it; Kotlin defaults must equal zod defaults.
        val minimal = """{"target":{"kind":"display","displayId":"0"},"video":{"codec":"h264","resolution":{"mode":"source"},"fps":30,"quality":"medium"},
            "audio":{"microphone":{"enabled":false},"system":{"enabled":false}},"output":{"directory":"/x"}}"""
        val c = CaptureJson.decodeFromString(RecordingConfig.serializer(), minimal)
        assertEquals(2.0, c.video.keyframeIntervalSec, 0.0)
        assertEquals(HardwareAcceleration.AUTO, c.video.hardwareAcceleration)
        assertEquals(AudioTrackMode.MIXED, c.audio.trackMode)
        assertEquals(48000, c.audio.sampleRate)
        assertEquals(160, c.audio.bitrateKbps)
        assertEquals(1.0, c.audio.microphone.gain, 0.0)
        assertEquals("CaptureOne {date} {time}", c.output.fileNameTemplate)
        assertEquals(500, c.limits.minFreeDiskMb)
        assertEquals(3, c.countdownSec)
    }

    @Test fun recordingConfigValidationMirrorsZodRefinement() {
        val bad = config.copy(video = config.video.copy(quality = QualityPreset.CUSTOM, bitrateKbps = null))
        assertEquals(CaptureErrorCode.INVALID_CONFIG, bad.validate()?.code)
        assertEquals(CaptureErrorCode.INVALID_CONFIG, config.copy(countdownSec = 4).validate()?.code)
        assertEquals(CaptureErrorCode.INVALID_CONFIG, config.copy(video = config.video.copy(fps = 25)).validate()?.code)
    }

    @Test fun recordingStatusAndErrors() {
        val err = CaptureError(CaptureErrorCode.STORAGE_FULL, "low", true, JsonObject(mapOf("freeMb" to JsonPrimitive(12))))
        assertValid("CaptureError", CaptureError.serializer(), err)
        val st = RecordingStatus("s", RecordingState.PAUSED, 1234.5, null, 10, 20, 1, 3.5e7, 9_000_000_000, "/tmp/a.mp4", "c2.qti.avc.encoder", listOf("w"), err)
        assertValid("RecordingStatus", RecordingStatus.serializer(), st)
        assertValid("RecordingStatus", RecordingStatus.serializer(), RecordingStatus("s", RecordingState.COUNTDOWN, 0.0, 3, 0, 0, 0))
    }

    @Test fun mediaMetadata() {
        assertValid("MediaMetadata", MediaMetadata.serializer(), video)
        assertValid("MediaMetadata", MediaMetadata.serializer(), shot)
    }

    @Test fun captureEvents() {
        val events = listOf(
            CaptureEvent.StateChanged("s", RecordingState.IDLE, RecordingState.COUNTDOWN, "2026-11-04T12:00:00Z"),
            CaptureEvent.Status(RecordingStatus("s", RecordingState.RECORDING, 1.0, null, 1, 1, 0)),
            CaptureEvent.Countdown("s", 2),
            CaptureEvent.Warning(null, "w"),
            CaptureEvent.Error("s", CaptureError(CaptureErrorCode.INTERRUPTED, "x", false)),
            CaptureEvent.Completed("s", StopReason.MAX_FILE_SIZE, video),
            CaptureEvent.Screenshot(shot),
        )
        for (e in events) assertValid("CaptureEvent", CaptureEvent.serializer(), e)
    }

    @Test fun capabilityReportsFromClassifier() {
        for (form in FormFactor.entries) for (sdk in listOf(26, 28, 29, 34, 35)) for (consent in listOf(true, false)) {
            assertValid("CapabilityReport", dev.captureone.capture.contract.CapabilityReport.serializer(), CapabilityClassifier.report(facts(form, sdk, consent)))
        }
    }

    @Test fun permissionsSourcesTargetsScreenshots() {
        for (k in PermissionKind.entries) for (s in PermissionState.entries) {
            assertValid("PermissionStatus", PermissionStatus.serializer(), PermissionStatus(k, s, s == PermissionState.DENIED, "m"))
        }
        assertValid("CaptureSource", CaptureSource.serializer(), CaptureSource("0", SourceKind.DISPLAY, "Built-in screen", 1080, 2400, 0, 0, 2.75, true))
        for (t in listOf(CaptureTarget.Display("0"), CaptureTarget.Window("w"), CaptureTarget.Region("0", Rect(-5, 3, 10, 10)), CaptureTarget.Device("hdmi"))) {
            assertValid("CaptureTarget", CaptureTarget.serializer(), t)
        }
        assertValid("ScreenshotRequest", ScreenshotRequest.serializer(), ScreenshotRequest(CaptureTarget.Display("0"), ImageFormat.JPEG, 80, delaySec = 3))
    }

    @Test fun companionTypes() {
        val report = CapabilityClassifier.report(facts(FormFactor.TV, 30, false))
        val hub = HubInfo(1, "hub-1", "Desk", Platform.LINUX, "0.1.0", report)
        assertValid("HubInfo", HubInfo.serializer(), hub)
        assertValid("PairRequest", PairRequest.serializer(), PairRequest("012345", "Living room TV", DeviceKind.ANDROID_TV, report))
        assertValid("PairResponse", PairResponse.serializer(), PairResponse("t".repeat(43), hub))
        for (c in listOf(RemoteCommand.StartRecording(), RemoteCommand.StartRecording(StartPreset.CAPTURE_DEVICE), RemoteCommand.PauseRecording, RemoteCommand.ResumeRecording, RemoteCommand.StopRecording, RemoteCommand.Screenshot)) {
            assertValid("RemoteCommand", RemoteCommand.serializer(), c)
        }
        assertValid("HubEvent", HubEvent.serializer(), HubEvent.Hello(hub))
        assertValid("HubEvent", HubEvent.serializer(), HubEvent.RecordingStatusChanged(RecordingStatus("s", RecordingState.RECORDING, 5.0, null, 1, 2, 0)))
        assertValid("HubEvent", HubEvent.serializer(), HubEvent.LibraryChanged)
        assertValid(
            "DiscoveredDevice", DiscoveredDevice.serializer(),
            DiscoveredDevice("d", "TV", DeviceKind.GOOGLE_TV, "Google", "Chromecast", "192.168.1.5", 8009, listOf(DiscoveryMethod.MANUAL), listOf(ControlProtocol.CAPTUREONE_COMPANION), emptyList(), "2026-11-04T12:00:00Z"),
        )
    }

    @Test fun nullRecordingStatusInHubEventDecodes() {
        val e = CaptureJson.decodeFromString(HubEvent.serializer(), """{"type":"recording-status","status":null}""")
        assertEquals(HubEvent.RecordingStatusChanged(null), e)
        assertTrue(schema("HubEvent").validate(mapper.readTree("""{"type":"recording-status","status":null}""")).isEmpty())
    }

    @Test fun schemasRejectWhatKotlinMustNeverProduce() {
        // Sanity check that validation is effective (null optional / bad enum / missing required).
        assertSchemaRejects("MediaMetadata", CaptureJson.encodeToString(MediaMetadata.serializer(), video).replace("\"fps\":29.97", "\"fps\":null"))
        assertSchemaRejects("CaptureError", """{"code":"NOPE","message":"x","recoverable":false}""")
        assertSchemaRejects("RecordingStatus", """{"sessionId":"s","state":"recording"}""")
    }

    @Test fun kotlinEnumsMatchEnumsJson() {
        val enums = kotlinx.serialization.json.Json.parseToJsonElement(RepoFiles.text("packages/capture-contracts/schema/enums.json")).jsonObject
        fun names(key: String) = enums.getValue(key).jsonArray.map { it.jsonPrimitive.content }
        fun <E : Enum<E>> wire(values: Array<E>, s: KSerializer<E>) = values.map { CaptureJson.encodeToString(s, it).trim('"') }
        assertEquals(names("Platform"), wire(Platform.entries.toTypedArray(), Platform.serializer()))
        assertEquals(names("SupportLevel"), wire(SupportLevel.entries.toTypedArray(), SupportLevel.serializer()))
        assertEquals(names("FeatureKey"), wire(FeatureKey.entries.toTypedArray(), FeatureKey.serializer()))
        assertEquals(names("PermissionKind"), wire(PermissionKind.entries.toTypedArray(), PermissionKind.serializer()))
        assertEquals(names("PermissionState"), wire(PermissionState.entries.toTypedArray(), PermissionState.serializer()))
        assertEquals(names("CaptureErrorCode"), wire(CaptureErrorCode.entries.toTypedArray(), CaptureErrorCode.serializer()))
        assertEquals(names("RecordingState"), wire(RecordingState.entries.toTypedArray(), RecordingState.serializer()))
        assertEquals(names("DeviceKind"), wire(DeviceKind.entries.toTypedArray(), DeviceKind.serializer()))
        if (enums.containsKey("ControlProtocol")) assertEquals(names("ControlProtocol"), wire(ControlProtocol.entries.toTypedArray(), ControlProtocol.serializer()))
        if (enums.containsKey("StopReason")) assertEquals(names("StopReason"), wire(StopReason.entries.toTypedArray(), StopReason.serializer()))
        if (enums.containsKey("VideoCodec")) assertEquals(names("VideoCodec"), wire(VideoCodec.entries.toTypedArray(), VideoCodec.serializer()))
        if (enums.containsKey("QualityPreset")) assertEquals(names("QualityPreset"), wire(QualityPreset.entries.toTypedArray(), QualityPreset.serializer()))
    }
}
