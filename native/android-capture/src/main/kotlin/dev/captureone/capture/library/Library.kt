package dev.captureone.capture.library

import android.app.PendingIntent
import android.app.RecoverableSecurityException
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.IntentSender
import android.database.sqlite.SQLiteDatabase
import android.database.sqlite.SQLiteOpenHelper
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.media.MediaMetadataRetriever
import android.media.ThumbnailUtils
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.engine.MediaStorePublisher
import dev.captureone.capture.engine.Storage
import dev.captureone.capture.util.FileNaming
import java.io.File
import java.io.FileOutputStream

/** One library entry: contract metadata plus where the bytes live. */
data class LibraryItem(val media: MediaMetadata, val uri: Uri, val published: Boolean) {
    val id: String get() = media.id
    val mime: String get() = if (media.kind == MediaKind.VIDEO) "video/mp4" else if (media.container == "png") "image/png" else "image/jpeg"
}

data class StorageUsage(val totalBytes: Long, val videoCount: Int, val screenshotCount: Int, val freeDiskBytes: Long)

/** Outcome of a delete or rename that may need the user's confirmation (scoped storage). */
sealed interface LibraryActionResult {
    data object Done : LibraryActionResult
    /** Launch with `startIntentSenderForResult`, then call the matching `complete…` on RESULT_OK. */
    data class NeedsConsent(val intentSender: IntentSender) : LibraryActionResult
    data class Failed(val message: String) : LibraryActionResult
}

/** SQLite history (plain SQLiteOpenHelper, no Room/KSP). */
internal class LibraryDb(context: Context) : SQLiteOpenHelper(context, "captureone-library.db", null, 1) {
    override fun onCreate(db: SQLiteDatabase) {
        db.execSQL(
            """CREATE TABLE media (
                id TEXT PRIMARY KEY NOT NULL,
                kind TEXT NOT NULL,
                title TEXT NOT NULL,
                uri TEXT NOT NULL,
                published INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                metadata_json TEXT NOT NULL
            )""",
        )
        db.execSQL("CREATE INDEX media_created ON media(created_at)")
    }

    override fun onUpgrade(db: SQLiteDatabase, oldVersion: Int, newVersion: Int) = Unit
}

/**
 * Recording/screenshot history with rename, delete (MediaStore; `createDeleteRequest` on API 30+
 * when the app no longer owns the item), share and storage usage. Blocking: call off the main thread.
 */
class LibraryRepository(context: Context) {
    private val app = context.applicationContext
    private val db = LibraryDb(app)

    fun add(media: MediaMetadata, uri: Uri, published: Boolean) {
        val v = ContentValues().apply {
            put("id", media.id)
            put("kind", media.kind.name)
            put("title", media.title)
            put("uri", uri.toString())
            put("published", if (published) 1 else 0)
            put("created_at", media.createdAt)
            put("size_bytes", media.sizeBytes)
            put("metadata_json", CaptureJson.encodeToString(MediaMetadata.serializer(), media))
        }
        db.writableDatabase.insertWithOnConflict("media", null, v, SQLiteDatabase.CONFLICT_REPLACE)
    }

    fun list(kind: MediaKind? = null): List<LibraryItem> {
        val out = ArrayList<LibraryItem>()
        val (sel, args) = if (kind != null) "kind = ?" to arrayOf(kind.name) else null to null
        db.readableDatabase.query("media", arrayOf("metadata_json", "uri", "published", "title"), sel, args, null, null, "created_at DESC").use { c ->
            while (c.moveToNext()) {
                val meta = runCatching { CaptureJson.decodeFromString(MediaMetadata.serializer(), c.getString(0)) }.getOrNull() ?: continue
                out += LibraryItem(meta.copy(title = c.getString(3)), Uri.parse(c.getString(1)), c.getInt(2) == 1)
            }
        }
        return out
    }

    fun get(id: String): LibraryItem? = list().firstOrNull { it.id == id }

    /** Removes rows whose file no longer exists (deleted outside the app). Returns the number removed. */
    fun prune(): Int {
        var removed = 0
        for (item in list()) {
            if (!exists(item)) {
                db.writableDatabase.delete("media", "id = ?", arrayOf(item.id))
                removed++
            }
        }
        return removed
    }

    private fun exists(item: LibraryItem): Boolean = if (item.uri.scheme == "file") {
        File(requireNotNull(item.uri.path)).exists()
    } else {
        try {
            app.contentResolver.openFileDescriptor(item.uri, "r")?.use { true } ?: false
        } catch (e: Exception) {
            false
        }
    }

    /**
     * Renames the title, and the shared file's display name when the app owns it. If the OS needs the
     * user's consent (API 30+, app reinstalled), returns [LibraryActionResult.NeedsConsent]; call
     * [rename] again after RESULT_OK.
     */
    fun rename(id: String, newTitle: String): LibraryActionResult {
        val item = get(id) ?: return LibraryActionResult.Failed("Item not found")
        val title = newTitle.trim().ifEmpty { return LibraryActionResult.Failed("Title must not be empty") }
        val ext = item.media.fileName.substringAfterLast('.', "")
        val fileName = FileNaming.sanitize(title) + if (ext.isNotEmpty()) ".$ext" else ""
        var newPath = item.media.path
        var newUri = item.uri
        if (item.published) {
            try {
                val v = ContentValues().apply { put(MediaStore.MediaColumns.DISPLAY_NAME, fileName) }
                app.contentResolver.update(item.uri, v, null, null)
            } catch (e: SecurityException) {
                consentFor(e, item.uri, write = true)?.let { return LibraryActionResult.NeedsConsent(it) }
                return LibraryActionResult.Failed("Not allowed to rename this file: ${e.message}")
            }
        } else {
            val f = File(item.media.path)
            val target = File(f.parentFile, fileName)
            if (target != f && !target.exists() && f.renameTo(target)) {
                newPath = target.absolutePath
                newUri = Uri.fromFile(target)
            }
        }
        val meta = item.media.copy(title = title, fileName = fileName, path = newPath)
        add(meta, newUri, item.published)
        return LibraryActionResult.Done
    }

    /** Deletes the file and the history row; may require user consent for shared files. */
    fun delete(id: String): LibraryActionResult {
        val item = get(id) ?: return LibraryActionResult.Done
        if (item.published) {
            try {
                app.contentResolver.delete(item.uri, null, null)
            } catch (e: SecurityException) {
                consentFor(e, item.uri, write = false)?.let { return LibraryActionResult.NeedsConsent(it) }
                return LibraryActionResult.Failed("Not allowed to delete this file: ${e.message}")
            }
        } else {
            File(item.media.path).delete()
        }
        completeDelete(id)
        return LibraryActionResult.Done
    }

    /** Call after the user confirmed a system delete request (the file is already gone). */
    fun completeDelete(id: String) {
        get(id)?.media?.thumbnailPath?.let { File(it).delete() }
        db.writableDatabase.delete("media", "id = ?", arrayOf(id))
    }

    private fun consentFor(e: SecurityException, uri: Uri, write: Boolean): IntentSender? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val pi: PendingIntent = if (write) {
                MediaStore.createWriteRequest(app.contentResolver, listOf(uri))
            } else {
                MediaStore.createDeleteRequest(app.contentResolver, listOf(uri))
            }
            return pi.intentSender
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && e is RecoverableSecurityException) {
            return e.userAction.actionIntent.intentSender
        }
        return null
    }

    /** ACTION_SEND chooser for a published item; null for files kept in app storage (not shareable). */
    fun shareIntent(item: LibraryItem): Intent? {
        if (!item.published) return null
        val send = Intent(Intent.ACTION_SEND).apply {
            type = item.mime
            putExtra(Intent.EXTRA_STREAM, item.uri)
            putExtra(Intent.EXTRA_TITLE, item.media.title)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        return Intent.createChooser(send, item.media.title).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }

    fun storageUsage(): StorageUsage {
        val items = list()
        val dir = app.getExternalFilesDir(null) ?: app.filesDir
        val free = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) Storage.freeBytes(dir) else Storage.freeBytes(Environment.getExternalStorageDirectory())
        return StorageUsage(
            totalBytes = items.sumOf { it.media.sizeBytes },
            videoCount = items.count { it.media.kind == MediaKind.VIDEO },
            screenshotCount = items.count { it.media.kind == MediaKind.SCREENSHOT },
            freeDiskBytes = free.coerceAtLeast(0),
        )
    }

    /** Loads (and caches as JPEG) a thumbnail, max [maxPx] on the long side. */
    fun thumbnail(item: LibraryItem, maxPx: Int = 384): Bitmap? {
        val cache = File(app.cacheDir, "thumbnails").apply { mkdirs() }
        val cached = File(cache, "${item.id}_$maxPx.jpg")
        if (cached.isFile) BitmapFactory.decodeFile(cached.absolutePath)?.let { return it }
        val bmp = createThumbnail(item, maxPx) ?: return null
        runCatching { FileOutputStream(cached).use { bmp.compress(Bitmap.CompressFormat.JPEG, 85, it) } }
        return bmp
    }

    private fun createThumbnail(item: LibraryItem, maxPx: Int): Bitmap? = try {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q && item.published) {
            app.contentResolver.loadThumbnail(item.uri, android.util.Size(maxPx, maxPx), null)
        } else if (item.media.kind == MediaKind.VIDEO) {
            val mmr = MediaMetadataRetriever()
            try {
                if (item.uri.scheme == "file") mmr.setDataSource(item.uri.path) else mmr.setDataSource(app, item.uri)
                mmr.getFrameAtTime(0)?.let { scale(it, maxPx) }
            } finally {
                mmr.release()
            }
        } else {
            val path = if (item.uri.scheme == "file") item.uri.path else null
            if (path != null) {
                ThumbnailUtils.extractThumbnail(BitmapFactory.decodeFile(path), maxPx, maxPx * 9 / 16)
            } else {
                app.contentResolver.openInputStream(item.uri)?.use { BitmapFactory.decodeStream(it) }?.let { scale(it, maxPx) }
            }
        }
    } catch (e: Exception) {
        null
    }

    private fun scale(src: Bitmap, maxPx: Int): Bitmap {
        val s = maxPx.toFloat() / maxOf(src.width, src.height)
        if (s >= 1f) return src
        return Bitmap.createScaledBitmap(src, (src.width * s).toInt().coerceAtLeast(1), (src.height * s).toInt().coerceAtLeast(1), true)
    }

    companion object {
        @Volatile private var instance: LibraryRepository? = null
        fun get(context: Context): LibraryRepository = instance ?: synchronized(this) {
            instance ?: LibraryRepository(context).also { instance = it }
        }

        /** Where shared files go (for display in the UI). */
        val SHARED_LOCATION = "Movies/${MediaStorePublisher.ALBUM} and Pictures/${MediaStorePublisher.ALBUM}"
    }
}
