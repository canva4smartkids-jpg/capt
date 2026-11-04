package dev.captureone.capture.audio

import kotlin.math.abs
import kotlin.math.roundToInt
import kotlin.math.roundToLong
import kotlin.math.tanh

/**
 * Pure PCM helpers (16-bit signed, interleaved). Mixing happens in float with per-source gain and a
 * soft-knee limiter, so summing loud sources can never wrap around or hard-clip.
 */
object Pcm {
    /** Limiter knee: samples below this magnitude pass unchanged. */
    const val KNEE = 0.8f

    /** Soft-knee limiter mapping (-∞, ∞) into (-1, 1); identity for |x| <= [KNEE]. */
    fun limit(x: Float): Float {
        val a = abs(x)
        if (a <= KNEE) return x
        val over = (a - KNEE) / (1f - KNEE)
        val y = KNEE + (1f - KNEE) * tanh(over.toDouble()).toFloat()
        return if (x < 0) -y else y
    }

    fun toFloat(s: Short): Float = s / 32768f

    fun toShort(x: Float): Short {
        val v = (limit(x) * 32767f).roundToInt()
        return v.coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort()
    }

    /** Duplicates a mono buffer into interleaved stereo. */
    fun monoToStereo(mono: ShortArray, frames: Int = mono.size): ShortArray {
        val out = ShortArray(frames * 2)
        for (i in 0 until frames) {
            out[2 * i] = mono[i]
            out[2 * i + 1] = mono[i]
        }
        return out
    }

    /** Averages interleaved stereo into mono. */
    fun stereoToMono(stereo: ShortArray, frames: Int = stereo.size / 2): ShortArray {
        val out = ShortArray(frames)
        for (i in 0 until frames) out[i] = ((stereo[2 * i] + stereo[2 * i + 1]) / 2).toShort()
        return out
    }

    /**
     * Mixes two equally laid out interleaved buffers (null = silence) with gains into a new buffer.
     */
    fun mix(a: ShortArray?, gainA: Float, b: ShortArray?, gainB: Float, samples: Int): ShortArray {
        val out = ShortArray(samples)
        for (i in 0 until samples) {
            var acc = 0f
            if (a != null) acc += toFloat(a[i]) * gainA
            if (b != null) acc += toFloat(b[i]) * gainB
            out[i] = toShort(acc)
        }
        return out
    }
}

/**
 * Assigns absolute frame positions (frames since the session origin on the monotonic clock) to
 * consecutive PCM chunks read from one AudioRecord. Chunks are contiguous unless the capture
 * timestamp drifts from the running position by more than [resyncThresholdFrames] (overrun, device
 * hiccup), in which case the position re-anchors to the timestamp.
 */
class FramePositionTracker(
    private val sampleRate: Int,
    private val originNs: Long,
    private val resyncThresholdFrames: Long = (sampleRate / 20).toLong(), // 50 ms
) {
    private var next: Long = Long.MIN_VALUE
    var resyncCount: Int = 0
        private set

    /** @param captureNs monotonic capture time of the first frame of the chunk. */
    fun position(captureNs: Long, frames: Int): Long {
        val fromClock = ((captureNs - originNs).toDouble() * sampleRate / 1e9).roundToLong()
        val pos = if (next == Long.MIN_VALUE) {
            fromClock
        } else if (abs(fromClock - next) > resyncThresholdFrames) {
            resyncCount++
            fromClock
        } else {
            next
        }
        next = pos + frames
        return pos
    }
}

/**
 * Real-time mixer of N sources on a shared frame timeline. Producers [write] chunks at absolute frame
 * positions; the consumer [read]s mixed blocks up to a frame position (normally "now minus latency").
 * Missing data is silence, late data (before the read cursor) is dropped and counted.
 * Thread-safe. With a single source it also serves as a gap-filling jitter buffer.
 */
class TimelineMixer(
    val channels: Int,
    gains: FloatArray,
) {
    private class Chunk(val start: Long, val data: ShortArray)

    private val sourceCount = gains.size
    private val gains = gains.copyOf()
    private val queues = Array(sourceCount) { ArrayDeque<Chunk>() }
    private var cursor: Long = Long.MIN_VALUE

    var lateFramesDropped: Long = 0
        private set

    @Synchronized
    fun write(source: Int, startFrame: Long, interleaved: ShortArray) {
        require(source in 0 until sourceCount)
        require(interleaved.size % channels == 0) { "buffer is not a whole number of frames" }
        if (interleaved.isNotEmpty()) queues[source].addLast(Chunk(startFrame, interleaved))
    }

    /** Starts the read cursor (otherwise the first [read] starts at the earliest queued frame). */
    @Synchronized
    fun startAt(frame: Long) {
        cursor = frame
    }

    @Synchronized
    fun earliestQueuedFrame(): Long? = queues.mapNotNull { q -> q.minOfOrNull { it.start } }.minOrNull()

    /** Frames queued for [source] that have not been mixed yet. */
    @Synchronized
    fun queuedFrames(source: Int): Long = queues[source].sumOf { (it.data.size / channels).toLong() }

    /**
     * Mixes everything in [cursor, upToFrame) and advances the cursor.
     * @return start frame and interleaved samples, or null if there is nothing to emit yet.
     */
    @Synchronized
    fun read(upToFrame: Long): Pair<Long, ShortArray>? {
        if (cursor == Long.MIN_VALUE) cursor = earliestQueuedFrame() ?: return null
        if (upToFrame <= cursor) return null
        val frames = (upToFrame - cursor).toInt()
        val acc = FloatArray(frames * channels)
        for (s in 0 until sourceCount) {
            val keep = ArrayDeque<Chunk>()
            val g = gains[s]
            for (c in queues[s]) {
                val cFrames = c.data.size / channels
                val cEnd = c.start + cFrames
                if (cEnd <= cursor) { // arrived too late: entirely before the read cursor
                    lateFramesDropped += cFrames
                    continue
                }
                if (c.start >= upToFrame) { // entirely in the future
                    keep.addLast(c)
                    continue
                }
                if (c.start < cursor) lateFramesDropped += cursor - c.start
                val from = maxOf(c.start, cursor)
                val to = minOf(cEnd, upToFrame)
                for (f in from until to) {
                    val src = ((f - c.start) * channels).toInt()
                    val dst = ((f - cursor) * channels).toInt()
                    for (ch in 0 until channels) acc[dst + ch] += Pcm.toFloat(c.data[src + ch]) * g
                }
                if (cEnd > upToFrame) { // keep the unconsumed tail
                    val consumed = ((upToFrame - c.start) * channels).toInt()
                    keep.addLast(Chunk(upToFrame, c.data.copyOfRange(consumed, c.data.size)))
                }
            }
            queues[s] = keep
        }
        val start = cursor
        cursor = upToFrame
        return start to ShortArray(acc.size) { Pcm.toShort(acc[it]) }
    }
}
