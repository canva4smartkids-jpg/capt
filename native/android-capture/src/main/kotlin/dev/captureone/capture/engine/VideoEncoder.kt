package dev.captureone.capture.engine

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.view.Surface
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.VideoCodec
import dev.captureone.capture.media.PauseClock
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import kotlin.math.abs
import kotlin.math.roundToInt

/** Parameters of the video track. */
data class VideoEncoderConfig(
    val codec: VideoCodec,
    val size: Size,
    val fps: Int,
    val bitrateBps: Int,
    val keyframeIntervalSec: Double,
)

/**
 * Surface-input H.264/H.265 encoder. Frames come from a VirtualDisplay rendering into [inputSurface];
 * their timestamps are CLOCK_MONOTONIC (same base as `System.nanoTime()`), mapped through the shared
 * [PauseClock] before muxing. Runs MediaCodec in async mode on [handler]'s thread.
 */
class VideoEncoder(
    private val candidate: VideoEncoderCandidate,
    val config: VideoEncoderConfig,
    private val muxer: MuxerSink,
    private val pauseClock: PauseClock,
    private val handler: Handler,
    private val onFatal: (CaptureException) -> Unit,
) {
    private val codec: MediaCodec = try {
        MediaCodec.createByCodecName(candidate.name)
    } catch (e: Exception) {
        throw CaptureException(CaptureErrorCode.ENCODER_UNAVAILABLE, "Cannot create encoder ${candidate.name}: ${e.message}", cause = e)
    }
    lateinit var inputSurface: Surface
        private set

    private var track = -1
    private val eos = CountDownLatch(1)
    private val released = AtomicBoolean(false)
    private val awaitKeyFrame = AtomicBoolean(false)

    val framesEncoded = AtomicLong(0)
    val framesDropped = AtomicLong(0)
    val name: String get() = candidate.name

    /** @param minimal omit the optional keys some encoders reject. */
    fun format(minimal: Boolean = false): MediaFormat = MediaFormat.createVideoFormat(config.codec.mime, config.size.width, config.size.height).apply {
        setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
        setInteger(MediaFormat.KEY_BIT_RATE, candidate.clampBitrate(config.bitrateBps))
        setInteger(MediaFormat.KEY_FRAME_RATE, config.fps)
        // I-frame interval in seconds (float accepted since API 25).
        setFloat(MediaFormat.KEY_I_FRAME_INTERVAL, config.keyframeIntervalSec.toFloat())
        if (minimal) return@apply
        // A static screen produces no new frames: repeat the last one so the stream stays continuous.
        setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 1_000_000L / config.fps * 3)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            // Caps the encoded frame rate even though the display may refresh at 90/120 Hz.
            setFloat(MediaFormat.KEY_MAX_FPS_TO_ENCODER, config.fps.toFloat())
        }
    }

    /** Configures the codec and creates the input surface. */
    fun prepare() {
        codec.setCallback(callback, handler)
        val primary = format()
        try {
            codec.configure(primary, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        } catch (e: Exception) {
            // Some encoders reject the optional keys (max fps / repeat frame); retry with the minimal format.
            codec.reset()
            codec.setCallback(callback, handler)
            try {
                codec.configure(format(minimal = true), null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            } catch (e2: Exception) {
                codec.release()
                throw CaptureException(
                    CaptureErrorCode.ENCODER_UNAVAILABLE,
                    "Encoder ${candidate.name} rejected ${config.size} @ ${config.fps} fps: ${e2.message}",
                    cause = e2,
                )
            }
        }
        inputSurface = codec.createInputSurface()
    }

    fun start() = codec.start()

    /** Asks for an IDR frame now (used after resume) and drops non-key frames until it arrives. */
    fun requestSyncFrame() {
        awaitKeyFrame.set(true)
        try {
            codec.setParameters(Bundle().apply { putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0) })
        } catch (_: IllegalStateException) {
        }
    }

    /** Signals end of stream and waits (bounded) for the encoder to drain. */
    fun finish(timeoutMs: Long = 3000): Boolean {
        try {
            codec.signalEndOfInputStream()
        } catch (_: IllegalStateException) {
            return false
        }
        return eos.await(timeoutMs, TimeUnit.MILLISECONDS)
    }

    fun release() {
        if (!released.compareAndSet(false, true)) return
        try { codec.stop() } catch (_: Exception) {}
        try { codec.release() } catch (_: Exception) {}
        if (this::inputSurface.isInitialized) try { inputSurface.release() } catch (_: Exception) {}
    }

    private val callback = object : MediaCodec.Callback() {
        override fun onInputBufferAvailable(codec: MediaCodec, index: Int) = Unit // surface input

        override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
            if (released.get()) return
            try {
                val isEos = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
                val isConfig = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
                if (!isConfig && info.size > 0 && track >= 0) {
                    val nowNs = System.nanoTime()
                    var captureNs = info.presentationTimeUs * 1000L
                    // Defensive: VirtualDisplay frames carry CLOCK_MONOTONIC timestamps; anything wildly off is replaced by now.
                    if (abs(captureNs - nowNs) > 5_000_000_000L) captureNs = nowNs
                    val ptsUs = pauseClock.map(captureNs)
                    val isKey = info.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
                    if (ptsUs == null) {
                        framesDropped.incrementAndGet()
                    } else if (awaitKeyFrame.get() && !isKey) {
                        framesDropped.incrementAndGet()
                    } else {
                        if (isKey) awaitKeyFrame.set(false)
                        val buf = codec.getOutputBuffer(index)
                        if (buf != null) {
                            muxer.write(track, buf, info, ptsUs)
                            framesEncoded.incrementAndGet()
                        }
                    }
                }
                codec.releaseOutputBuffer(index, false)
                if (isEos) eos.countDown()
            } catch (e: CaptureException) {
                onFatal(e)
            } catch (e: Exception) {
                onFatal(CaptureException(CaptureErrorCode.ENCODER_FAILED, "Video output failed: ${e.message}", cause = e))
            }
        }

        override fun onError(codec: MediaCodec, e: MediaCodec.CodecException) {
            onFatal(
                CaptureException(
                    CaptureErrorCode.ENCODER_FAILED,
                    "Video encoder ${candidate.name} failed: ${e.diagnosticInfo}",
                    recoverable = e.isRecoverable || e.isTransient,
                    cause = e,
                ),
            )
            eos.countDown()
        }

        override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {
            if (track >= 0) return
            try {
                track = muxer.addTrack(format)
            } catch (e: Exception) {
                onFatal(CaptureException(CaptureErrorCode.ENCODER_FAILED, "Cannot add video track: ${e.message}", cause = e))
            }
        }
    }

    companion object {
        /** Bitrate in bps from the preset kbps (rounded). */
        fun kbpsToBps(kbps: Int): Int = (kbps * 1000.0).roundToInt()
    }
}
