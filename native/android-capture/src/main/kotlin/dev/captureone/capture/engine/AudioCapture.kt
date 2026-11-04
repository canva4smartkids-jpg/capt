package dev.captureone.capture.engine

import android.Manifest
import android.annotation.SuppressLint
import android.annotation.TargetApi
import android.content.Context
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.AudioTimestamp
import android.media.MediaRecorder
import android.media.projection.MediaProjection
import android.os.Build
import android.os.Process
import dev.captureone.capture.audio.FramePositionTracker
import dev.captureone.capture.audio.Pcm
import dev.captureone.capture.audio.TimelineMixer
import dev.captureone.capture.contract.AudioSettings
import dev.captureone.capture.contract.AudioTrackMode
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.media.PauseClock
import dev.captureone.capture.media.framesToUs
import java.util.concurrent.atomic.AtomicBoolean

/** Which physical source an [AudioSource] reads. */
enum class AudioSourceKind(val label: String) { MICROPHONE("microphone"), SYSTEM("system") }

/**
 * One AudioRecord with its reader thread. Every chunk is stamped against the session's monotonic
 * clock (AudioRecord.getTimestamp with TIMEBASE_MONOTONIC when available) and written to the
 * mixer at its absolute frame position.
 */
class AudioSource(
    val kind: AudioSourceKind,
    private val record: AudioRecord,
    val channels: Int,
    private val sampleRate: Int,
    originNs: Long,
    private val sink: (startFrame: Long, interleaved: ShortArray) -> Unit,
    private val onError: (CaptureException) -> Unit,
) {
    private val running = AtomicBoolean(false)
    private val tracker = FramePositionTracker(sampleRate, originNs)
    private var thread: Thread? = null

    fun start() {
        record.startRecording()
        if (record.recordingState != AudioRecord.RECORDSTATE_RECORDING) {
            throw CaptureException(CaptureErrorCode.DEVICE_BUSY, "${kind.label} audio could not start (in use by another app?)")
        }
        running.set(true)
        thread = Thread({ loop() }, "c1-audio-${kind.label}").also { it.start() }
    }

    private fun loop() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val chunkFrames = sampleRate / 50 // 20 ms
        val buf = ShortArray(chunkFrames * channels)
        val ts = AudioTimestamp()
        var framesRead = 0L
        while (running.get()) {
            val n = record.read(buf, 0, buf.size)
            if (n < 0) {
                if (running.get()) onError(CaptureException(CaptureErrorCode.IO_ERROR, "${kind.label} audio read failed ($n)"))
                return
            }
            if (n == 0) continue
            val frames = n / channels
            val captureNs = if (record.getTimestamp(ts, AudioTimestamp.TIMEBASE_MONOTONIC) == AudioRecord.SUCCESS) {
                ts.nanoTime + (framesRead - ts.framePosition) * 1_000_000_000L / sampleRate
            } else {
                System.nanoTime() - frames * 1_000_000_000L / sampleRate
            }
            framesRead += frames
            val start = tracker.position(captureNs, frames)
            sink(start, buf.copyOf(frames * channels))
        }
    }

    fun stop() {
        running.set(false)
        try { record.stop() } catch (_: IllegalStateException) {}
        thread?.join(1000)
        thread = null
    }

    fun release() {
        stop()
        record.release()
    }

    companion object {
        private fun minBuffer(rate: Int, channelMask: Int): Int {
            val min = AudioRecord.getMinBufferSize(rate, channelMask, AudioFormat.ENCODING_PCM_16BIT)
            if (min <= 0) throw CaptureException(CaptureErrorCode.UNSUPPORTED, "Audio format $rate Hz is not supported")
            return maxOf(min * 4, rate / 5 * 4) // >= 200 ms
        }

        private fun hasRecordAudio(context: Context) =
            context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

        /** Microphone via `MIC`, falling back to `VOICE_RECOGNITION` (less processing) if MIC cannot initialise. */
        @SuppressLint("MissingPermission") // checked above
        fun microphone(context: Context, sampleRate: Int, deviceId: String?): Pair<AudioRecord, Int> {
            if (!hasRecordAudio(context)) throw CaptureException(CaptureErrorCode.PERMISSION_DENIED, "Microphone permission (RECORD_AUDIO) is not granted.")
            val mask = AudioFormat.CHANNEL_IN_MONO
            for (source in intArrayOf(MediaRecorder.AudioSource.MIC, MediaRecorder.AudioSource.VOICE_RECOGNITION)) {
                val rec = AudioRecord.Builder()
                    .setAudioSource(source)
                    .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(sampleRate).setChannelMask(mask).build())
                    .setBufferSizeInBytes(minBuffer(sampleRate, mask))
                    .build()
                if (rec.state == AudioRecord.STATE_INITIALIZED) {
                    deviceId?.toIntOrNull()?.let { id ->
                        val am = context.getSystemService(AudioManager::class.java)
                        am.getDevices(AudioManager.GET_DEVICES_INPUTS).firstOrNull { it.id == id }?.let { dev: AudioDeviceInfo -> rec.setPreferredDevice(dev) }
                    }
                    return rec to 1
                }
                rec.release()
            }
            throw CaptureException(CaptureErrorCode.DEVICE_BUSY, "The microphone could not be opened (in use by another app?)")
        }

        /**
         * Internal audio via AudioPlaybackCapture (API 29+). Captures media/game/unknown usages only;
         * apps can opt out. Requires the active MediaProjection and RECORD_AUDIO.
         */
        @TargetApi(Build.VERSION_CODES.Q)
        @SuppressLint("MissingPermission")
        fun playbackCapture(context: Context, projection: MediaProjection, sampleRate: Int): Pair<AudioRecord, Int> {
            if (!hasRecordAudio(context)) throw CaptureException(CaptureErrorCode.PERMISSION_DENIED, "Internal audio capture needs the microphone permission (RECORD_AUDIO).")
            val mask = AudioFormat.CHANNEL_IN_STEREO
            val config = AudioPlaybackCaptureConfiguration.Builder(projection)
                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                .build()
            val rec = AudioRecord.Builder()
                .setAudioPlaybackCaptureConfig(config)
                .setAudioFormat(AudioFormat.Builder().setEncoding(AudioFormat.ENCODING_PCM_16BIT).setSampleRate(sampleRate).setChannelMask(mask).build())
                .setBufferSizeInBytes(minBuffer(sampleRate, mask))
                .build()
            if (rec.state != AudioRecord.STATE_INITIALIZED) {
                rec.release()
                throw CaptureException(CaptureErrorCode.DEVICE_BUSY, "Internal audio capture could not be initialised on this device.")
            }
            return rec to 2
        }
    }
}

/**
 * Owns the audio sources, the timeline mixer(s) and the AAC encoder(s). One pipeline thread pulls
 * mixed blocks (with a small latency so both sources have arrived), maps them through the shared
 * [PauseClock] and feeds the encoder(s). Mixed mode: one stereo track; separate mode: one track per source.
 */
class AudioPipeline(
    private val settings: AudioSettings,
    private val originNs: Long,
    private val pauseClock: PauseClock,
    private val onError: (CaptureException) -> Unit,
) {
    private class Lane(val mixer: TimelineMixer, val encoder: AudioEncoder, val sourceIndex: Map<AudioSourceKind, Int>)

    private val sampleRate = settings.sampleRate
    private val sources = ArrayList<AudioSource>()
    private val lanes = ArrayList<Lane>()
    private val running = AtomicBoolean(false)
    private var thread: Thread? = null
    private val latencyFrames = (sampleRate * LATENCY_MS / 1000).toLong()
    private var owned: List<AudioRecord> = emptyList()

    /**
     * Opens [records] (kind → AudioRecord + channel count) and creates encoders writing into [muxer].
     * Ownership of the AudioRecords passes to the pipeline.
     */
    fun open(records: List<Triple<AudioSourceKind, AudioRecord, Int>>, muxer: MuxerSink) {
        owned = records.map { it.second }
        if (records.isEmpty()) return
        val gainOf = { k: AudioSourceKind -> (if (k == AudioSourceKind.MICROPHONE) settings.microphone.gain else settings.system.gain).toFloat() }
        if (settings.trackMode == AudioTrackMode.MIXED) {
            val kinds = records.map { it.first }
            val mixer = TimelineMixer(channels = 2, gains = FloatArray(kinds.size) { gainOf(kinds[it]) })
            val enc = AudioEncoder("mixed", sampleRate, 2, settings.bitrateKbps, muxer)
            lanes += Lane(mixer, enc, kinds.withIndex().associate { it.value to it.index })
        } else {
            for ((kind, _, ch) in records) {
                val mixer = TimelineMixer(channels = ch, gains = floatArrayOf(gainOf(kind)))
                lanes += Lane(mixer, AudioEncoder(kind.label, sampleRate, ch, settings.bitrateKbps, muxer), mapOf(kind to 0))
            }
        }
        for ((kind, rec, ch) in records) {
            val lane = lanes.first { kind in it.sourceIndex }
            val idx = lane.sourceIndex.getValue(kind)
            val upmix = lane.mixer.channels == 2 && ch == 1
            sources += AudioSource(kind, rec, ch, sampleRate, originNs, sink = { start, pcm ->
                lane.mixer.write(idx, start, if (upmix) Pcm.monoToStereo(pcm) else pcm)
            }, onError = onError)
        }
        for (lane in lanes) lane.mixer.startAt(0)
    }

    val encoderLabels: List<String> get() = lanes.map { it.encoder.label }

    fun start() {
        if (lanes.isEmpty()) return
        for (s in sources) s.start()
        running.set(true)
        thread = Thread({ loop() }, "c1-audio-mix").also { it.start() }
    }

    private fun nowFrame(): Long = (System.nanoTime() - originNs) * sampleRate / 1_000_000_000L

    private fun loop() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_AUDIO)
        try {
            while (running.get()) {
                pump(nowFrame() - latencyFrames)
                Thread.sleep(20)
            }
        } catch (e: CaptureException) {
            onError(e)
        } catch (e: InterruptedException) {
            // stopping
        } catch (e: Exception) {
            onError(CaptureException(CaptureErrorCode.ENCODER_FAILED, "Audio pipeline failed: ${e.message}", cause = e))
        }
    }

    /** Emits everything up to [upTo] in blocks of [BLOCK_FRAMES], dropping blocks captured while paused. */
    private fun pump(upTo: Long) {
        for (lane in lanes) {
            val (start, pcm) = lane.mixer.read(upTo) ?: continue
            val ch = lane.mixer.channels
            val frames = pcm.size / ch
            var f = 0
            while (f < frames) {
                val n = minOf(BLOCK_FRAMES, frames - f)
                val blockStart = start + f
                val captureNs = originNs + framesToUs(blockStart, sampleRate) * 1000L
                val pts = pauseClock.map(captureNs)
                if (pts != null) lane.encoder.encode(pcm.copyOfRange(f * ch, (f + n) * ch), pts)
                f += n
            }
            lane.encoder.drain()
        }
    }

    /** Stops capture, flushes buffered audio, ends the AAC streams. */
    fun finish() {
        for (s in sources) s.stop()
        running.set(false)
        thread?.interrupt()
        thread?.join(2000)
        if (thread?.isAlive != true) {
            try {
                pump(nowFrame())
            } catch (e: Exception) {
                // best effort on the way out
            }
        }
        for (lane in lanes) {
            val lastPts = pauseClock.map(System.nanoTime().coerceAtLeast(originNs)) ?: 0L
            lane.encoder.finish(lastPts)
        }
    }

    fun release() {
        running.set(false)
        thread?.interrupt()
        for (s in sources) try { s.release() } catch (_: Exception) {}
        for (r in owned) try { r.release() } catch (_: Exception) {} // not yet wrapped if open() failed half-way
        owned = emptyList()
        for (lane in lanes) lane.encoder.release()
        sources.clear()
        lanes.clear()
    }

    companion object {
        private const val LATENCY_MS = 200
        private const val BLOCK_FRAMES = 1024

        /** Number of muxer tracks for [sources] opened sources in [mode]. */
        fun trackCount(mode: AudioTrackMode, sources: Int): Int = when {
            sources == 0 -> 0
            mode == AudioTrackMode.MIXED -> 1
            else -> sources
        }
    }
}
