package dev.captureone.android

import android.Manifest
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.ActivityResult
import androidx.activity.result.IntentSenderRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.material3.dynamicLightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.captureone.android.logic.ControlsModel
import dev.captureone.android.logic.LibraryRowModel
import dev.captureone.android.logic.PermissionCardModel
import dev.captureone.android.logic.PermissionCards
import dev.captureone.android.logic.RecordOptions
import dev.captureone.android.logic.StorageUsageModel
import dev.captureone.android.ui.AppScaffold
import dev.captureone.android.ui.CaptureOneTheme
import dev.captureone.android.ui.Destination
import dev.captureone.android.ui.LibraryActions
import dev.captureone.android.ui.LibraryScreen
import dev.captureone.android.ui.RecordActions
import dev.captureone.android.ui.RecordScreen
import dev.captureone.android.ui.RecordScreenModel
import dev.captureone.android.ui.SettingsScreen
import dev.captureone.capture.capability.CapabilityClassifier
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.capability.PermissionInspector
import dev.captureone.capture.contract.PermissionKind
import dev.captureone.capture.contract.PermissionState
import dev.captureone.capture.library.LibraryRepository
import dev.captureone.capture.service.CaptureController
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val vm: AppViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val startFromTile = savedInstanceState == null && intent?.action == ACTION_START_RECORDING
        setContent {
            val ctx = LocalContext.current
            val dark = isSystemInDarkTheme()
            val scheme = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                if (dark) dynamicDarkColorScheme(ctx) else dynamicLightColorScheme(ctx)
            } else null
            CaptureOneTheme(scheme) { AppRoot(vm, startFromTile) }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (intent.action == ACTION_START_RECORDING) pendingTileStart = true
    }

    companion object {
        const val ACTION_START_RECORDING = "dev.captureone.android.START_RECORDING"

        /** Set when the Quick Settings tile re-launches an existing activity. */
        @Volatile var pendingTileStart = false
    }
}

private enum class PendingCapture { RECORD, SCREENSHOT }

@Composable
private fun AppRoot(vm: AppViewModel, startFromTile: Boolean) {
    val context = LocalContext.current
    val activity = context as Activity
    val scope = rememberCoroutineScope()

    val settings by vm.settings.collectAsStateWithLifecycle()
    val report by vm.report.collectAsStateWithLifecycle()
    val consentAvailable by vm.consentAvailable.collectAsStateWithLifecycle()
    val permissions by vm.permissions.collectAsStateWithLifecycle()
    val status by CaptureController.status.collectAsStateWithLifecycle()
    val lastMedia by CaptureController.lastMedia.collectAsStateWithLifecycle()
    val message by vm.message.collectAsStateWithLifecycle()
    val items by vm.items.collectAsStateWithLifecycle()
    val usage by vm.usage.collectAsStateWithLifecycle()

    var destination by rememberSaveable { mutableStateOf(Destination.RECORD) }
    var selectedId by rememberSaveable { mutableStateOf<String?>(null) }
    var pending by rememberSaveable { mutableStateOf<PendingCapture?>(null) }
    var pendingDeleteId by rememberSaveable { mutableStateOf<String?>(null) }
    var pendingRename by remember { mutableStateOf<Pair<String, String>?>(null) }

    LifecycleResumeEffect(Unit) {
        vm.refreshPermissions(activity)
        onPauseOrDispose { }
    }

    // System consent dialog (MediaProjection). Each result is handed to the service exactly once.
    val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r: ActivityResult ->
        val data = r.data
        val what = pending
        pending = null
        if (r.resultCode != Activity.RESULT_OK || data == null) {
            vm.showMessage("Screen capture was not allowed. Nothing was recorded.", false)
            return@rememberLauncherForActivityResult
        }
        when (what) {
            PendingCapture.RECORD -> CaptureController.startRecording(context, r.resultCode, data, vm.recordingConfig())
            PendingCapture.SCREENSHOT -> CaptureController.takeScreenshot(context, r.resultCode, data, settings.screenshotFormat, settings.jpegQuality, settings.fileNameTemplate)
            null -> Unit
        }
    }

    fun launchConsent(kind: PendingCapture) {
        vm.dismissMessage()
        pending = kind
        try {
            consent.launch(DeviceFactsCollector.screenCaptureIntent(context))
        } catch (e: ActivityNotFoundException) {
            pending = null
            vm.showMessage("This device has no screen-capture consent screen, so recording is not possible here.", true)
        }
    }

    val permissionLauncher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        vm.refreshPermissions(activity)
    }
    val notificationThenRecord = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {
        vm.refreshPermissions(activity)
        launchConsent(PendingCapture.RECORD)
    }

    fun requestPermission(card: PermissionCardModel) {
        if (card.opensSettings) {
            context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)))
            return
        }
        val p = PermissionInspector.requestablePermission(card.kind) ?: return
        vm.markRequested(p)
        permissionLauncher.launch(p)
    }

    fun startAfterAudioCheck() {
        // Ask for notifications once (timer + controls), then go to the system consent dialog either way.
        val notif = permissions.firstOrNull { it.kind == PermissionKind.NOTIFICATIONS }
        val p = PermissionInspector.requestablePermission(PermissionKind.NOTIFICATIONS)
        if (notif?.state == PermissionState.PROMPT && p != null) {
            vm.markRequested(p)
            notificationThenRecord.launch(p)
        } else {
            launchConsent(PendingCapture.RECORD)
        }
    }

    val audioThenRecord = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        vm.refreshPermissions(activity)
        if (granted) {
            startAfterAudioCheck()
        } else {
            vm.showMessage("Audio recording needs the microphone permission. Allow it, or turn microphone and internal audio off.", true)
        }
    }

    fun startRecording() {
        val wantsAudio = settings.audio.microphone.enabled || settings.audio.system.enabled
        if (wantsAudio && context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            vm.markRequested(Manifest.permission.RECORD_AUDIO)
            audioThenRecord.launch(Manifest.permission.RECORD_AUDIO)
        } else {
            startAfterAudioCheck()
        }
    }

    LaunchedEffect(startFromTile) {
        if (startFromTile || MainActivity.pendingTileStart) {
            MainActivity.pendingTileStart = false
            if (!CaptureController.isBusy) startRecording()
        }
    }

    val intentSender = rememberLauncherForActivityResult(ActivityResultContracts.StartIntentSenderForResult()) { r ->
        val del = pendingDeleteId
        val ren = pendingRename
        pendingDeleteId = null
        pendingRename = null
        if (r.resultCode != Activity.RESULT_OK) return@rememberLauncherForActivityResult
        if (del != null) {
            vm.completeDelete(del)
            if (selectedId == del) selectedId = null
        }
        if (ren != null) scope.launch { vm.rename(ren.first, ren.second) }
    }

    val wide = LocalConfiguration.current.screenWidthDp >= 600

    BackHandler(enabled = destination != Destination.RECORD || selectedId != null) {
        if (destination == Destination.LIBRARY && selectedId != null && !wide) selectedId = null else destination = Destination.RECORD
    }

    val options = RecordOptions.options(report, vm.display)
    val micState = permissions.firstOrNull { it.kind == PermissionKind.MICROPHONE }?.state
    val captureAvailable = consentAvailable

    AppScaffold(destination, { destination = it }, wide) { pad ->
        val mod = Modifier.padding(pad)
        when (destination) {
            Destination.RECORD -> RecordScreen(
                model = RecordScreenModel(
                    captureAvailable = captureAvailable,
                    captureUnavailableReason = if (!consentAvailable) "This device has no MediaProjection consent screen; no app can record the screen here." else null,
                    permissions = PermissionCards.forRecordScreen(permissions),
                    settings = settings,
                    options = options,
                    display = vm.display,
                    mic = RecordOptions.mic(report, settings, micState),
                    system = RecordOptions.system(report, settings),
                    estimate = RecordOptions.estimate(settings, vm.display, vm.presets),
                    controls = ControlsModel.from(status),
                    message = message?.first,
                    messageIsError = message?.second == true,
                    lastMedia = lastMedia,
                    notes = listOf(CapabilityClassifier.DRM_NOTE),
                ),
                actions = RecordActions(
                    onRequestPermission = ::requestPermission,
                    onSettingsChange = vm::updateSettings,
                    onStartRecording = ::startRecording,
                    onScreenshot = { launchConsent(PendingCapture.SCREENSHOT) },
                    onPause = { CaptureController.pause(context) },
                    onResume = { CaptureController.resume(context) },
                    onStop = { CaptureController.stop(context) },
                    onCancel = { CaptureController.cancel(context) },
                    onScreenshotDuringRecording = { CaptureController.screenshotDuringRecording(context, settings.screenshotFormat, settings.jpegQuality) },
                    onDismissMessage = vm::dismissMessage,
                    onOpenLibrary = {
                        selectedId = lastMedia?.id
                        destination = Destination.LIBRARY
                    },
                ),
                wide = wide,
                modifier = mod,
            )
            Destination.LIBRARY -> LibraryScreen(
                rows = items.map { LibraryRowModel.from(it.media, it.published) },
                usage = usage?.let { StorageUsageModel.from(it.totalBytes, it.videoCount, it.screenshotCount, it.freeDiskBytes) },
                selectedId = selectedId,
                wide = wide,
                actions = LibraryActions(
                    onSelect = { selectedId = it },
                    onRename = { id, title ->
                        scope.launch {
                            vm.rename(id, title)?.let { sender ->
                                pendingRename = id to title
                                intentSender.launch(IntentSenderRequest.Builder(sender).build())
                            }
                        }
                    },
                    onDelete = { id ->
                        scope.launch {
                            val sender = vm.delete(id)
                            if (sender != null) {
                                pendingDeleteId = id
                                intentSender.launch(IntentSenderRequest.Builder(sender).build())
                            } else if (selectedId == id) {
                                selectedId = null
                            }
                        }
                    },
                    onShare = { id ->
                        val share = vm.shareIntent(id)
                        if (share != null) context.startActivity(share) else vm.showMessage("Files kept in app storage cannot be shared.", false)
                    },
                    onRefresh = vm::refreshLibrary,
                ),
                thumbnail = { id, m -> Thumbnail(vm.item(id), m) },
                preview = { id, m -> MediaPreview(vm.item(id), m) },
                modifier = mod,
            )
            Destination.SETTINGS -> SettingsScreen(
                settings = settings,
                options = options,
                display = vm.display,
                onChange = vm::updateSettings,
                footer = "Recordings are saved to ${LibraryRepository.SHARED_LOCATION}. Encoders: " +
                    (report?.encoders?.joinToString { "${it.name}${if (it.hardware) " (hw)" else ""}${if (it.verified) " ✓" else ""}" } ?: "…"),
                modifier = mod,
            )
        }
    }
}
