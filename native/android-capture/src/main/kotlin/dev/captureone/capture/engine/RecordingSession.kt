package dev.captureone.capture.engine

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.AudioRecord
import android.media.MediaMetadataRetriever
import android.media.projection.MediaProjection
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import dev.captureone.capture.contract.AudioTrackMode
import dev.captureone.capture.contract.CaptureError
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.CaptureTarget
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.QualityPreset
import dev.captureone.capture.contract.RecordingConfig
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.StopReason
import dev.captureone.capture.contract.toCaptureError
import dev.captureone.capture.library.LibraryRepository
import dev.captureone.capture.media.PauseClock
import dev.captureone.capture.presets.EncodingPresets
import dev.captureone.capture.presets.resolveOutputSize
import dev.captureone.capture.settings.CaptureSettings
import dev.captureone.capture.settings.SettingsStore
import dev.captureone.capture.state.RecordingStateMachine
import dev.captureone.capture.util.FileNaming
import java.io.File
import java.io.FileOutputStream
import java.time.Instant
import java.time.LocalDateTime
import java.util.UUID
import java.util.concurrent.CopyOnWriteArrayList

/** Session callbacks (same shape as the Rust `RecorderListener`). */
interface RecorderListener {
    fun onState(sessionId: String, from: RecordingState, to: RecordingState)
    fun onStatus(status: RecordingStatus)
    fun onWarning(sessionId: String, message: String)
    fun onError(sessionId: String, error: CaptureError)
    /** Exactly once per session, after finalization (user stop, limits, revoked projection, error, cancel). */
    fun onCompleted(sessionId: String, reason: StopReason, result: Result<MediaMetadata>)
    fun onScreenshot(media: MediaMetadata) {}
}

/**
 * One screen recording: MediaProjection → VirtualDisplay → MediaCodec (surface input) → MediaMuxer,
 * plus optional microphone / internal audio. All control runs on one HandlerThread; codec callbacks
 * and audio run on their own threads and report failures back to it.
 *
 * The caller must already be a foreground service of type `mediaProjection` and own [projection]
 * (obtained from a fresh consent result — one token per session on Android 14+).
 */
class RecordingSession(
    context: Context,
    private val projection: MediaProjection,
    val config: RecordingConfig,
    private val presets: EncodingPresets,
    private val listener: RecorderListener,
    private val platformName: String,
    val id: String = UUID.randomUUID().toString(),
) {
    private val app = context.applicationContext
    private val control = HandlerThread("c1-session").apply { start() }
    private val handler = Handler(control.looper)
    private val codecThread = HandlerThread("c1-video-codec").apply { start() }
    private val imageThread = HandlerThread("c1-image").apply { start() }
    private val imageHandler = Handler(imageThread.looper)
    private val warnings = CopyOnWriteArrayList<String>()
    private val settingsStore = SettingsStore(app)

    private val sm = RecordingStateMachine { from, to ->
        listener.onState(id, from, to)
        publishStatus()
    }

    val state: RecordingState get() = sm.state

    private var countdownRemaining: Int? = null
    private var lastError: CaptureError? = null
    private var completedSent = false
    private var projectionCallbackRegistered = false

    private var outputDir: File = File(config.output.directory)
    private var tempFile: File? = null
    private var muxer: MuxerSink? = null
    private var video: VideoEncoder? = null
    private var audio: AudioPipeline? = null
    /** AudioRecords opened during start-up and not yet owned by the pipeline (released on failure). */
    private var pendingRecords: List<Triple<AudioSourceKind, AudioRecord, Int>> = emptyList()
    private var virtualDisplay: VirtualDisplay? = null
    private var pauseClock: PauseClock? = null
    private var outputSize: Size? = null
    private var videoKbps = 0
    private var audioTrackCount = 0
    private val createdAt: Instant = Instant.now()
    private val createdLocal: LocalDateTime = LocalDateTime.now()
    private var displayBaseName = ""

    // ------------------------------------------------------------------ public control (thread-safe)

    /** Starts the countdown (if any) and then the capture. */
    fun begin() = handler.post { runCatchingToFail { beginInternal() } }

    fun pause() = handler.post { runCatchingToReport { pauseInternal() } }

    fun resume() = handler.post { runCatchingToReport { resumeInternal() } }

    fun stop(reason: StopReason = StopReason.USER) = handler.post { stopInternal(reason, null) }

    /** Cancels a countdown, or stops and discards an active recording. */
    fun cancel() = handler.post { cancelInternal() }

    /** Screenshot from the running recording (same resolution as the recording). */
    fun screenshot(format: ImageFormat, jpegQuality: Int) = handler.post { runCatchingToReport { screenshotInternal(format, jpegQuality) } }

    fun currentStatus(): RecordingStatus = buildStatus()

    // ------------------------------------------------------------------ countdown & start

    private fun beginInternal() {
        config.validate()?.let { throw CaptureException(it) }
        if (config.countdownSec > 0) {
            sm.transition(RecordingState.COUNTDOWN)
            countdownTick(config.countdownSec)
        } else {
            startCapture()
        }
    }

    private fun countdownTick(remaining: Int) {
        if (sm.state != RecordingState.COUNTDOWN) return
        countdownRemaining = remaining
        publishStatus()
        if (remaining <= 0) {
            countdownRemaining = null
            runCatchingToFail { startCapture() }
        } else {
            handler.postDelayed({ countdownTick(remaining - 1) }, 1000)
        }
    }

    private fun hasRecordAudio() = app.checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED

    private fun startCapture() {
        sm.transition(RecordingState.STARTING)
        val target = config.target
        if (target !is CaptureTarget.Display || target.displayId != CaptureSettings.DEFAULT_DISPLAY_ID) {
            throw CaptureException(CaptureErrorCode.UNSUPPORTED, "Android records the default display only (MediaProjection); window/region/device targets are not supported.")
        }
        val wantMic = config.audio.microphone.enabled
        val wantSystem = config.audio.system.enabled
        if ((wantMic || wantSystem) && !hasRecordAudio()) {
            throw CaptureException(CaptureErrorCode.PERMISSION_DENIED, "Audio recording needs the microphone permission (RECORD_AUDIO).")
        }
        if (wantSystem && Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            throw CaptureException(CaptureErrorCode.UNSUPPORTED, "Internal audio capture requires Android 10 (API 29).")
        }

        outputDir.mkdirs()
        if (!outputDir.isDirectory) throw CaptureException(CaptureErrorCode.IO_ERROR, "Cannot create ${outputDir.absolutePath}")
        Storage.requireFree(outputDir, (config.limits.minFreeDiskMb + 50L) * 1024 * 1024)

        val display = DisplayInfo.of(app)
        val desired = resolveOutputSize(display.size, config.video.resolution)

        // Audio sources first (playback capture needs the projection); ownership moves to the pipeline.
        val records = ArrayList<Triple<AudioSourceKind, AudioRecord, Int>>()
        try {
            if (wantMic) {
                val (rec, ch) = AudioSource.microphone(app, config.audio.sampleRate, config.audio.microphone.deviceId)
                records += Triple(AudioSourceKind.MICROPHONE, rec, ch)
            }
            if (wantSystem && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val (rec, ch) = AudioSource.playbackCapture(app, projection, config.audio.sampleRate)
                records += Triple(AudioSourceKind.SYSTEM, rec, ch)
            }
        } catch (e: Exception) {
            records.forEach { it.second.release() }
            throw e
        }
        pendingRecords = records

        displayBaseName = FileNaming.expand(config.output.fileNameTemplate, createdLocal, "Screen")
        val name = FileNaming.unique(displayBaseName, "mp4") { File(outputDir, it).exists() }
        val file = File(outputDir, name)
        tempFile = file

        audioTrackCount = AudioPipeline.trackCount(config.audio.trackMode, records.size)
        val sink = try {
            MuxerSink(file, expectedTracks = 1 + audioTrackCount)
        } catch (e: Exception) {
            throw CaptureException(CaptureErrorCode.IO_ERROR, "Cannot create ${file.name}: ${e.message}", cause = e)
        }
        muxer = sink

        val origin = System.nanoTime()
        val clock = PauseClock(origin)
        pauseClock = clock

        // Encoder: first candidate (hardware first unless disabled) that accepts the fitted size.
        val codecHandler = Handler(codecThread.looper)
        var chosen: VideoEncoder? = null
        var lastFailure: CaptureException? = null
        for (candidate in EncoderCatalog.candidates(config.video.codec, config.video.hardwareAcceleration)) {
            val size = candidate.fit(desired) ?: continue
            val kbps = presets.videoBitrateKbps(size, config.video.fps, config.video.codec, config.video.quality, config.video.bitrateKbps)
            val enc = try {
                VideoEncoder(
                    candidate,
                    VideoEncoderConfig(config.video.codec, size, config.video.fps, VideoEncoder.kbpsToBps(kbps), config.video.keyframeIntervalSec),
                    sink, clock, codecHandler, onFatal = { e -> handler.post { failInternal(e.error) } },
                ).also { it.prepare() }
            } catch (e: CaptureException) {
                lastFailure = e
                continue
            }
            if (!candidate.supportsFrameRate(size, config.video.fps)) {
                warn("Encoder ${candidate.name} does not advertise ${config.video.fps} fps at $size; frames may be dropped.")
            }
            if (size != desired) warn("Output size adjusted from $desired to $size for encoder ${candidate.name}.")
            chosen = enc
            outputSize = size
            videoKbps = kbps
            break
        }
        val enc = chosen ?: run {
            throw lastFailure ?: CaptureException(
                CaptureErrorCode.ENCODER_UNAVAILABLE,
                "No ${config.video.codec.name} encoder on this device accepts $desired.",
            )
        }
        video = enc

        val realPipeline = AudioPipeline(config.audio, origin, clock, onError = { e -> handler.post { failInternal(e.error) } })
        audio = realPipeline // owns the AudioRecords from here on, even if open() fails half-way
        pendingRecords = emptyList()
        realPipeline.open(records, sink)

        // Android 14+: the callback must be registered before createVirtualDisplay.
        projection.registerCallback(projectionCallback, handler)
        projectionCallbackRegistered = true

        enc.start()
        val size = requireNotNull(outputSize)
        virtualDisplay = try {
            projection.createVirtualDisplay(
                "CaptureOne", size.width, size.height, display.densityDpi,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, enc.inputSurface, null, handler,
            )
        } catch (e: SecurityException) {
            throw CaptureException(CaptureErrorCode.PERMISSION_DENIED, "Screen capture consent is no longer valid; request it again.", cause = e)
        } ?: throw CaptureException(CaptureErrorCode.INTERNAL, "createVirtualDisplay returned null")

        audio?.start()
        sm.transition(RecordingState.RECORDING)
        handler.postDelayed(ticker, TICK_MS)
    }

    // ------------------------------------------------------------------ pause / resume

    private fun pauseInternal() {
        if (sm.state != RecordingState.RECORDING) throw CaptureException(CaptureErrorCode.INVALID_STATE, "Can only pause while recording.")
        sm.transition(RecordingState.PAUSED)
        requireNotNull(pauseClock).pause(System.nanoTime())
        virtualDisplay?.surface = null // stop feeding the encoder while paused
        publishStatus()
    }

    private fun resumeInternal() {
        if (sm.state != RecordingState.PAUSED) throw CaptureException(CaptureErrorCode.INVALID_STATE, "Can only resume while paused.")
        requireNotNull(pauseClock).resume(System.nanoTime())
        video?.let { v ->
            virtualDisplay?.surface = v.inputSurface
            v.requestSyncFrame()
        }
        sm.transition(RecordingState.RECORDING)
    }

    // ------------------------------------------------------------------ status & limits

    private val ticker: Runnable = object : Runnable {
        override fun run() {
            val st = sm.state
            if (st != RecordingState.RECORDING && st != RecordingState.PAUSED) return
            val status = buildStatus()
            publishStatus(status)
            val limits = config.limits
            val free = status.freeDiskBytes ?: -1
            when {
                limits.maxDurationSec != null && status.elapsedMs >= limits.maxDurationSec * 1000.0 -> stopInternal(StopReason.MAX_DURATION, null)
                limits.maxFileSizeMb != null && status.bytesWritten >= limits.maxFileSizeMb * 1024L * 1024L -> stopInternal(StopReason.MAX_FILE_SIZE, null)
                free in 0 until limits.minFreeDiskMb * 1024L * 1024L -> stopInternal(
                    StopReason.LOW_DISK,
                    CaptureError(CaptureErrorCode.STORAGE_FULL, "Free space dropped below ${limits.minFreeDiskMb} MB; recording stopped and saved.", true),
                )
                else -> handler.postDelayed(this, TICK_MS)
            }
        }
    }

    private fun estimatedBytesPerMinute(): Double? {
        if (videoKbps <= 0) return null
        val audioKbps = audioTrackCount * config.audio.bitrateKbps.toDouble()
        return presets.estimateFileSizeBytes(videoKbps.toDouble(), audioKbps, 60.0).toDouble()
    }

    private fun buildStatus(): RecordingStatus {
        val now = System.nanoTime()
        return RecordingStatus(
            sessionId = id,
            state = sm.state,
            elapsedMs = pauseClock?.elapsedMs(now) ?: 0.0,
            countdownRemainingSec = countdownRemaining,
            bytesWritten = muxer?.bytesWritten ?: 0,
            framesEncoded = video?.framesEncoded?.get() ?: 0,
            framesDropped = video?.framesDropped?.get() ?: 0,
            estimatedBytesPerMinute = estimatedBytesPerMinute(),
            freeDiskBytes = Storage.freeBytes(outputDir).takeIf { it >= 0 },
            outputPath = tempFile?.absolutePath,
            encoder = video?.name,
            warnings = warnings.toList(),
            error = lastError,
        )
    }

    private fun publishStatus(status: RecordingStatus = buildStatus()) = listener.onStatus(status)

    private fun warn(message: String) {
        warnings += message
        listener.onWarning(id, message)
    }

    // ------------------------------------------------------------------ projection callbacks

    private val projectionCallback = object : MediaProjection.Callback() {
        override fun onStop() {
            // User tapped "Stop sharing" / the system revoked capture (e.g. screen locked on some devices).
            if (sm.state == RecordingState.RECORDING || sm.state == RecordingState.PAUSED) {
                stopInternal(
                    StopReason.INTERRUPTED,
                    CaptureError(CaptureErrorCode.INTERRUPTED, "Screen capture was stopped by the system or the user. The partial recording was saved.", false),
                )
            } else if (sm.state == RecordingState.COUNTDOWN || sm.state == RecordingState.STARTING) {
                failInternal(CaptureError(CaptureErrorCode.INTERRUPTED, "Screen capture was revoked before recording started.", false))
            }
        }

        override fun onCapturedContentResize(width: Int, height: Int) {
            // API 34+, single-app capture: the system scales the app window into our fixed-size display.
            outputSize?.let { out ->
                if (width > 0 && height > 0 && (width.toLong() * out.height != height.toLong() * out.width)) {
                    warn("Captured app is ${width}x$height; it is scaled into the ${out} recording with borders.")
                }
            }
        }

        override fun onCapturedContentVisibilityChanged(isVisible: Boolean) {
            // API 34+, single-app capture: when the app is hidden the recording shows its last frame.
            if (!isVisible) warn("The captured app is no longer visible; the recording shows a frozen/blank frame until it returns.")
        }
    }

    // ------------------------------------------------------------------ stop / finalize

    private fun cancelInternal() {
        when (sm.state) {
            RecordingState.IDLE, RecordingState.COUNTDOWN -> {
                if (sm.state == RecordingState.COUNTDOWN) sm.transition(RecordingState.IDLE)
                releaseAll()
                complete(StopReason.USER, Result.failure(CaptureException(CaptureErrorCode.INTERRUPTED, "Recording cancelled before it started.", recoverable = true)))
            }
            RecordingState.RECORDING, RecordingState.PAUSED -> {
                stopInternal(StopReason.USER, null, discard = true)
            }
            else -> Unit
        }
    }

    private fun failInternal(error: CaptureError) {
        val st = sm.state
        if (st == RecordingState.RECORDING || st == RecordingState.PAUSED) {
            stopInternal(StopReason.ERROR, error) // salvages the partial file and reports the error
            return
        }
        lastError = error
        listener.onError(id, error)
        when (st) {
            RecordingState.IDLE, RecordingState.COUNTDOWN -> {
                // countdown -> failed is not a legal transition: go back to idle and report.
                if (sm.state == RecordingState.COUNTDOWN) sm.transition(RecordingState.IDLE)
                releaseAll()
                complete(StopReason.ERROR, Result.failure(CaptureException(error)))
            }
            RecordingState.STARTING -> {
                sm.tryTransition(RecordingState.FAILED)
                releaseAll()
                tempFile?.delete()
                complete(StopReason.ERROR, Result.failure(CaptureException(error)))
            }
            else -> Unit // stopping/finalizing: the finalizer handles it; completed/failed: too late
        }
    }

    private fun stopInternal(reason: StopReason, error: CaptureError?, discard: Boolean = false) {
        val st = sm.state
        if (st == RecordingState.COUNTDOWN) {
            cancelInternal()
            return
        }
        if (st != RecordingState.RECORDING && st != RecordingState.PAUSED) return
        handler.removeCallbacks(ticker)
        if (error != null) {
            lastError = error
            listener.onError(id, error)
        }
        sm.transition(RecordingState.STOPPING)

        // 1. stop producing frames, 2. drain audio, 3. drain video.
        try { virtualDisplay?.release() } catch (_: Exception) {}
        virtualDisplay = null
        try { audio?.finish() } catch (e: Exception) { warn("Audio did not finish cleanly: ${e.message}") }
        val videoDrained = try { video?.finish() ?: false } catch (_: Exception) { false }
        if (!videoDrained) warn("Video encoder did not signal end-of-stream in time; the last frames may be missing.")

        sm.transition(RecordingState.FINALIZING)
        val muxOk = try { muxer?.finish() ?: false } catch (_: Exception) { false }
        releaseAll()

        val file = tempFile
        if (discard) {
            file?.delete()
            sm.transition(RecordingState.FAILED)
            complete(reason, Result.failure(CaptureException(CaptureErrorCode.INTERRUPTED, "Recording discarded.", recoverable = true)))
            return
        }
        if (file == null || !muxOk) {
            file?.delete()
            val err = error ?: CaptureError(CaptureErrorCode.ENCODER_FAILED, "No media was written; the file could not be finalized.", false)
            lastError = err
            sm.transition(RecordingState.FAILED)
            complete(reason, Result.failure(CaptureException(err)))
            return
        }
        try {
            val media = finalizeFile(file)
            if (media.validation?.valid == true) {
                video?.name?.let { settingsStore.markEncoderVerified(it) }
                sm.transition(RecordingState.COMPLETED)
                complete(reason, Result.success(media))
            } else {
                val issues = media.validation?.issues?.joinToString { it.message }.orEmpty()
                val err = CaptureError(CaptureErrorCode.ENCODER_FAILED, "The recording failed validation: $issues", false)
                lastError = err
                listener.onError(id, err)
                sm.transition(RecordingState.FAILED)
                complete(reason, Result.failure(CaptureException(err)))
            }
        } catch (e: Exception) {
            val err = e.toCaptureError()
            lastError = err
            listener.onError(id, err)
            sm.tryTransition(RecordingState.FAILED)
            complete(reason, Result.failure(CaptureException(err, e)))
        }
    }

    /** Validates the temp file, writes a thumbnail, publishes to MediaStore and records history. */
    private fun finalizeFile(file: File): MediaMetadata {
        val labels = when {
            audioTrackCount == 0 -> emptyList()
            config.audio.trackMode == AudioTrackMode.MIXED -> listOf(
                if (config.audio.microphone.enabled && config.audio.system.enabled) "mixed" else if (config.audio.microphone.enabled) "microphone" else "system",
            )
            else -> audio?.encoderLabels.orEmpty()
        }
        val inspected = MediaValidator.inspectVideo(file, expectAudioTracks = audioTrackCount, audioLabels = labels)
        val sizeBytes = file.length()
        val thumb = writeThumbnail(file)
        val out = outputSize
        val published = if (inspected.validation.valid) {
            MediaStorePublisher.publish(app, file, file.name, "video/mp4", MediaKind.VIDEO, createdAt.toEpochMilli())
        } else {
            Published(null, file, file.name, null)
        }
        published.warning?.let { warn(it) }
        val uri: Uri = published.uri ?: Uri.fromFile(published.file ?: file)
        val audioKbps = inspected.audioTracks.size * config.audio.bitrateKbps
        val media = MediaMetadata(
            id = id,
            kind = MediaKind.VIDEO,
            path = published.uri?.toString() ?: (published.file ?: file).absolutePath,
            fileName = published.displayName,
            title = displayBaseName,
            createdAt = createdAt.toString(),
            sizeBytes = sizeBytes,
            width = inspected.size?.width ?: out?.width ?: 1,
            height = inspected.size?.height ?: out?.height ?: 1,
            durationMs = inspected.durationMs,
            fps = inspected.fps ?: config.video.fps.toDouble(),
            videoCodec = inspected.videoCodec ?: config.video.codec,
            videoBitrateKbps = inspected.overallBitrateKbps?.let { (it - audioKbps).coerceAtLeast(0.0) },
            audioTracks = inspected.audioTracks,
            container = "mp4",
            sourceName = "Screen",
            platform = platformName,
            validation = inspected.validation,
            thumbnailPath = thumb?.absolutePath,
        )
        if (inspected.validation.valid) LibraryRepository.get(app).add(media, uri, published.uri != null)
        return media
    }

    private fun writeThumbnail(file: File): File? = try {
        val mmr = MediaMetadataRetriever()
        try {
            mmr.setDataSource(file.absolutePath)
            val frame = mmr.getFrameAtTime(0) ?: return null
            val dir = File(app.filesDir, "thumbnails").apply { mkdirs() }
            val out = File(dir, "$id.jpg")
            val scale = 384f / maxOf(frame.width, frame.height)
            val bmp = if (scale < 1f) Bitmap.createScaledBitmap(frame, (frame.width * scale).toInt().coerceAtLeast(1), (frame.height * scale).toInt().coerceAtLeast(1), true) else frame
            FileOutputStream(out).use { bmp.compress(Bitmap.CompressFormat.JPEG, 85, it) }
            out
        } finally {
            mmr.release()
        }
    } catch (e: Exception) {
        null
    }

    // ------------------------------------------------------------------ screenshot during recording

    private fun screenshotInternal(format: ImageFormat, jpegQuality: Int) {
        val vd = virtualDisplay
        val size = outputSize
        val enc = video
        if (vd == null || size == null || enc == null || !sm.state.isActive) {
            throw CaptureException(CaptureErrorCode.INVALID_STATE, "Screenshots from a recording need an active recording.")
        }
        val restore = if (sm.state == RecordingState.PAUSED) null else enc.inputSurface
        val bitmap = Screenshotter.captureFrom(vd, size, restore ?: enc.inputSurface, imageHandler)
        if (restore == null) vd.surface = null
        val media = ScreenshotWriter.save(app, bitmap, format, jpegQuality, config.output.fileNameTemplate, platformName)
        bitmap.recycle()
        listener.onScreenshot(media)
    }

    // ------------------------------------------------------------------ helpers

    private inline fun runCatchingToFail(block: () -> Unit) {
        try {
            block()
        } catch (e: Exception) {
            failInternal(e.toCaptureError())
        } catch (e: LinkageError) {
            failInternal(e.toCaptureError())
        }
    }

    private inline fun runCatchingToReport(block: () -> Unit) {
        try {
            block()
        } catch (e: LinkageError) {
            listener.onError(id, e.toCaptureError())
            publishStatus()
        } catch (e: Exception) {
            val err = e.toCaptureError()
            listener.onError(id, err)
            publishStatus()
        }
    }

    private fun releaseAll() {
        handler.removeCallbacks(ticker)
        pendingRecords.forEach { runCatching { it.second.release() } }
        pendingRecords = emptyList()
        try { virtualDisplay?.release() } catch (_: Exception) {}
        virtualDisplay = null
        try { audio?.release() } catch (_: Exception) {}
        audio = null
        try { video?.release() } catch (_: Exception) {}
        muxer?.let { m -> if (!m.started) runCatching { m.finish() } }
        if (projectionCallbackRegistered) {
            try { projection.unregisterCallback(projectionCallback) } catch (_: Exception) {}
            projectionCallbackRegistered = false
        }
        try { projection.stop() } catch (_: Exception) {}
    }

    private fun complete(reason: StopReason, result: Result<MediaMetadata>) {
        if (completedSent) return
        completedSent = true
        publishStatus()
        listener.onCompleted(id, reason, result)
        handler.post {
            codecThread.quitSafely()
            imageThread.quitSafely()
            control.quitSafely()
        }
    }

    private companion object {
        const val TICK_MS = 500L
    }
}

/** Writes a screenshot bitmap to temp storage, validates it, publishes it and records it in the library. */
object ScreenshotWriter {
    fun save(context: Context, bitmap: Bitmap, format: ImageFormat, jpegQuality: Int, template: String, platformName: String): MediaMetadata {
        val app = context.applicationContext
        val dir = File(app.getExternalFilesDir(null) ?: app.filesDir, "screenshots-tmp").apply { mkdirs() }
        Storage.requireFree(dir, 20L * 1024 * 1024)
        val now = LocalDateTime.now()
        val created = Instant.now()
        val base = FileNaming.expand(template, now, "Screen")
        val name = FileNaming.unique(base, format.extension) { File(dir, it).exists() }
        val file = File(dir, name)
        Screenshotter.write(bitmap, file, format, jpegQuality)
        val inspected = MediaValidator.inspectImage(file, format)
        if (!inspected.validation.valid) {
            file.delete()
            throw CaptureException(CaptureErrorCode.IO_ERROR, "Screenshot failed validation: ${inspected.validation.issues.joinToString { it.message }}")
        }
        val sizeBytes = file.length()
        val published = MediaStorePublisher.publish(app, file, name, format.mime, MediaKind.SCREENSHOT, created.toEpochMilli())
        val id = UUID.randomUUID().toString()
        val uri = published.uri ?: Uri.fromFile(published.file ?: file)
        val media = MediaMetadata(
            id = id,
            kind = MediaKind.SCREENSHOT,
            path = published.uri?.toString() ?: (published.file ?: file).absolutePath,
            fileName = published.displayName,
            title = base,
            createdAt = created.toString(),
            sizeBytes = sizeBytes,
            width = inspected.size?.width ?: bitmap.width,
            height = inspected.size?.height ?: bitmap.height,
            imageFormat = format,
            container = if (format == ImageFormat.PNG) "png" else "jpeg",
            sourceName = "Screen",
            platform = platformName,
            validation = inspected.validation,
        )
        LibraryRepository.get(app).add(media, uri, published.uri != null)
        return media
    }
}
