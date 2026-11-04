package dev.captureone.capture.engine

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.media.framesToUs
import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * AAC-LC encoder driven synchronously from the audio pipeline thread. Input: interleaved 16-bit PCM
 * blocks with session-timeline PTS (already pause-adjusted). Output goes straight to the muxer.
 */
class AudioEncoder(
    val label: String,
    val sampleRate: Int,
    val channels: Int,
    bitrateKbps: Int,
    private val muxer: MuxerSink,
) {
    private val codec: MediaCodec
    private var track = -1
    private val info = MediaCodec.BufferInfo()
    private var released = false
    var eosReached = false
        private set

    init {
        val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, sampleRate, channels).apply {
            setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            setInteger(MediaFormat.KEY_BIT_RATE, bitrateKbps * 1000)
            setInteger(MediaFormat.KEY_MAX_INPUT_SIZE, 16 * 1024)
        }
        codec = try {
            MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).also {
                it.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
                it.start()
            }
        } catch (e: Exception) {
            throw CaptureException(CaptureErrorCode.ENCODER_UNAVAILABLE, "AAC encoder unavailable: ${e.message}", cause = e)
        }
    }

    /** Queues one PCM block whose first frame has presentation time [ptsUs]. */
    fun encode(pcm: ShortArray, ptsUs: Long) {
        val bytes = ByteBuffer.allocate(pcm.size * 2).order(ByteOrder.LITTLE_ENDIAN)
        bytes.asShortBuffer().put(pcm)
        var offset = 0
        val total = pcm.size * 2
        var spins = 0
        while (offset < total) {
            val idx = codec.dequeueInputBuffer(10_000)
            if (idx < 0) {
                drain()
                if (++spins > 200) throw CaptureException(CaptureErrorCode.ENCODER_FAILED, "AAC encoder ($label) stalled")
                continue
            }
            val buf = codec.getInputBuffer(idx) ?: continue
            buf.clear()
            val n = minOf(buf.remaining(), total - offset)
            bytes.limit(offset + n)
            bytes.position(offset)
            buf.put(bytes)
            val frameOffset = offset / (2 * channels)
            codec.queueInputBuffer(idx, 0, n, ptsUs + framesToUs(frameOffset.toLong(), sampleRate), 0)
            offset += n
            drain()
        }
    }

    /** Pulls all available output into the muxer. */
    fun drain(untilEos: Boolean = false, timeoutUs: Long = 0) {
        var waited = 0
        while (true) {
            val idx = codec.dequeueOutputBuffer(info, if (untilEos) 10_000 else timeoutUs)
            when {
                idx == MediaCodec.INFO_TRY_AGAIN_LATER -> {
                    if (!untilEos || ++waited > 300) return // up to ~3 s while draining for EOS
                }
                idx == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    if (track < 0) track = muxer.addTrack(codec.outputFormat)
                }
                idx >= 0 -> {
                    val buf = codec.getOutputBuffer(idx)
                    if (buf != null && track >= 0 && info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
                        muxer.write(track, buf, info, info.presentationTimeUs)
                    }
                    codec.releaseOutputBuffer(idx, false)
                    if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) {
                        eosReached = true
                        return
                    }
                }
            }
        }
    }

    /** Sends end-of-stream and drains the remaining output. */
    fun finish(lastPtsUs: Long) {
        if (released) return
        try {
            val idx = codec.dequeueInputBuffer(100_000)
            if (idx >= 0) codec.queueInputBuffer(idx, 0, 0, lastPtsUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
            drain(untilEos = true)
        } catch (_: IllegalStateException) {
        }
    }

    fun release() {
        if (released) return
        released = true
        try { codec.stop() } catch (_: Exception) {}
        try { codec.release() } catch (_: Exception) {}
    }
}
