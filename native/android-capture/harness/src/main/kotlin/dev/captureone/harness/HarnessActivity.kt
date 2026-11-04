package dev.captureone.harness

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.os.Environment
import android.view.ViewGroup
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.service.CaptureController
import dev.captureone.capture.settings.CaptureSettings
import dev.captureone.capture.util.Format
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/**
 * Minimal, framework-only UI over the real capture engine, for on-device checks of the engine
 * without the Compose apps: consent -> foreground service -> record/stop/screenshot, live status,
 * last validated result, and the device's capability report.
 */
class HarnessActivity : Activity() {
    private val scope = MainScope()
    private lateinit var status: TextView
    private lateinit var output: TextView
    private var pendingScreenshot = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL; setPadding(32, 48, 32, 32) }
        fun button(label: String, onClick: () -> Unit) = Button(this).apply { text = label; setOnClickListener { onClick() } }.also { root.addView(it) }
        status = TextView(this).apply { textSize = 18f; text = "Idle" }
        root.addView(status)
        button("Record (video only)") { consent(screenshot = false, audio = false) }
        button("Record with mic + internal audio") { consent(screenshot = false, audio = true) }
        button("Pause / resume") { if (CaptureController.status.value?.state?.name == "PAUSED") CaptureController.resume(this) else CaptureController.pause(this) }
        button("Stop and save") { CaptureController.stop(this) }
        button("Screenshot (PNG)") { consent(screenshot = true, audio = false) }
        button("Capability report") { showReport() }
        output = TextView(this).apply { textSize = 13f; setTextIsSelectable(true) }
        root.addView(ScrollView(this).apply { addView(output) }, LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f))
        setContentView(root)

        val wanted = buildList {
            add(Manifest.permission.RECORD_AUDIO)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) add(Manifest.permission.POST_NOTIFICATIONS)
        }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }
        if (wanted.isNotEmpty()) requestPermissions(wanted.toTypedArray(), 1)

        scope.launch {
            CaptureController.status.collect { s ->
                status.text = if (s == null) "Idle" else "${s.state.wireName} · ${Format.duration(s.elapsedMs)} · ${Format.bytes(s.bytesWritten)}" +
                    (s.countdownRemainingSec?.let { " · starts in $it" } ?: "") + (s.encoder?.let { " · $it" } ?: "")
            }
        }
        scope.launch { CaptureController.lastMedia.collect { m -> if (m != null) output.text = CaptureJson.encodeToString(dev.captureone.capture.contract.MediaMetadata.serializer(), m) } }
        scope.launch { CaptureController.lastError.collect { e -> if (e != null) output.text = "ERROR ${e.code}: ${e.message}" } }
    }

    private var audioRequested = false

    private fun consent(screenshot: Boolean, audio: Boolean) {
        pendingScreenshot = screenshot
        audioRequested = audio
        CaptureController.clearError()
        try {
            startActivityForResult(DeviceFactsCollector.screenCaptureIntent(this), REQ_CONSENT)
        } catch (e: android.content.ActivityNotFoundException) {
            output.text = "No MediaProjection consent activity on this device: native capture is impossible here."
        }
    }

    @Deprecated("Framework Activity API (no AndroidX in the harness)")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQ_CONSENT) return
        if (resultCode != RESULT_OK || data == null) {
            output.text = "Consent denied."
            return
        }
        if (pendingScreenshot) {
            CaptureController.takeScreenshot(this, resultCode, data, ImageFormat.PNG, 90, "CaptureOne {date} {time}")
            return
        }
        val defaults = CaptureSettings(countdownSec = 3)
        val withAudio = if (audioRequested) {
            defaults.copy(audio = defaults.audio.copy(
                microphone = defaults.audio.microphone.copy(enabled = packageManager.hasSystemFeature(PackageManager.FEATURE_MICROPHONE)),
                system = defaults.audio.system.copy(enabled = Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q),
            ))
        } else defaults
        val dir = File(getExternalFilesDir(Environment.DIRECTORY_MOVIES) ?: filesDir, "recordings-tmp")
        CaptureController.startRecording(this, resultCode, data, withAudio.toRecordingConfig(dir.absolutePath))
    }

    private fun showReport() {
        scope.launch {
            val json = withContext(Dispatchers.Default) {
                CaptureJson.encodeToString(CapabilityReport.serializer(), DeviceFactsCollector.capabilityReport(this@HarnessActivity))
            }
            output.text = json
        }
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private companion object {
        const val REQ_CONSENT = 41
    }
}
