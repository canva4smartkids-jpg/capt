package dev.captureone.capture

import dev.captureone.capture.contract.QualityPreset
import dev.captureone.capture.contract.ResolutionSetting
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.presets.EncodingPresets
import dev.captureone.capture.presets.resolveOutputSize
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.long
import kotlinx.serialization.json.double
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** Kotlin presets logic vs. tests/contract/fixtures/presets-cases.json (shared with TS and Rust). */
class PresetsContractTest {
    private val presets = EncodingPresets.parse(RepoFiles.presetsJson)
    private val fixtures = Json.parseToJsonElement(RepoFiles.text("tests/contract/fixtures/presets-cases.json")).jsonObject

    private fun size(o: JsonObject) = Size(o.getValue("width").jsonPrimitive.int, o.getValue("height").jsonPrimitive.int)

    private fun setting(o: JsonObject): ResolutionSetting = when (o.getValue("mode").jsonPrimitive.content) {
        "source" -> ResolutionSetting.Source
        "height" -> ResolutionSetting.Height(o.getValue("height").jsonPrimitive.int)
        "custom" -> ResolutionSetting.Custom(o.getValue("width").jsonPrimitive.int, o.getValue("height").jsonPrimitive.int)
        else -> error("unknown mode")
    }

    @Test
    fun resolutionCasesMatchFixtures() {
        val cases = fixtures.getValue("resolution").jsonArray
        assertTrue(cases.size >= 50)
        for (c in cases) {
            val o = c.jsonObject
            val got = resolveOutputSize(size(o.getValue("source").jsonObject), setting(o.getValue("setting").jsonObject))
            assertEquals("case $o", size(o.getValue("expected").jsonObject), got)
            assertEquals(0, got.width % 2)
            assertEquals(0, got.height % 2)
        }
    }

    @Test
    fun bitrateCasesMatchFixtures() {
        val cases = fixtures.getValue("bitrate").jsonArray
        assertTrue(cases.size >= 90)
        for (c in cases) {
            val o = c.jsonObject
            val codec = if (o.getValue("codec").jsonPrimitive.content == "h264") VideoCodec.H264 else VideoCodec.H265
            val quality = QualityPreset.valueOf(o.getValue("quality").jsonPrimitive.content.uppercase())
            val got = presets.videoBitrateKbps(size(o.getValue("size").jsonObject), o.getValue("fps").jsonPrimitive.int, codec, quality)
            assertEquals("case $o", o.getValue("expectedKbps").jsonPrimitive.int, got)
        }
    }

    @Test
    fun fileSizeCasesMatchFixtures() {
        for (c in fixtures.getValue("fileSize").jsonArray) {
            val o = c.jsonObject
            val got = presets.estimateFileSizeBytes(
                o.getValue("videoKbps").jsonPrimitive.double,
                o.getValue("audioKbps").jsonPrimitive.double,
                o.getValue("durationSec").jsonPrimitive.double,
            )
            assertEquals("case $o", o.getValue("expectedBytes").jsonPrimitive.long, got)
        }
    }

    @Test
    fun customQualityUsesCustomBitrate() {
        assertEquals(7777, presets.videoBitrateKbps(Size(1920, 1080), 30, VideoCodec.H264, QualityPreset.CUSTOM, 7777))
    }

    @Test
    fun clampsToPresetBounds() {
        assertEquals(presets.minBitrateKbps.toInt(), presets.videoBitrateKbps(Size(16, 16), 24, VideoCodec.H265, QualityPreset.LOW))
        assertEquals(presets.maxBitrateKbps.toInt(), presets.videoBitrateKbps(Size(7680, 4320), 60, VideoCodec.H264, QualityPreset.ULTRA))
    }

    @Test
    fun presetsFileShapeIsWhatTheEngineExpects() {
        assertEquals(listOf(2160, 1440, 1080, 720, 480), presets.resolutionHeights)
        assertEquals(listOf(24, 30, 60), presets.frameRates)
        assertEquals(ResolutionSetting.Height.HEIGHTS, presets.resolutionHeights)
    }
}
