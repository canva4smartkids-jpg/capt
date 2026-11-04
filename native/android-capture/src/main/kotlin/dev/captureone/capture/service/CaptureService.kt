package dev.captureone.capture.service

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.os.Looper
import android.os.SystemClock
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.contract.CaptureError
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.RecordingConfig
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.StopReason
import dev.captureone.capture.contract.toCaptureError
import dev.captureone.capture.engine.DisplayInfo
import dev.captureone.capture.engine.RecorderListener
import dev.captureone.capture.engine.RecordingSession
import dev.captureone.capture.engine.ScreenshotWriter
import dev.captureone.capture.engine.Screenshotter
import dev.captureone.capture.presets.EncodingPresets
import dev.captureone.capture.util.Format

/**
 * Foreground service (type `mediaProjection`) hosting recordings and standalone screenshots.
 *
 * Order mandated by Android 10+/14+: consent activity result → startForegroundService → startForeground
 * (type mediaProjection) → getMediaProjection → registerCallback → createVirtualDisplay. Each consent
 * result is used once; the projection is stopped when the session ends.
 */
class CaptureService : Service(), RecorderListener {
    private var session: RecordingSession? = null
    private val main = Handler(Looper.getMainLooper())
    private var lastNotifyAt = 0L
    private var lastStatus: RecordingStatus? = null
    private var worker: HandlerThread? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        ensureChannels(this)
        CaptureController.serviceListener = this
    }

    override fun onDestroy() {
        if (CaptureController.serviceListener === this) CaptureController.serviceListener = null
        session?.cancel()
        worker?.quitSafely()
        super.onDestroy()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START_RECORDING -> handleStartRecording(intent)
            ACTION_SCREENSHOT -> handleScreenshot(intent)
            ACTION_PAUSE -> session?.pause()
            ACTION_RESUME -> session?.resume()
            ACTION_STOP -> session?.stop(StopReason.USER)
            ACTION_CANCEL -> session?.cancel()
            ACTION_RECORDING_SCREENSHOT -> {
                val format = intent.getStringExtra(EXTRA_FORMAT)?.let { runCatching { ImageFormat.valueOf(it) }.getOrNull() } ?: ImageFormat.PNG
                session?.screenshot(format, intent.getIntExtra(EXTRA_JPEG_QUALITY, 90))
            }
            else -> if (session == null) stopSelf(startId)
        }
        // Never restart without a fresh consent token.
        return START_NOT_STICKY
    }

    // ------------------------------------------------------------------ start paths

    private fun goForeground(notification: Notification): Boolean = try {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
        true
    } catch (e: Exception) {
        // e.g. ForegroundServiceStartNotAllowedException / SecurityException (missing consent or permission)
        CaptureController.reportStandaloneError(
            CaptureError(CaptureErrorCode.PERMISSION_DENIED, "Could not start the screen-capture service: ${e.message}", false),
        )
        false
    }

    private fun projectionFrom(intent: Intent): MediaProjection? {
        val code = intent.getIntExtra(EXTRA_RESULT_CODE, 0)
        val data: Intent? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            intent.getParcelableExtra(EXTRA_RESULT_DATA, Intent::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableExtra(EXTRA_RESULT_DATA)
        }
        if (data == null) return null
        val mpm = getSystemService(MediaProjectionManager::class.java)
        return try {
            mpm.getMediaProjection(code, data)
        } catch (e: SecurityException) {
            null
        } catch (e: IllegalStateException) {
            null // token already used (Android 14+)
        }
    }

    private fun platformName(): String = if (DeviceFactsCollector.isTelevision(this)) "android-tv" else "android"

    private fun handleStartRecording(intent: Intent) {
        if (session != null) {
            CaptureController.reportStandaloneError(CaptureError(CaptureErrorCode.DEVICE_BUSY, "A recording is already in progress.", true))
            return
        }
        if (!goForeground(buildNotification(null, "Preparing screen recording…"))) {
            stopSelf()
            return
        }
        val config = try {
            CaptureJson.decodeFromString(RecordingConfig.serializer(), requireNotNull(intent.getStringExtra(EXTRA_CONFIG_JSON)))
        } catch (e: Exception) {
            failEarly(CaptureError(CaptureErrorCode.INVALID_CONFIG, "Invalid recording configuration: ${e.message}", false))
            return
        }
        val projection = projectionFrom(intent) ?: run {
            failEarly(CaptureError(CaptureErrorCode.PERMISSION_DENIED, "Screen-capture consent was not granted or has expired. Start again to show the system dialog.", true))
            return
        }
        val presets = try {
            loadPresets(this)
        } catch (e: Exception) {
            projection.stop()
            failEarly(CaptureError(CaptureErrorCode.INTERNAL, "Encoding presets are missing from the app: ${e.message}", false))
            return
        }
        val s = RecordingSession(this, projection, config, presets, CaptureController, platformName())
        session = s
        s.begin()
    }

    private fun handleScreenshot(intent: Intent) {
        if (session != null) {
            // Reuse the running projection instead of asking for a second one.
            val format = intent.getStringExtra(EXTRA_FORMAT)?.let { runCatching { ImageFormat.valueOf(it) }.getOrNull() } ?: ImageFormat.PNG
            session?.screenshot(format, intent.getIntExtra(EXTRA_JPEG_QUALITY, 90))
            return
        }
        if (!goForeground(buildNotification(null, "Taking screenshot…"))) {
            stopSelf()
            return
        }
        val projection = projectionFrom(intent) ?: run {
            failEarly(CaptureError(CaptureErrorCode.PERMISSION_DENIED, "Screen-capture consent was not granted or has expired.", true))
            return
        }
        val format = intent.getStringExtra(EXTRA_FORMAT)?.let { runCatching { ImageFormat.valueOf(it) }.getOrNull() } ?: ImageFormat.PNG
        val quality = intent.getIntExtra(EXTRA_JPEG_QUALITY, 90)
        val template = intent.getStringExtra(EXTRA_TEMPLATE) ?: "CaptureOne {date} {time}"
        val thread = HandlerThread("c1-screenshot").apply { start() }
        worker = thread
        val imageThread = HandlerThread("c1-screenshot-image").apply { start() }
        Handler(thread.looper).post {
            val imageHandler = Handler(imageThread.looper)
            val callback = object : MediaProjection.Callback() {
                override fun onStop() = Unit
            }
            try {
                // Android 14+: register before createVirtualDisplay.
                projection.registerCallback(callback, imageHandler)
                // Let the consent dialog disappear before grabbing a frame.
                val bitmap = Screenshotter.capture(projection, DisplayInfo.of(this), imageHandler, settleMs = 600)
                val media = ScreenshotWriter.save(this, bitmap, format, quality, template, platformName())
                bitmap.recycle()
                main.post { CaptureController.onScreenshot(media) }
            } catch (e: Exception) {
                val err = e.toCaptureError()
                main.post { CaptureController.reportStandaloneError(err) }
            } finally {
                try { projection.unregisterCallback(callback) } catch (_: Exception) {}
                try { projection.stop() } catch (_: Exception) {}
                imageThread.quitSafely()
                main.post {
                    stopForegroundCompat()
                    stopSelf()
                }
                thread.quitSafely()
            }
        }
    }

    private fun failEarly(error: CaptureError) {
        CaptureController.reportStandaloneError(error)
        stopForegroundCompat()
        stopSelf()
    }

    private fun stopForegroundCompat() {
        stopForeground(STOP_FOREGROUND_REMOVE)
    }

    // ------------------------------------------------------------------ RecorderListener (via CaptureController)

    override fun onState(sessionId: String, from: RecordingState, to: RecordingState) {
        main.post { updateNotification(force = true) }
    }

    override fun onStatus(status: RecordingStatus) {
        lastStatus = status
        main.post { updateNotification(force = false) }
    }

    override fun onWarning(sessionId: String, message: String) = Unit

    override fun onError(sessionId: String, error: CaptureError) = Unit

    override fun onCompleted(sessionId: String, reason: StopReason, result: Result<MediaMetadata>) {
        main.post {
            session = null
            stopForegroundCompat()
            result.onSuccess { postResultNotification(it, reason) }
            stopSelf()
        }
    }

    override fun onScreenshot(media: MediaMetadata) {
        main.post { postResultNotification(media, null) }
    }

    // ------------------------------------------------------------------ notifications

    private fun updateNotification(force: Boolean) {
        val now = SystemClock.uptimeMillis()
        if (!force && now - lastNotifyAt < 1000) return
        lastNotifyAt = now
        val status = lastStatus ?: return
        val nm = getSystemService(NotificationManager::class.java)
        nm.notify(NOTIFICATION_ID, buildNotification(status, null))
    }

    private fun pending(action: String, req: Int): PendingIntent = PendingIntent.getService(
        this, req, Intent(this, CaptureService::class.java).setAction(action),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun contentIntent(): PendingIntent? {
        val launch = packageManager.getLaunchIntentForPackage(packageName) ?: packageManager.getLeanbackLaunchIntentForPackage(packageName) ?: return null
        launch.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        return PendingIntent.getActivity(this, 10, launch, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    @SuppressLint("DiscouragedApi") // the library has no R class in host builds; resolve by name with a framework fallback
    private fun smallIcon(): Int = resources.getIdentifier("c1_ic_notification", "drawable", packageName).takeIf { it != 0 }
        ?: android.R.drawable.presence_video_online

    private fun buildNotification(status: RecordingStatus?, text: String?): Notification {
        val b = Notification.Builder(this, CHANNEL_RECORDING)
        b.setSmallIcon(smallIcon())
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setContentTitle("CaptureOne")
        contentIntent()?.let { b.setContentIntent(it) }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) b.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
        when (status?.state) {
            RecordingState.COUNTDOWN -> {
                b.setContentTitle("Recording starts in ${status.countdownRemainingSec ?: 0} s")
                b.addAction(Notification.Action.Builder(null, "Cancel", pending(ACTION_CANCEL, 3)).build())
            }
            RecordingState.RECORDING -> {
                b.setContentTitle("Recording screen")
                b.setUsesChronometer(true)
                b.setShowWhen(true)
                b.setWhen(System.currentTimeMillis() - status.elapsedMs.toLong())
                b.setContentText("${Format.bytes(status.bytesWritten)} written")
                b.addAction(Notification.Action.Builder(null, "Pause", pending(ACTION_PAUSE, 1)).build())
                b.addAction(Notification.Action.Builder(null, "Stop", pending(ACTION_STOP, 2)).build())
            }
            RecordingState.PAUSED -> {
                b.setContentTitle("Recording paused · ${Format.duration(status.elapsedMs)}")
                b.setUsesChronometer(false)
                b.setShowWhen(false)
                b.addAction(Notification.Action.Builder(null, "Resume", pending(ACTION_RESUME, 4)).build())
                b.addAction(Notification.Action.Builder(null, "Stop", pending(ACTION_STOP, 2)).build())
            }
            RecordingState.STOPPING, RecordingState.FINALIZING -> b.setContentTitle("Saving recording…")
            RecordingState.STARTING -> b.setContentTitle("Starting recording…")
            else -> b.setContentTitle(text ?: "CaptureOne")
        }
        return b.build()
    }

    private fun postResultNotification(media: MediaMetadata, reason: StopReason?) {
        val nm = getSystemService(NotificationManager::class.java)
        if (!nm.areNotificationsEnabled()) return
        val b = Notification.Builder(this, CHANNEL_RESULTS)
        val what = if (media.kind == MediaKind.VIDEO) "Recording saved" else "Screenshot saved"
        val why = when (reason) {
            StopReason.MAX_DURATION -> " (max duration reached)"
            StopReason.MAX_FILE_SIZE -> " (max file size reached)"
            StopReason.LOW_DISK -> " (low storage)"
            StopReason.INTERRUPTED -> " (capture was interrupted)"
            StopReason.ERROR -> " (stopped after an error)"
            else -> ""
        }
        b.setSmallIcon(smallIcon()).setAutoCancel(true).setContentTitle(what + why).setContentText(media.fileName)
        contentIntent()?.let { b.setContentIntent(it) }
        nm.notify(RESULT_NOTIFICATION_ID, b.build())
    }

    companion object {
        const val ACTION_START_RECORDING = "dev.captureone.capture.START_RECORDING"
        const val ACTION_SCREENSHOT = "dev.captureone.capture.SCREENSHOT"
        const val ACTION_RECORDING_SCREENSHOT = "dev.captureone.capture.RECORDING_SCREENSHOT"
        const val ACTION_PAUSE = "dev.captureone.capture.PAUSE"
        const val ACTION_RESUME = "dev.captureone.capture.RESUME"
        const val ACTION_STOP = "dev.captureone.capture.STOP"
        const val ACTION_CANCEL = "dev.captureone.capture.CANCEL"

        const val EXTRA_RESULT_CODE = "resultCode"
        const val EXTRA_RESULT_DATA = "resultData"
        const val EXTRA_CONFIG_JSON = "configJson"
        const val EXTRA_FORMAT = "format"
        const val EXTRA_JPEG_QUALITY = "jpegQuality"
        const val EXTRA_TEMPLATE = "template"

        const val CHANNEL_RECORDING = "recording"
        const val CHANNEL_RESULTS = "results"
        private const val NOTIFICATION_ID = 4710
        private const val RESULT_NOTIFICATION_ID = 4711

        fun ensureChannels(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java)
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL_RECORDING, "Screen recording", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Shown while CaptureOne records or captures the screen"
                    setShowBadge(false)
                },
            )
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL_RESULTS, "Saved captures", NotificationManager.IMPORTANCE_DEFAULT).apply {
                    description = "Recording and screenshot results"
                },
            )
        }

        /** Loads presets.json copied into assets by the `copyEncodingPresets` Gradle task. */
        fun loadPresets(context: Context): EncodingPresets =
            context.assets.open(EncodingPresets.ASSET_NAME).bufferedReader().use { EncodingPresets.parse(it.readText()) }
    }
}
