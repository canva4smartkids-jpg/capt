package dev.captureone.tv

import android.app.Activity
import android.content.ActivityNotFoundException
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.Image
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.annotation.OptIn
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.ui.PlayerView
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.service.CaptureController
import dev.captureone.tv.logic.TvHomeModel
import dev.captureone.tv.logic.TvText
import dev.captureone.tv.tvkit.TvBackground
import dev.captureone.tv.tvkit.TvTheme
import dev.captureone.tv.ui.CompanionConnectScreen
import dev.captureone.tv.ui.HomeScreen
import dev.captureone.tv.ui.HubScreen
import dev.captureone.tv.ui.MediaDetailScreen
import dev.captureone.tv.ui.MediaListScreen
import dev.captureone.tv.ui.RecordScreen
import dev.captureone.tv.ui.TvRecordActions
import dev.captureone.tv.ui.TvRecordModel
import dev.captureone.tv.ui.TvRow
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Android TV / Google TV entry point (LEANBACK_LAUNCHER). */
class TvActivity : ComponentActivity() {
    private val vm: TvViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            TvTheme { TvBackground { TvApp(vm) } }
        }
    }
}

private sealed interface Route {
    data object Home : Route
    data object Record : Route
    data object Library : Route
    data class LocalMedia(val id: String) : Route
    data object Connect : Route
    data object Hub : Route
    data class HubMedia(val id: String) : Route
}

private enum class Pending { RECORD, SCREENSHOT }

@Composable
private fun TvApp(vm: TvViewModel) {
    val context = LocalContext.current
    val stack = remember { mutableStateListOf<Route>(Route.Home) }
    val route = stack.last()
    fun push(r: Route) { stack.add(r) }
    fun pop() { if (stack.size > 1) stack.removeAt(stack.lastIndex) }
    BackHandler(enabled = stack.size > 1) { pop() }

    val assessment by vm.assessment.collectAsStateWithLifecycle()
    val settings by vm.settings.collectAsStateWithLifecycle()
    val status by CaptureController.status.collectAsStateWithLifecycle()
    val message by vm.message.collectAsStateWithLifecycle()
    val items by vm.items.collectAsStateWithLifecycle()
    val pad by vm.pad.collectAsStateWithLifecycle()
    val savedHub by vm.savedHub.collectAsStateWithLifecycle()
    val busy by vm.companionBusy.collectAsStateWithLifecycle()
    val companionError by vm.companionError.collectAsStateWithLifecycle()
    val hub by vm.hub.collectAsStateWithLifecycle()
    val hubStatus by vm.hubStatus.collectAsStateWithLifecycle()
    val hubConnected by vm.hubConnected.collectAsStateWithLifecycle()
    val hubRecordings by vm.hubRecordings.collectAsStateWithLifecycle()
    val lastCommand by vm.lastCommandResult.collectAsStateWithLifecycle()

    val pending = remember { mutableStateListOf<Pending>() }
    val consent = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { r ->
        val what = pending.removeLastOrNull()
        val data = r.data
        if (r.resultCode != Activity.RESULT_OK || data == null) {
            vm.showMessage("Screen capture was not allowed.")
            return@rememberLauncherForActivityResult
        }
        when (what) {
            Pending.RECORD -> CaptureController.startRecording(context, r.resultCode, data, vm.recordingConfig())
            Pending.SCREENSHOT -> CaptureController.takeScreenshot(context, r.resultCode, data, settings.screenshotFormat, settings.jpegQuality, settings.fileNameTemplate)
            null -> Unit
        }
    }
    fun launchConsent(p: Pending) {
        vm.clearMessage()
        pending.clear()
        pending.add(p)
        try {
            consent.launch(DeviceFactsCollector.screenCaptureIntent(context))
        } catch (e: ActivityNotFoundException) {
            pending.clear()
            vm.showMessage("This TV has no screen-capture consent screen. Use companion mode or HDMI capture.")
        }
    }

    when (val r = route) {
        Route.Home -> {
            val a = assessment
            if (a == null) {
                MediaListScreen("CaptureOne for TV", "Checking what this device can do…", emptyList(), "", {})
            } else {
                HomeScreen(
                    model = TvHomeModel.from(a),
                    onRecord = { push(Route.Record) },
                    onScreenshot = { launchConsent(Pending.SCREENSHOT) },
                    onLibrary = { push(Route.Library) },
                    onCompanion = { push(if (hub != null) Route.Hub else Route.Connect) },
                )
            }
        }
        Route.Record -> RecordScreen(
            model = TvRecordModel(
                qualityLabel = settings.video.quality.name.lowercase().replaceFirstChar { it.uppercase() },
                fps = settings.video.fps,
                countdownSec = settings.countdownSec,
                systemAudioAvailable = vm.systemAudioAvailable(),
                systemAudioOn = settings.audio.system.enabled && vm.systemAudioAvailable(),
                systemAudioNote = if (vm.systemAudioAvailable()) "Apps that allow playback capture only" else "Needs Android 10+",
                maxDurationMin = settings.maxDurationSec?.let { it / 60 },
                status = status,
                message = message,
            ),
            actions = TvRecordActions(
                onCycleQuality = vm::cycleQuality,
                onCycleFps = vm::cycleFps,
                onCycleCountdown = vm::cycleCountdown,
                onToggleSystemAudio = vm::setSystemAudio,
                onCycleMaxDuration = vm::cycleMaxDuration,
                onStart = { launchConsent(Pending.RECORD) },
                onPause = { CaptureController.pause(context) },
                onResume = { CaptureController.resume(context) },
                onStop = { CaptureController.stop(context) },
                onCancel = { CaptureController.cancel(context) },
            ),
        )
        Route.Library -> MediaListScreen(
            title = "Library on this TV",
            subtitle = message,
            rows = items.map { TvRow(it.id, it.media.title, TvText.mediaLine(it.media)) },
            emptyText = "No recordings or screenshots on this TV yet.",
            onOpen = { push(Route.LocalMedia(it)) },
        )
        is Route.LocalMedia -> {
            val item = vm.item(r.id)
            if (item == null) {
                LaunchedEffect(r) { pop() }
            } else {
                MediaDetailScreen(
                    title = item.media.title,
                    subtitle = TvText.mediaLine(item.media),
                    player = { m -> if (item.media.kind == MediaKind.VIDEO) TvVideoPlayer(item.uri.toString(), emptyMap(), m) else TvImage(item.uri, m) },
                    actions = listOf(
                        "Back" to { pop() },
                        "Delete" to {
                            vm.delete(item.id)
                            pop()
                        },
                    ),
                )
            }
        }
        Route.Connect -> CompanionConnectScreen(
            pad = pad,
            savedHub = savedHub,
            busy = busy,
            error = companionError,
            onKey = vm::padKey,
            onSelectField = vm::selectField,
            onConnect = { vm.pairAndConnect { stack.removeAt(stack.lastIndex); push(Route.Hub) } },
            onReconnectSaved = { vm.reconnectSaved { stack.removeAt(stack.lastIndex); push(Route.Hub) } },
        )
        Route.Hub -> {
            val h = hub
            if (h == null) {
                CompanionConnectScreen(pad, savedHub, busy, companionError, vm::padKey, vm::selectField, { vm.pairAndConnect { } }, { vm.reconnectSaved { } })
            } else {
                HubScreen(
                    hubName = h.name,
                    statusText = TvText.hubStatus(hubStatus, hubConnected) + (companionError?.let { " · $it" } ?: ""),
                    status = hubStatus,
                    rows = hubRecordings.map { TvRow(it.id, it.title, TvText.mediaLine(it)) },
                    lastResult = lastCommand,
                    onCommand = vm::command,
                    onOpen = { push(Route.HubMedia(it)) },
                    onRefresh = vm::refreshHubRecordings,
                    onForget = {
                        vm.forgetHub()
                        pop()
                    },
                )
            }
        }
        is Route.HubMedia -> {
            val m = hubRecordings.firstOrNull { it.id == r.id }
            val c = vm.client
            if (m == null || c == null) {
                LaunchedEffect(r) { pop() }
            } else {
                MediaDetailScreen(
                    title = m.title,
                    subtitle = TvText.mediaLine(m) + " · streamed from ${c.address}",
                    player = { mod ->
                        if (m.kind == MediaKind.VIDEO) TvVideoPlayer(c.recordingFileUrl(m.id), c.authHeaders(), mod) else TvRemoteImage(c.recordingFileUrl(m.id), c.authHeaders(), mod)
                    },
                    actions = listOf("Back" to { pop() }),
                )
            }
        }
    }
}

/** media3 player; [headers] carries the hub's bearer token for streamed recordings. */
@OptIn(UnstableApi::class) // DefaultHttpDataSource / DefaultMediaSourceFactory configuration
@Composable
private fun TvVideoPlayer(url: String, headers: Map<String, String>, modifier: Modifier) {
    val context = LocalContext.current
    val player = remember(url) {
        val http = DefaultHttpDataSource.Factory().setDefaultRequestProperties(headers).setAllowCrossProtocolRedirects(false)
        ExoPlayer.Builder(context)
            .setMediaSourceFactory(DefaultMediaSourceFactory(context).setDataSourceFactory(androidx.media3.datasource.DefaultDataSource.Factory(context, http)))
            .build().apply {
                setMediaItem(MediaItem.fromUri(url))
                playWhenReady = true
                prepare()
            }
    }
    DisposableEffect(player) { onDispose { player.release() } }
    AndroidView(factory = { PlayerView(it).apply { this.player = player; useController = true } }, modifier = modifier)
}

@Composable
private fun TvImage(uri: Uri, modifier: Modifier) {
    val context = LocalContext.current
    val bmp by produceState<Bitmap?>(null, uri) {
        value = withContext(Dispatchers.IO) {
            runCatching { context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, BitmapFactory.Options().apply { inSampleSize = 2 }) } }.getOrNull()
        }
    }
    bmp?.let { Image(it.asImageBitmap(), contentDescription = "Screenshot", contentScale = ContentScale.Fit, modifier = modifier) }
}

@Composable
private fun TvRemoteImage(url: String, headers: Map<String, String>, modifier: Modifier) {
    val bmp by produceState<Bitmap?>(null, url) {
        value = withContext(Dispatchers.IO) {
            runCatching {
                val req = okhttp3.Request.Builder().url(url).apply { headers.forEach { (k, v) -> header(k, v) } }.build()
                dev.captureone.capture.companion.CompanionClient.defaultHttpClient().newCall(req).execute().use { res ->
                    if (!res.isSuccessful) null else res.body?.byteStream()?.use { BitmapFactory.decodeStream(it) }
                }
            }.getOrNull()
        }
    }
    bmp?.let { Image(it.asImageBitmap(), contentDescription = "Screenshot on hub", contentScale = ContentScale.Fit, modifier = modifier) }
}
