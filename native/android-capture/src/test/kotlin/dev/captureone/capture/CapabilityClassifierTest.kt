package dev.captureone.capture

import dev.captureone.capture.capability.CapabilityClassifier
import dev.captureone.capture.capability.DeviceFacts
import dev.captureone.capture.capability.EncoderFacts
import dev.captureone.capture.capability.FormFactor
import dev.captureone.capture.capability.NativeCaptureTier
import dev.captureone.capture.contract.DeviceKind
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.Platform
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.SupportLevel
import dev.captureone.capture.contract.VideoCodec
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CapabilityClassifierTest {
    private val avcHw = EncoderFacts(VideoCodec.H264, "c2.exynos.h264.encoder", true, 4096, 2304, false)
    private val avcSwLegacy = EncoderFacts(VideoCodec.H264, "OMX.google.h264.encoder", null, 2048, 2048, false)
    private val avcVendorLegacy = EncoderFacts(VideoCodec.H264, "OMX.qcom.video.encoder.avc", null, 4096, 2160, false)
    private val hevcHw = EncoderFacts(VideoCodec.H265, "c2.exynos.hevc.encoder", true, 4096, 2304, true)

    private fun facts(
        form: FormFactor = FormFactor.PHONE, sdk: Int = 34, consent: Boolean = true, mic: Boolean = true,
        encoders: List<EncoderFacts> = listOf(avcHw, hevcHw), googleTv: Boolean = false,
    ) = DeviceFacts(sdk, "14", "Acme", "Model", "1.0", form, googleTv, consent, mic, encoders, Size(1080, 2400), Size(1080, 2400), "2026-11-04T00:00:00Z")

    @Test
    fun modernPhoneIsNativeWithPartialSystemAudio() {
        val r = CapabilityClassifier.report(facts())
        assertEquals(Platform.ANDROID, r.platform)
        assertEquals(SupportLevel.NATIVE, r.level(FeatureKey.VIDEO_RECORDING))
        assertEquals(SupportLevel.NATIVE, r.level(FeatureKey.SCREENSHOT))
        assertEquals(SupportLevel.PARTIAL, r.level(FeatureKey.SYSTEM_AUDIO))
        assertEquals(SupportLevel.PARTIAL, r.level(FeatureKey.WINDOW_CAPTURE)) // API 34 single-app capture
        assertEquals(SupportLevel.NATIVE, r.level(FeatureKey.H265))
        assertEquals(SupportLevel.NATIVE, r.level(FeatureKey.HARDWARE_ENCODING))
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.REGION_CAPTURE))
        assertEquals(listOf(24, 30, 60), r.frameRates)
        assertTrue(r.encoders.single { it.codec == VideoCodec.H265 }.verified)
        assertTrue(r.notes.any { it.contains("FLAG_SECURE") })
    }

    @Test
    fun android9HasNoInternalAudioAndInfersHardware() {
        val r = CapabilityClassifier.report(facts(sdk = 28, encoders = listOf(avcVendorLegacy, avcSwLegacy)))
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.SYSTEM_AUDIO))
        assertEquals("Android 10", r.features[FeatureKey.SYSTEM_AUDIO]?.minOsVersion)
        assertEquals(SupportLevel.PARTIAL, r.level(FeatureKey.HARDWARE_ENCODING))
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.WINDOW_CAPTURE))
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.H265))
        assertTrue(r.encoders.first { it.name.startsWith("OMX.qcom") }.hardware)
        assertFalse(r.encoders.first { it.name.startsWith("OMX.google") }.hardware)
        assertTrue(r.notes.any { it.contains("KEY_MAX_FPS_TO_ENCODER") })
    }

    @Test
    fun tvWithoutConsentActivityIsHonest() {
        val f = facts(form = FormFactor.TV, sdk = 31, consent = false, mic = false)
        val r = CapabilityClassifier.report(f)
        assertEquals(Platform.ANDROID_TV, r.platform)
        assertEquals(SupportLevel.EXTERNAL_HARDWARE, r.level(FeatureKey.VIDEO_RECORDING))
        assertEquals(SupportLevel.EXTERNAL_HARDWARE, r.level(FeatureKey.SCREENSHOT))
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.SYSTEM_AUDIO))
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.TV_NATIVE_SCREENSHOT))
        assertEquals(SupportLevel.NATIVE, r.level(FeatureKey.REMOTE_CONTROL))
        assertEquals(SupportLevel.EXTERNAL_HARDWARE, r.level(FeatureKey.HDMI_CAPTURE_DEVICE))
        assertTrue(r.frameRates.isEmpty())
        assertTrue(r.screenshotFormats.isEmpty())
        assertEquals(null, r.maxRecordingResolution)
        val a = CapabilityClassifier.nativeCaptureAssessment(f)
        assertEquals(NativeCaptureTier.NOT_AVAILABLE, a.tier)
        assertTrue(a.reasons.any { it.contains("companion mode") })
    }

    @Test
    fun googleTvWithConsentIsPartialBecauseOfDrm() {
        val f = facts(form = FormFactor.TV, sdk = 33, consent = true, mic = false, googleTv = true)
        val r = CapabilityClassifier.report(f)
        assertEquals(Platform.GOOGLE_TV, r.platform)
        assertEquals(SupportLevel.PARTIAL, r.level(FeatureKey.VIDEO_RECORDING))
        assertEquals(SupportLevel.PARTIAL, r.level(FeatureKey.SYSTEM_AUDIO))
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.MICROPHONE_AUDIO))
        assertEquals(NativeCaptureTier.SUPPORTED, CapabilityClassifier.nativeCaptureAssessment(f).tier)
        assertEquals(DeviceKind.GOOGLE_TV, CapabilityClassifier.deviceKind(f))
    }

    @Test
    fun oldTvWithConsentIsPartiallySupported() {
        val f = facts(form = FormFactor.TV, sdk = 28, consent = true, mic = false)
        assertEquals(NativeCaptureTier.PARTIAL, CapabilityClassifier.nativeCaptureAssessment(f).tier)
    }

    @Test
    fun noEncoderMeansNoRecording() {
        val f = facts(encoders = emptyList())
        val r = CapabilityClassifier.report(f)
        assertEquals(SupportLevel.UNSUPPORTED, r.level(FeatureKey.VIDEO_RECORDING))
        assertEquals(SupportLevel.NATIVE, r.level(FeatureKey.SCREENSHOT))
        assertEquals(NativeCaptureTier.NOT_AVAILABLE, CapabilityClassifier.nativeCaptureAssessment(f).tier)
    }

    @Test
    fun everyNonNativeFeatureHasAReason() {
        for (form in FormFactor.entries) for (sdk in listOf(26, 29, 34)) for (consent in listOf(true, false)) {
            val r = CapabilityClassifier.report(facts(form = form, sdk = sdk, consent = consent))
            for ((k, v) in r.features) if (v.level != SupportLevel.NATIVE) assertFalse("$form/$sdk/$consent $k lacks reason", v.reason.isNullOrBlank())
        }
    }

    @Test
    fun deviceKinds() {
        assertEquals(DeviceKind.ANDROID_PHONE, CapabilityClassifier.deviceKind(facts()))
        assertEquals(DeviceKind.ANDROID_TABLET, CapabilityClassifier.deviceKind(facts(form = FormFactor.TABLET)))
        assertEquals(DeviceKind.ANDROID_TV, CapabilityClassifier.deviceKind(facts(form = FormFactor.TV)))
    }
}
