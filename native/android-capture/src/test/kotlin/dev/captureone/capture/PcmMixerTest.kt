package dev.captureone.capture

import dev.captureone.capture.audio.FramePositionTracker
import dev.captureone.capture.audio.Pcm
import dev.captureone.capture.audio.TimelineMixer
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.abs

class PcmMixerTest {
    @Test
    fun silencePlusSilenceIsSilence() {
        val out = Pcm.mix(ShortArray(8), 1f, ShortArray(8), 1f, 8)
        assertArrayEquals(ShortArray(8), out)
    }

    @Test
    fun singleQuietSourcePassesThroughUnchanged() {
        val a = shortArrayOf(0, 100, -100, 12_000, -12_000, 26_000)
        val out = Pcm.mix(a, 1f, null, 1f, a.size)
        for (i in a.indices) assertTrue("sample $i: ${a[i]} vs ${out[i]}", abs(a[i] - out[i]) <= 1)
    }

    @Test
    fun loudSumsNeverWrapOrHardClip() {
        val a = ShortArray(4) { Short.MAX_VALUE }
        val b = ShortArray(4) { Short.MAX_VALUE }
        val out = Pcm.mix(a, 2f, b, 2f, 4)
        out.forEach { assertTrue("positive, below full scale: $it", it > 30_000 && it < Short.MAX_VALUE) }
        val n = Pcm.mix(ShortArray(4) { Short.MIN_VALUE }, 2f, ShortArray(4) { Short.MIN_VALUE }, 2f, 4)
        n.forEach { assertTrue("negative: $it", it < -30_000 && it > Short.MIN_VALUE) }
    }

    @Test
    fun limiterIsMonotonicAndBounded() {
        var prev = -10f
        var x = -8f
        while (x <= 8f) {
            val y = Pcm.limit(x)
            assertTrue(y >= prev)
            assertTrue(abs(y) < 1f)
            prev = y
            x += 0.01f
        }
        assertEquals(0.5f, Pcm.limit(0.5f), 0f)
    }

    @Test
    fun gainScales() {
        val out = Pcm.mix(shortArrayOf(1000, -1000), 2f, null, 1f, 2)
        assertArrayEquals(shortArrayOf(2000, -2000), out)
        assertArrayEquals(ShortArray(2), Pcm.mix(shortArrayOf(1000, -1000), 0f, null, 1f, 2))
    }

    @Test
    fun monoStereoConversion() {
        assertArrayEquals(shortArrayOf(1, 1, 2, 2), Pcm.monoToStereo(shortArrayOf(1, 2)))
        assertArrayEquals(shortArrayOf(2, 6), Pcm.stereoToMono(shortArrayOf(1, 3, 5, 7)))
    }

    @Test
    fun timelineMixerAlignsSourcesByFramePosition() {
        val m = TimelineMixer(channels = 1, gains = floatArrayOf(1f, 1f))
        m.startAt(0)
        m.write(0, 0, ShortArray(10) { 1000 })
        m.write(1, 5, ShortArray(10) { 500 }) // starts 5 frames later
        val (start, out) = m.read(10)!!
        assertEquals(0L, start)
        assertEquals(10, out.size)
        for (i in 0 until 5) assertEquals(1000, out[i].toInt(), 1)
        for (i in 5 until 10) assertEquals(1500, out[i].toInt(), 1)
        val (start2, out2) = m.read(15)!!
        assertEquals(10L, start2)
        out2.forEach { assertEquals(500, it.toInt(), 1) } // source 0 ended: silence + source 1
    }

    @Test
    fun gapsAreSilenceAndLateDataIsDropped() {
        val m = TimelineMixer(channels = 2, gains = floatArrayOf(1f))
        m.startAt(0)
        val (_, out) = m.read(4)!!
        assertArrayEquals(ShortArray(8), out)
        m.write(0, 0, ShortArray(8) { 7 }) // frames 0..3 arrive after they were emitted
        m.write(0, 4, ShortArray(4) { 9 })
        val (s, out2) = m.read(6)!!
        assertEquals(4L, s)
        assertArrayEquals(shortArrayOf(9, 9, 9, 9), out2)
        assertEquals(4L, m.lateFramesDropped)
        assertNull(m.read(6))
    }

    @Test
    fun partiallyConsumedChunkKeepsItsTail() {
        val m = TimelineMixer(channels = 1, gains = floatArrayOf(1f))
        m.startAt(0)
        m.write(0, 0, ShortArray(10) { it.toShort() })
        assertEquals(10L, m.queuedFrames(0))
        val (_, a) = m.read(4)!!
        assertArrayEquals(shortArrayOf(0, 1, 2, 3), a)
        assertEquals(6L, m.queuedFrames(0))
        val (_, b) = m.read(10)!!
        assertArrayEquals(shortArrayOf(4, 5, 6, 7, 8, 9), b)
    }

    @Test
    fun framePositionTrackerIsContiguousAndResyncsOnDrift() {
        val rate = 48_000
        val origin = 1_000_000_000L
        val t = FramePositionTracker(rate, origin)
        assertEquals(48_000L, t.position(origin + 1_000_000_000L, 960))
        // 10 ms of timestamp jitter is ignored: contiguous.
        assertEquals(48_960L, t.position(origin + 1_030_000_000L, 960))
        assertEquals(0, t.resyncCount)
        // 500 ms jump (overrun) re-anchors to the clock.
        assertEquals(96_000L, t.position(origin + 2_000_000_000L, 960))
        assertEquals(1, t.resyncCount)
    }
}
