package dev.captureone.capture.engine

import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.Image
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.os.Handler
import android.os.SystemClock
import android.view.Surface
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.Size
import java.io.File
import java.io.FileOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * Screenshots with ImageReader (RGBA_8888) on a VirtualDisplay. Handles row padding when copying
 * the plane into a Bitmap. Used standalone (own VirtualDisplay) or during a recording by briefly
 * pointing the recording's VirtualDisplay at the ImageReader (Android 14+ forbids a second
 * createVirtualDisplay on the same MediaProjection).
 */
object Screenshotter {
    /** Converts an RGBA_8888 [Image] to a tightly sized ARGB_8888 bitmap. */
    fun imageToBitmap(image: Image): Bitmap {
        val plane = image.planes[0]
        val pixelStride = plane.pixelStride
        val rowStride = plane.rowStride
        val width = image.width
        val height = image.height
        val rowPaddingPx = (rowStride - pixelStride * width) / pixelStride
        val padded = Bitmap.createBitmap(width + rowPaddingPx, height, Bitmap.Config.ARGB_8888)
        val buffer = plane.buffer
        buffer.rewind()
        padded.copyPixelsFromBuffer(buffer)
        if (rowPaddingPx == 0) return padded
        val cropped = Bitmap.createBitmap(padded, 0, 0, width, height)
        padded.recycle()
        return cropped
    }

    /**
     * Waits for a frame on [reader] that arrived at least [settleMs] after the call (the first frames
     * can still show the consent dialog fading out) and converts it.
     */
    fun awaitFrame(reader: ImageReader, handler: Handler, settleMs: Long, timeoutMs: Long = 4000): Bitmap {
        val notBefore = SystemClock.uptimeMillis() + settleMs
        val result = AtomicReference<Bitmap?>()
        val error = AtomicReference<Throwable?>()
        val latch = CountDownLatch(1)
        reader.setOnImageAvailableListener({ r ->
            val img = try { r.acquireLatestImage() } catch (e: IllegalStateException) { null } ?: return@setOnImageAvailableListener
            try {
                if (latch.count > 0 && SystemClock.uptimeMillis() >= notBefore) {
                    result.set(imageToBitmap(img))
                    latch.countDown()
                }
            } catch (t: Throwable) {
                error.set(t)
                latch.countDown()
            } finally {
                img.close()
            }
        }, handler)
        // A static screen may not produce new frames after the settle time: poll for the latest one too.
        val deadline = SystemClock.uptimeMillis() + timeoutMs
        var lastPoll = 0L
        while (!latch.await(50, TimeUnit.MILLISECONDS)) {
            val now = SystemClock.uptimeMillis()
            if (now >= deadline) break
            if (now >= notBefore + 300 && now - lastPoll > 300) {
                lastPoll = now
                handler.post {
                    val img = try { reader.acquireLatestImage() } catch (e: IllegalStateException) { null }
                    if (img != null) {
                        try {
                            if (latch.count > 0) { result.set(imageToBitmap(img)); latch.countDown() }
                        } finally { img.close() }
                    }
                }
            }
        }
        reader.setOnImageAvailableListener(null, null)
        error.get()?.let { throw CaptureException(CaptureErrorCode.INTERNAL, "Screenshot conversion failed: ${it.message}", cause = it) }
        return result.get() ?: throw CaptureException(
            CaptureErrorCode.INTERRUPTED,
            "No frame was delivered by the display within ${timeoutMs / 1000} s (screen off or capture blocked).",
            recoverable = true,
        )
    }

    /** Standalone capture with its own VirtualDisplay at full display resolution. */
    fun capture(projection: MediaProjection, display: DisplayInfo, handler: Handler, settleMs: Long): Bitmap {
        val reader = ImageReader.newInstance(display.size.width, display.size.height, PixelFormat.RGBA_8888, 2)
        var vd: VirtualDisplay? = null
        try {
            vd = projection.createVirtualDisplay(
                "CaptureOne-screenshot", display.size.width, display.size.height, display.densityDpi,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, reader.surface, null, handler,
            ) ?: throw CaptureException(CaptureErrorCode.INTERNAL, "createVirtualDisplay returned null")
            return awaitFrame(reader, handler, settleMs)
        } finally {
            vd?.release()
            reader.close()
        }
    }

    /**
     * Grabs a frame from an existing recording VirtualDisplay by swapping its surface for a moment.
     * The recording shows a short freeze (the encoder repeats the previous frame).
     */
    fun captureFrom(vd: VirtualDisplay, size: Size, restore: Surface, handler: Handler): Bitmap {
        val reader = ImageReader.newInstance(size.width, size.height, PixelFormat.RGBA_8888, 2)
        try {
            vd.surface = reader.surface
            return awaitFrame(reader, handler, settleMs = 0, timeoutMs = 2000)
        } finally {
            vd.surface = restore
            reader.close()
        }
    }

    /** Encodes and writes [bitmap]; returns the file. */
    fun write(bitmap: Bitmap, file: File, format: ImageFormat, jpegQuality: Int) {
        val ok = FileOutputStream(file).use { out ->
            when (format) {
                ImageFormat.PNG -> bitmap.compress(Bitmap.CompressFormat.PNG, 100, out)
                ImageFormat.JPEG -> bitmap.compress(Bitmap.CompressFormat.JPEG, jpegQuality.coerceIn(1, 100), out)
            }
        }
        if (!ok) {
            file.delete()
            throw CaptureException(CaptureErrorCode.IO_ERROR, "Could not encode the screenshot as ${format.name}")
        }
    }
}
