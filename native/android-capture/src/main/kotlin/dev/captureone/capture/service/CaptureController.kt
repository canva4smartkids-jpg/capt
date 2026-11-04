package dev.captureone.capture.service

import android.content.Context
import android.content.Intent
import android.os.Build
import dev.captureone.capture.contract.CaptureError
import dev.captureone.capture.contract.CaptureEvent
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.RecordingConfig
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.StopReason
import dev.captureone.capture.contract.toCaptureError
import dev.captureone.capture.engine.RecorderListener
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import java.time.Instant

/**
 * Process-wide entry point used by the phone and TV UIs. Commands go to [CaptureService] as intents
 * (the same path the notification actions use); state comes back as flows.
 */
object CaptureController : RecorderListener {
    private val _status = MutableStateFlow<RecordingStatus?>(null)
    /** Status of the current or last session; null before the first session. */
    val status: StateFlow<RecordingStatus?> = _status.asStateFlow()

    private val _events = MutableSharedFlow<CaptureEvent>(extraBufferCapacity = 64)
    val events: SharedFlow<CaptureEvent> = _events.asSharedFlow()

    private val _lastMedia = MutableStateFlow<MediaMetadata?>(null)
    /** Most recent finished recording or screenshot. */
    val lastMedia: StateFlow<MediaMetadata?> = _lastMedia.asStateFlow()

    private val _lastError = MutableStateFlow<CaptureError?>(null)
    val lastError: StateFlow<CaptureError?> = _lastError.asStateFlow()

    /** Set by the service so it can refresh its notification. */
    @Volatile internal var serviceListener: RecorderListener? = null

    val isBusy: Boolean
        get() = when (_status.value?.state) {
            RecordingState.COUNTDOWN, RecordingState.STARTING, RecordingState.RECORDING,
            RecordingState.PAUSED, RecordingState.STOPPING, RecordingState.FINALIZING -> true
            else -> false
        }

    fun clearError() {
        _lastError.value = null
    }

    // ------------------------------------------------------------------ commands

    /**
     * Starts a recording with the consent result from `MediaProjectionManager.createScreenCaptureIntent()`.
     * Each consent result is used exactly once (Android 14+: one token per session).
     */
    fun startRecording(context: Context, resultCode: Int, consentData: Intent, config: RecordingConfig) {
        val i = Intent(context, CaptureService::class.java)
            .setAction(CaptureService.ACTION_START_RECORDING)
            .putExtra(CaptureService.EXTRA_RESULT_CODE, resultCode)
            .putExtra(CaptureService.EXTRA_RESULT_DATA, consentData)
            .putExtra(CaptureService.EXTRA_CONFIG_JSON, CaptureJson.encodeToString(RecordingConfig.serializer(), config))
        startForegroundServiceCompat(context, i)
    }

    /** Takes a standalone screenshot with a fresh consent result. */
    fun takeScreenshot(context: Context, resultCode: Int, consentData: Intent, format: ImageFormat, jpegQuality: Int, fileNameTemplate: String) {
        val i = Intent(context, CaptureService::class.java)
            .setAction(CaptureService.ACTION_SCREENSHOT)
            .putExtra(CaptureService.EXTRA_RESULT_CODE, resultCode)
            .putExtra(CaptureService.EXTRA_RESULT_DATA, consentData)
            .putExtra(CaptureService.EXTRA_FORMAT, format.name)
            .putExtra(CaptureService.EXTRA_JPEG_QUALITY, jpegQuality)
            .putExtra(CaptureService.EXTRA_TEMPLATE, fileNameTemplate)
        startForegroundServiceCompat(context, i)
    }

    fun pause(context: Context) = command(context, CaptureService.ACTION_PAUSE)
    fun resume(context: Context) = command(context, CaptureService.ACTION_RESUME)
    fun stop(context: Context) = command(context, CaptureService.ACTION_STOP)
    fun cancel(context: Context) = command(context, CaptureService.ACTION_CANCEL)

    /** Screenshot from the active recording (no extra consent needed). */
    fun screenshotDuringRecording(context: Context, format: ImageFormat, jpegQuality: Int) {
        context.startService(
            Intent(context, CaptureService::class.java)
                .setAction(CaptureService.ACTION_RECORDING_SCREENSHOT)
                .putExtra(CaptureService.EXTRA_FORMAT, format.name)
                .putExtra(CaptureService.EXTRA_JPEG_QUALITY, jpegQuality),
        )
    }

    private fun command(context: Context, action: String) {
        // Only meaningful while the service runs; startService on a running service is allowed from background.
        if (!isBusy) return
        context.startService(Intent(context, CaptureService::class.java).setAction(action))
    }

    private fun startForegroundServiceCompat(context: Context, intent: Intent) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) context.startForegroundService(intent) else context.startService(intent)
    }

    // ------------------------------------------------------------------ RecorderListener (from the session)

    override fun onState(sessionId: String, from: RecordingState, to: RecordingState) {
        _events.tryEmit(CaptureEvent.StateChanged(sessionId, from, to, Instant.now().toString()))
        serviceListener?.onState(sessionId, from, to)
    }

    override fun onStatus(status: RecordingStatus) {
        _status.value = status
        _events.tryEmit(CaptureEvent.Status(status))
        status.countdownRemainingSec?.let { _events.tryEmit(CaptureEvent.Countdown(status.sessionId, it)) }
        serviceListener?.onStatus(status)
    }

    override fun onWarning(sessionId: String, message: String) {
        _events.tryEmit(CaptureEvent.Warning(sessionId, message))
        serviceListener?.onWarning(sessionId, message)
    }

    override fun onError(sessionId: String, error: CaptureError) {
        _lastError.value = error
        _events.tryEmit(CaptureEvent.Error(sessionId, error))
        serviceListener?.onError(sessionId, error)
    }

    override fun onCompleted(sessionId: String, reason: StopReason, result: Result<MediaMetadata>) {
        result.onSuccess { media ->
            _lastMedia.value = media
            _events.tryEmit(CaptureEvent.Completed(sessionId, reason, media))
        }
        result.onFailure { e ->
            val err = e.toCaptureError()
            if (_lastError.value == null) _lastError.value = err
        }
        serviceListener?.onCompleted(sessionId, reason, result)
    }

    override fun onScreenshot(media: MediaMetadata) {
        _lastMedia.value = media
        _events.tryEmit(CaptureEvent.Screenshot(media))
        serviceListener?.onScreenshot(media)
    }

    /** Used by the service for standalone screenshots and early failures (no session object). */
    internal fun reportStandaloneError(error: CaptureError) {
        _lastError.value = error
        _events.tryEmit(CaptureEvent.Error(null, error))
    }
}
