package dev.captureone.capture

import dev.captureone.capture.contract.Size
import dev.captureone.capture.presets.EncoderSizeLimits
import dev.captureone.capture.presets.fitToEncoder
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class EncoderSizeFitTest {
    private fun limits(maxW: Int, maxH: Int, align: Int = 2, maxBlocks: Long = Long.MAX_VALUE) = EncoderSizeLimits(
        widthAlignment = align, heightAlignment = align, minWidth = 16, minHeight = 16, maxWidth = maxW, maxHeight = maxH,
        isSupported = { w, h -> w <= maxW && h <= maxH && w % align == 0 && h % align == 0 && ((w + 15) / 16).toLong() * ((h + 15) / 16) <= maxBlocks },
    )

    @Test
    fun keepsSupportedSize() {
        assertEquals(Size(1080, 2400), fitToEncoder(Size(1080, 2400), limits(4096, 4096)))
    }

    @Test
    fun shrinksPortraitScreenForLandscapeOnlyLimits() {
        val got = fitToEncoder(Size(1080, 2400), limits(1920, 1088))!!
        assertTrue(got.width <= 1920 && got.height <= 1088)
        val aspect = 1080.0 / 2400
        assertEquals(aspect, got.width.toDouble() / got.height, 0.01)
    }

    @Test
    fun respectsAlignment() {
        val got = fitToEncoder(Size(1082, 2402), limits(4096, 4096, align = 16))!!
        assertEquals(0, got.width % 16)
        assertEquals(0, got.height % 16)
        assertTrue(got.width <= 1082 && got.height <= 2402)
    }

    @Test
    fun respectsBlockCountLimit() {
        // 1080p-class encoder: 8160 macroblocks.
        val got = fitToEncoder(Size(2560, 1440), limits(4096, 4096, maxBlocks = 8160))!!
        assertTrue(((got.width + 15) / 16) * ((got.height + 15) / 16) <= 8160)
        assertEquals(16.0 / 9, got.width.toDouble() / got.height, 0.02)
    }

    @Test
    fun impossibleReturnsNull() {
        assertNull(fitToEncoder(Size(1000, 1000), EncoderSizeLimits(2, 2, 16, 16, 4096, 4096) { _, _ -> false }))
    }
}
