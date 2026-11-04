package dev.captureone.capture.engine

import android.media.MediaCodec
import android.media.MediaFormat
import android.media.MediaMuxer
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.media.MonotonicPts
import java.io.File
import java.nio.ByteBuffer

/**
 * Thread-safe wrapper around [MediaMuxer] (MP4). Starts once every expected track has been added;
 * samples that arrive earlier are copied and replayed after start. Guarantees strictly increasing
 * PTS per track and never writes codec-config buffers (they travel in the track format).
 */
class MuxerSink(val file: File, private val expectedTracks: Int) {
    private class Pending(val track: Int, val data: ByteArray, val ptsUs: Long, val flags: Int)

    private val muxer = MediaMuxer(file.absolutePath, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
    private val pending = ArrayList<Pending>()
    private var pendingBytes = 0L
    private val pts = HashMap<Int, MonotonicPts>()
    private val samples = HashMap<Int, Long>()
    private var tracksAdded = 0
    private var released = false

    @Volatile var started = false
        private set

    @Volatile var bytesWritten: Long = 0
        private set

    /** Highest written PTS over all tracks (µs). */
    @Volatile var maxPtsUs: Long = 0
        private set

    @Synchronized
    fun addTrack(format: MediaFormat): Int {
        check(!started) { "muxer already started" }
        val idx = muxer.addTrack(format)
        pts[idx] = MonotonicPts()
        samples[idx] = 0
        tracksAdded++
        if (tracksAdded == expectedTracks) {
            muxer.start()
            started = true
            val replay = pending.toList()
            pending.clear()
            pendingBytes = 0
            for (p in replay) writeInternal(p.track, ByteBuffer.wrap(p.data), p.data.size, p.ptsUs, p.flags)
        }
        return idx
    }

    @Synchronized
    fun write(track: Int, buffer: ByteBuffer, info: MediaCodec.BufferInfo, ptsUs: Long) {
        if (released) return
        if (info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0 || info.size <= 0) return
        buffer.position(info.offset)
        buffer.limit(info.offset + info.size)
        if (!started) {
            val copy = ByteArray(info.size)
            buffer.get(copy)
            pendingBytes += copy.size
            if (pendingBytes > MAX_PENDING_BYTES) {
                throw CaptureException(CaptureErrorCode.ENCODER_FAILED, "An encoder never produced its output format; muxer could not start.")
            }
            pending += Pending(track, copy, ptsUs, info.flags)
            return
        }
        writeInternal(track, buffer.slice(), info.size, ptsUs, info.flags)
    }

    private fun writeInternal(track: Int, data: ByteBuffer, size: Int, ptsUs: Long, flags: Int) {
        val p = pts.getValue(track).next(ptsUs)
        val bi = MediaCodec.BufferInfo().apply { set(0, size, p, flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM.inv()) }
        data.position(0)
        data.limit(size)
        muxer.writeSampleData(track, data, bi)
        bytesWritten += size
        samples[track] = (samples[track] ?: 0) + 1
        if (p > maxPtsUs) maxPtsUs = p
    }

    @Synchronized
    fun sampleCount(track: Int): Long = samples[track] ?: 0

    @Synchronized
    fun totalSamples(): Long = samples.values.sum()

    /**
     * Stops and releases the muxer. Returns true when the MP4 was finalized (moov written).
     * Always releases, even if stop throws (e.g. no samples were written).
     */
    @Synchronized
    fun finish(): Boolean {
        if (released) return false
        released = true
        var ok = false
        try {
            if (started && totalSamples() > 0) {
                muxer.stop()
                ok = true
            }
        } catch (e: IllegalStateException) {
            ok = false
        } finally {
            try {
                muxer.release()
            } catch (_: Exception) {
            }
        }
        return ok
    }

    private companion object {
        const val MAX_PENDING_BYTES = 32L * 1024 * 1024
    }
}
