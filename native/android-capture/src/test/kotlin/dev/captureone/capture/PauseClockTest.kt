package dev.captureone.capture

import dev.captureone.capture.media.MonotonicPts
import dev.captureone.capture.media.PauseClock
import dev.captureone.capture.media.framesToUs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PauseClockTest {
    private val ms = 1_000_000L
    private val origin = 5_000 * ms

    @Test
    fun ptsIsRelativeToOrigin() {
        val c = PauseClock(origin)
        assertEquals(0L, c.map(origin))
        assertEquals(1_500_000L, c.map(origin + 1500 * ms))
        assertNull("before origin is dropped", c.map(origin - 1))
    }

    @Test
    fun samplesDuringPauseAreDroppedAndLaterOnesShifted() {
        val c = PauseClock(origin)
        c.pause(origin + 2000 * ms)
        assertNull(c.map(origin + 2000 * ms))
        assertNull(c.map(origin + 2500 * ms))
        assertEquals(3000 * ms, c.resume(origin + 5000 * ms))
        assertNull("end-exclusive interval still covers 4999 ms", c.map(origin + 4999 * ms))
        assertEquals(2_000_000L, c.map(origin + 5000 * ms))
        assertEquals(3_000_000L, c.map(origin + 6000 * ms))
        // A sample captured before the pause and delivered late keeps its original PTS.
        assertEquals(1_000_000L, c.map(origin + 1000 * ms))
    }

    @Test
    fun multiplePausesAccumulateIdenticallyForAudioAndVideo() {
        val c = PauseClock(origin)
        c.pause(origin + 1000 * ms); c.resume(origin + 2000 * ms)
        c.pause(origin + 3000 * ms); c.resume(origin + 3500 * ms)
        val videoPts = c.map(origin + 4000 * ms)
        val audioPts = c.map(origin + 4000 * ms)
        assertEquals(2_500_000L, videoPts)
        assertEquals(videoPts, audioPts)
        assertEquals(2500.0, c.elapsedMs(origin + 4000 * ms), 0.001)
    }

    @Test
    fun elapsedFreezesWhilePaused() {
        val c = PauseClock(origin)
        c.pause(origin + 1000 * ms)
        assertEquals(1000.0, c.elapsedMs(origin + 1000 * ms), 0.001)
        assertEquals(1000.0, c.elapsedMs(origin + 9000 * ms), 0.001)
        assertTrue(c.isPaused)
    }

    @Test(expected = IllegalStateException::class)
    fun doublePauseRejected() {
        val c = PauseClock(origin)
        c.pause(origin + 1)
        c.pause(origin + 2)
    }

    @Test
    fun monotonicPtsIsStrictlyIncreasing() {
        val m = MonotonicPts()
        assertEquals(10L, m.next(10))
        assertEquals(11L, m.next(10))
        assertEquals(12L, m.next(5))
        assertEquals(100L, m.next(100))
    }

    @Test
    fun framesToMicroseconds() {
        assertEquals(1_000_000L, framesToUs(48_000, 48_000))
        assertEquals(21_333L, framesToUs(1024, 48_000))
        assertEquals(23_219L, framesToUs(1024, 44_100))
    }
}
