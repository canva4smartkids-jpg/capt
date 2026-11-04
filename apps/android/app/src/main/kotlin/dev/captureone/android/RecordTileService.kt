package dev.captureone.android

import android.annotation.SuppressLint
import android.app.PendingIntent
import android.content.Intent
import android.os.Build
import android.service.quicksettings.Tile
import android.service.quicksettings.TileService
import dev.captureone.capture.service.CaptureController

/**
 * Quick Settings tile: starts a recording (opens CaptureOne, which shows the system consent dialog)
 * or stops the current one. The consent dialog cannot be skipped — Android requires it per session.
 */
class RecordTileService : TileService() {
    override fun onStartListening() {
        super.onStartListening()
        val tile = qsTile ?: return
        val busy = CaptureController.isBusy
        tile.state = if (busy) Tile.STATE_ACTIVE else Tile.STATE_INACTIVE
        tile.label = if (busy) "Stop recording" else "Record screen"
        tile.updateTile()
    }

    @SuppressLint("StartActivityAndCollapseDeprecated")
    override fun onClick() {
        super.onClick()
        if (CaptureController.isBusy) {
            CaptureController.stop(this)
            return
        }
        val intent = Intent(this, MainActivity::class.java)
            .setAction(MainActivity.ACTION_START_RECORDING)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(PendingIntent.getActivity(this, 0, intent, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}
