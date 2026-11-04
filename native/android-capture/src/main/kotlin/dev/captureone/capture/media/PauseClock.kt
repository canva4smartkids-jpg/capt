package dev.captureone.capture.media

/**
 * Pause/resume bookkeeping on the single monotonic clock used by the whole session
 * (`System.nanoTime()`, which on Android is CLOCK_MONOTONIC — the same base as VirtualDisplay
 * frame timestamps and `AudioTimestamp.TIMEBASE_MONOTONIC`).
 *
 * Every captured sample (video frame or audio chunk) is passed through [map] with its *capture*
 * timestamp. Samples captured while paused are dropped (null); later samples have the accumulated
 * paused time subtracted. Video and audio use the identical function, so they stay in sync.
 */
class PauseClock(val originNs: Long) {
    private data class Interval(val startNs: Long, val endNs: Long)

    private val closed = ArrayList<Interval>()
    private var openPauseStartNs: Long = -1

    @get:Synchronized
    val isPaused: Boolean get() = openPauseStartNs >= 0

    @Synchronized
    fun pause(atNs: Long) {
        check(openPauseStartNs < 0) { "already paused" }
        require(atNs >= originNs) { "pause before origin" }
        val last = closed.lastOrNull()
        require(last == null || atNs >= last.endNs) { "pause before previous resume" }
        openPauseStartNs = atNs
    }

    /** Ends the current pause; returns its duration in ns. */
    @Synchronized
    fun resume(atNs: Long): Long {
        check(openPauseStartNs >= 0) { "not paused" }
        require(atNs >= openPauseStartNs) { "resume before pause" }
        closed += Interval(openPauseStartNs, atNs)
        val d = atNs - openPauseStartNs
        openPauseStartNs = -1
        return d
    }

    /** Total paused time that ended at or before [atNs] plus the running part of an open pause. */
    @Synchronized
    fun pausedNsBefore(atNs: Long): Long {
        var total = 0L
        for (i in closed) {
            if (i.endNs <= atNs) total += i.endNs - i.startNs
            else if (i.startNs < atNs) total += atNs - i.startNs
        }
        if (openPauseStartNs in 0 until atNs) total += atNs - openPauseStartNs
        return total
    }

    /**
     * Maps a capture timestamp to the session timeline in microseconds, or null when the sample was
     * captured during a pause (or before the session origin) and must be dropped.
     */
    @Synchronized
    fun map(captureNs: Long): Long? {
        if (captureNs < originNs) return null
        if (openPauseStartNs in 0..captureNs) return null
        for (i in closed) if (captureNs >= i.startNs && captureNs < i.endNs) return null
        return (captureNs - originNs - pausedNsBefore(captureNs)) / 1000L
    }

    /** Recorded (non-paused) time at [nowNs] in ms. */
    @Synchronized
    fun elapsedMs(nowNs: Long): Double = (nowNs - originNs - pausedNsBefore(nowNs)).coerceAtLeast(0) / 1_000_000.0
}

/**
 * Guarantees strictly increasing presentation timestamps per muxer track (MediaMuxer rejects
 * out-of-order samples; AAC tracks must be strictly increasing).
 */
class MonotonicPts {
    private var last = Long.MIN_VALUE

    fun next(ptsUs: Long): Long {
        val v = if (ptsUs <= last) last + 1 else ptsUs
        last = v
        return v
    }

    val lastUs: Long get() = last
}

/** Converts an audio frame position to microseconds. */
fun framesToUs(frames: Long, sampleRate: Int): Long = frames * 1_000_000L / sampleRate
