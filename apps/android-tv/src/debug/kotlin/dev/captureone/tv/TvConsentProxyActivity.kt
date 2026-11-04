package dev.captureone.tv

import android.app.Activity
import android.content.Intent
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import java.util.concurrent.CompletableFuture

/** Test-only: requests MediaProjection consent and hands the result to [onResult] while foreground. */
class TvConsentProxyActivity : Activity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) {
            try {
                startActivityForResult(getSystemService(MediaProjectionManager::class.java).createScreenCaptureIntent(), REQ)
            } catch (e: Exception) {
                pending?.complete(RESULT_CANCELED)
                finish()
            }
        }
    }

    @Deprecated("framework Activity API")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQ) {
            if (resultCode == RESULT_OK && data != null) onResult?.invoke(this, resultCode, data)
            pending?.complete(resultCode)
            finish()
        }
    }

    companion object {
        private const val REQ = 7
        @Volatile var onResult: ((Activity, Int, Intent) -> Unit)? = null
        @Volatile var pending: CompletableFuture<Int>? = null
    }
}
