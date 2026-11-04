package dev.captureone.capture.engine

import android.content.ContentValues
import android.content.Context
import android.graphics.BitmapFactory
import android.hardware.display.DisplayManager
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.StatFs
import android.provider.MediaStore
import android.util.DisplayMetrics
import android.view.Display
import android.view.WindowManager
import dev.captureone.capture.contract.AudioTrackInfo
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.IssueSeverity
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaValidation
import dev.captureone.capture.contract.Size
import dev.captureone.capture.contract.ValidationIssue
import dev.captureone.capture.contract.VideoCodec
import java.io.File
import java.io.FileInputStream
import java.io.IOException
import java.time.Instant

/** Physical pixel size and density of the default display (what MediaProjection mirrors). */
data class DisplayInfo(val size: Size, val densityDpi: Int, val refreshRate: Float) {
    companion object {
        fun of(context: Context): DisplayInfo {
            val dm = context.getSystemService(DisplayManager::class.java)
            val display = dm.getDisplay(Display.DEFAULT_DISPLAY)
            val metrics = DisplayMetrics()
            val size: Size
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val wm = context.getSystemService(WindowManager::class.java)
                val b = wm.maximumWindowMetrics.bounds
                size = Size(b.width(), b.height())
                @Suppress("DEPRECATION")
                display.getRealMetrics(metrics)
            } else {
                @Suppress("DEPRECATION")
                display.getRealMetrics(metrics)
                size = Size(metrics.widthPixels, metrics.heightPixels)
            }
            return DisplayInfo(size, metrics.densityDpi, display.refreshRate)
        }
    }
}

/** Free-space checks with StatFs. */
object Storage {
    fun freeBytes(dir: File): Long = try {
        StatFs(dir.absolutePath).availableBytes
    } catch (e: IllegalArgumentException) {
        -1
    }

    fun requireFree(dir: File, neededBytes: Long) {
        val free = freeBytes(dir)
        if (free in 0 until neededBytes) {
            throw CaptureException(
                CaptureErrorCode.STORAGE_FULL,
                "Only ${free / (1024 * 1024)} MB free; at least ${neededBytes / (1024 * 1024)} MB are required.",
                recoverable = true,
            )
        }
    }
}

/** Facts read back from a finished file. */
data class InspectedMedia(
    val size: Size?,
    val durationMs: Double?,
    val fps: Double?,
    val videoCodec: VideoCodec?,
    /** Average bitrate of the whole file (all tracks + container), kbps. */
    val overallBitrateKbps: Double?,
    val audioTracks: List<AudioTrackInfo>,
    val validation: MediaValidation,
)

/**
 * Validates written media by reading it back: MediaExtractor (tracks, codecs, formats) +
 * MediaMetadataRetriever (duration, size) for video; BitmapFactory for images.
 */
object MediaValidator {
    fun inspectVideo(file: File, expectAudioTracks: Int, audioLabels: List<String>, minDurationMs: Long = 100): InspectedMedia {
        val issues = ArrayList<ValidationIssue>()
        fun err(code: String, msg: String) { issues += ValidationIssue(IssueSeverity.ERROR, code, msg) }
        fun warn(code: String, msg: String) { issues += ValidationIssue(IssueSeverity.WARNING, code, msg) }
        var size: Size? = null
        var durationMs: Double? = null
        var fps: Double? = null
        var codec: VideoCodec? = null
        var bitrate: Double? = null
        val audio = ArrayList<AudioTrackInfo>()
        if (!file.isFile || file.length() == 0L) {
            err("missing", "Output file is missing or empty")
        } else {
            val ex = MediaExtractor()
            try {
                FileInputStream(file).use { fis -> ex.setDataSource(fis.fd) }
                var videoTracks = 0
                for (i in 0 until ex.trackCount) {
                    val f = ex.getTrackFormat(i)
                    val mime = f.getString(MediaFormat.KEY_MIME).orEmpty()
                    when {
                        mime.startsWith("video/") -> {
                            videoTracks++
                            codec = VideoCodec.entries.firstOrNull { it.mime == mime }
                            if (codec == null) warn("video-codec", "Unexpected video codec $mime")
                            size = Size(f.getInteger(MediaFormat.KEY_WIDTH), f.getInteger(MediaFormat.KEY_HEIGHT))
                            if (f.containsKey(MediaFormat.KEY_FRAME_RATE)) fps = runCatching { f.getInteger(MediaFormat.KEY_FRAME_RATE).toDouble() }.getOrNull()
                        }
                        mime.startsWith("audio/") -> {
                            val idx = audio.size
                            audio += AudioTrackInfo(
                                index = idx,
                                codec = if (mime == MediaFormat.MIMETYPE_AUDIO_AAC) "aac" else mime,
                                channels = f.getInteger(MediaFormat.KEY_CHANNEL_COUNT),
                                sampleRate = f.getInteger(MediaFormat.KEY_SAMPLE_RATE),
                                label = audioLabels.getOrNull(idx),
                                durationMs = if (f.containsKey(MediaFormat.KEY_DURATION)) f.getLong(MediaFormat.KEY_DURATION) / 1000.0 else null,
                            )
                        }
                    }
                }
                if (videoTracks != 1) err("video-tracks", "Expected 1 video track, found $videoTracks")
                if (audio.size != expectAudioTracks) err("audio-tracks", "Expected $expectAudioTracks audio track(s), found ${audio.size}")
            } catch (e: IOException) {
                err("unreadable", "MediaExtractor could not read the file: ${e.message}")
            } finally {
                ex.release()
            }
            val mmr = MediaMetadataRetriever()
            try {
                mmr.setDataSource(file.absolutePath)
                durationMs = mmr.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toDoubleOrNull()
                val w = mmr.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toIntOrNull()
                val h = mmr.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toIntOrNull()
                if (size == null && w != null && h != null) size = Size(w, h)
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    val frames = mmr.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT)?.toDoubleOrNull()
                    val d = durationMs
                    if (frames != null && d != null && d > 0) fps = frames * 1000.0 / d
                }
            } catch (e: RuntimeException) {
                err("metadata", "MediaMetadataRetriever failed: ${e.message}")
            } finally {
                try { mmr.release() } catch (_: Exception) {}
            }
            val d = durationMs
            if (d == null) err("duration", "No duration in the file") else if (d < minDurationMs) warn("short", "Recording is only ${d.toLong()} ms long")
            if (d != null && d > 0) bitrate = file.length() * 8.0 / (d / 1000.0) / 1000.0
        }
        val validation = MediaValidation(
            valid = issues.none { it.severity == IssueSeverity.ERROR },
            checkedAt = Instant.now().toString(),
            tool = "android.media.MediaExtractor+MediaMetadataRetriever",
            issues = issues,
        )
        return InspectedMedia(size, durationMs, fps, codec, bitrate, audio, validation)
    }

    fun inspectImage(file: File, expected: ImageFormat): InspectedMedia {
        val issues = ArrayList<ValidationIssue>()
        val opts = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeFile(file.absolutePath, opts)
        var size: Size? = null
        if (opts.outWidth <= 0 || opts.outHeight <= 0) {
            issues += ValidationIssue(IssueSeverity.ERROR, "undecodable", "BitmapFactory could not decode the image")
        } else {
            size = Size(opts.outWidth, opts.outHeight)
            if (opts.outMimeType != expected.mime) {
                issues += ValidationIssue(IssueSeverity.ERROR, "format", "Expected ${expected.mime}, got ${opts.outMimeType}")
            }
        }
        val validation = MediaValidation(issues.isEmpty(), Instant.now().toString(), "android.graphics.BitmapFactory", issues)
        return InspectedMedia(size, null, null, null, null, emptyList(), validation)
    }
}

/** Result of publishing a temp file into shared storage. */
data class Published(val uri: Uri?, val file: File?, val displayName: String, val warning: String?)

/**
 * Publishes finished files from app-specific temp storage into MediaStore
 * (`Movies/CaptureOne`, `Pictures/CaptureOne`). API 29+: RELATIVE_PATH + IS_PENDING, no permission.
 * API 26–28: WRITE_EXTERNAL_STORAGE is required; without it the file stays in app storage.
 */
object MediaStorePublisher {
    const val ALBUM = "CaptureOne"

    fun collection(kind: MediaKind): Uri = when (kind) {
        MediaKind.VIDEO -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY) else MediaStore.Video.Media.EXTERNAL_CONTENT_URI
        MediaKind.SCREENSHOT -> if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY) else MediaStore.Images.Media.EXTERNAL_CONTENT_URI
    }

    private fun publicDir(kind: MediaKind): File {
        val type = if (kind == MediaKind.VIDEO) Environment.DIRECTORY_MOVIES else Environment.DIRECTORY_PICTURES
        @Suppress("DEPRECATION")
        return File(Environment.getExternalStoragePublicDirectory(type), ALBUM)
    }

    fun canPublish(context: Context): Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q ||
        context.checkSelfPermission(android.Manifest.permission.WRITE_EXTERNAL_STORAGE) == android.content.pm.PackageManager.PERMISSION_GRANTED

    /**
     * Copies [temp] into MediaStore and deletes the temp file on success. On failure (no permission,
     * not enough space for the copy, I/O error) the temp file is kept and returned with a warning.
     */
    fun publish(context: Context, temp: File, displayName: String, mime: String, kind: MediaKind, createdAtMs: Long): Published {
        if (!canPublish(context)) {
            return Published(null, temp, temp.name, "Storage permission not granted: the file was kept in CaptureOne's app storage.")
        }
        val free = Storage.freeBytes(temp.parentFile ?: temp)
        if (free in 0 until temp.length() + 16L * 1024 * 1024) {
            return Published(null, temp, temp.name, "Not enough free space to copy the file to the shared ${if (kind == MediaKind.VIDEO) "Movies" else "Pictures"} folder; it was kept in app storage.")
        }
        val resolver = context.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, mime)
            put(MediaStore.MediaColumns.DATE_ADDED, createdAtMs / 1000)
            put(MediaStore.MediaColumns.DATE_MODIFIED, createdAtMs / 1000)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val base = if (kind == MediaKind.VIDEO) Environment.DIRECTORY_MOVIES else Environment.DIRECTORY_PICTURES
                put(MediaStore.MediaColumns.RELATIVE_PATH, "$base/$ALBUM")
                put(MediaStore.MediaColumns.IS_PENDING, 1)
            } else {
                val dir = publicDir(kind).apply { mkdirs() }
                var target = File(dir, displayName)
                var n = 2
                while (target.exists()) {
                    target = File(dir, displayName.substringBeforeLast('.') + " ($n)." + displayName.substringAfterLast('.'))
                    n++
                }
                @Suppress("DEPRECATION")
                put(MediaStore.MediaColumns.DATA, target.absolutePath)
                put(MediaStore.MediaColumns.DISPLAY_NAME, target.name)
            }
        }
        val uri = try {
            resolver.insert(collection(kind), values)
        } catch (e: Exception) {
            null
        } ?: return Published(null, temp, temp.name, "MediaStore refused the new item; the file was kept in app storage.")
        return try {
            resolver.openOutputStream(uri, "w").use { out ->
                requireNotNull(out) { "no output stream" }
                FileInputStream(temp).use { it.copyTo(out, 1 shl 16) }
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                resolver.update(uri, ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) }, null, null)
            }
            temp.delete()
            Published(uri, null, values.getAsString(MediaStore.MediaColumns.DISPLAY_NAME) ?: displayName, null)
        } catch (e: Exception) {
            try { resolver.delete(uri, null, null) } catch (_: Exception) {}
            Published(null, temp, temp.name, "Copy to shared storage failed (${e.message}); the file was kept in app storage.")
        }
    }
}
