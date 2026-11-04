package dev.captureone.tv.logic

import dev.captureone.capture.capability.NativeCaptureAssessment
import dev.captureone.capture.capability.NativeCaptureTier
import dev.captureone.capture.companion.HubAddress
import dev.captureone.capture.contract.CaptureError
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class TvModelsTest {
    private fun type(s: PadState, keys: String): PadState = keys.fold(s) { acc, c ->
        NumericPad.reduce(acc, when (c) {
            '.' -> PadKey.Dot
            ':' -> PadKey.Colon
            '<' -> PadKey.Backspace
            '>' -> PadKey.NextField
            else -> PadKey.Digit(c)
        })
    }

    @Test
    fun typesAddressAndCode() {
        val s = type(PadState(), "192.168.1.20>123456")
        assertEquals("192.168.1.20", s.address)
        assertEquals("123456", s.code)
        assertEquals(HubAddress("192.168.1.20", 47800), s.parsedAddress)
        assertTrue(s.canConnect)
    }

    @Test
    fun rejectsKeysThatCannotLeadToAValidAddress() {
        assertEquals("192.168.1.2", type(PadState(), "192..168.1.2").address)
        assertEquals("25", type(PadState(), "256").address)
        assertEquals("a fifth group is refused; the digit extends the fourth", "1.2.3.45", type(PadState(), "1.2.3.4.5").address)
        assertEquals("port separator only after four groups", "12", type(PadState(), "1:2").address)
        assertEquals("10.0.0.1:47800", type(PadState(), "10.0.0.1:47800000").address)
        assertEquals("10.0.0.1:6553", type(PadState(), "10.0.0.1:65536").address)
        assertEquals("10.0.0.1:47800", type(PadState(), "10.0.0.1:47800").address)
        assertEquals("0", type(PadState(), "01").address)
    }

    @Test
    fun portIsParsed() {
        val s = type(PadState(), "10.0.0.1:8080")
        assertEquals(8080, s.parsedAddress?.port)
    }

    @Test
    fun codeIsDigitsOnlyAndMaxSix() {
        val s = type(PadState(field = PadField.CODE), "12.3:4567890")
        assertEquals("123456", s.code)
        assertTrue(s.codeComplete)
        assertFalse(s.canConnect)
    }

    @Test
    fun backspaceAndClearActOnTheCurrentField() {
        var s = type(PadState(), "10.0.0.12<")
        assertEquals("10.0.0.1", s.address)
        s = type(s, ">12")
        s = NumericPad.reduce(s, PadKey.Clear)
        assertEquals("", s.code)
        assertEquals("10.0.0.1", s.address)
    }

    @Test
    fun addressErrorOnlyForImpossibleInput() {
        assertNull(PadState(address = "192.168").addressError)
        assertNull(PadState(address = "192.168.1.1").addressError)
    }

    @Test
    fun padLayoutIsAFiveByThreeGrid() {
        assertEquals(5, NumericPad.LAYOUT.size)
        assertTrue(NumericPad.LAYOUT.all { it.size == 3 })
        assertEquals(10, NumericPad.LAYOUT.flatten().count { it is PadKey.Digit })
    }

    @Test
    fun homeModelTiers() {
        val na = TvHomeModel.from(NativeCaptureAssessment(NativeCaptureTier.NOT_AVAILABLE, listOf("x")))
        assertEquals("Native capture not available on this device", na.headline)
        assertFalse(na.canRecord)
        assertTrue(TvHomeModel.from(NativeCaptureAssessment(NativeCaptureTier.PARTIAL, emptyList())).canRecord)
        assertEquals("Native capture supported", TvHomeModel.from(NativeCaptureAssessment(NativeCaptureTier.SUPPORTED, emptyList())).headline)
    }

    @Test
    fun hubStatusTexts() {
        assertEquals("Hub is idle", TvText.hubStatus(null, connected = true))
        assertEquals("Not connected to the hub's event stream", TvText.hubStatus(null, connected = false))
        assertEquals("Recording · 01:05 · 1.0 KB", TvText.hubStatus(RecordingStatus("s", RecordingState.RECORDING, 65_000.0, null, 1024, 1, 0), true))
        assertTrue(TvText.hubStatus(RecordingStatus("s", RecordingState.FAILED, 0.0, null, 0, 0, 0, error = CaptureError(CaptureErrorCode.NETWORK, "boom", false)), true).endsWith("boom"))
    }
}
