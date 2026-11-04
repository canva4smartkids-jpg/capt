package dev.captureone.tv.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.unit.dp
import dev.captureone.capture.capability.NativeCaptureTier
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.RemoteCommand
import dev.captureone.capture.util.Format
import dev.captureone.tv.logic.NumericPad
import dev.captureone.tv.logic.PadField
import dev.captureone.tv.logic.PadKey
import dev.captureone.tv.logic.PadState
import dev.captureone.tv.logic.TvHomeModel
import dev.captureone.tv.tvkit.TvButton
import dev.captureone.tv.tvkit.TvKey
import dev.captureone.tv.tvkit.TvListRow
import dev.captureone.tv.tvkit.TvPanel
import dev.captureone.tv.tvkit.TvText
import dev.captureone.tv.tvkit.TvTextStyle
import dev.captureone.tv.tvkit.TvToggle

/*
 * Compose for TV screens. Fully D-pad navigable: every interactive element is a focusable TV
 * component, each screen requests initial focus, and Back is handled by the activity's nav stack.
 */

const val DRM_NOTICE = "Protected content (DRM/HDCP video, FLAG_SECURE apps) is recorded as black. This is enforced by Android and cannot be bypassed."

/** TV overscan-safe screen frame (48 dp horizontal / 27 dp vertical). */
@Composable
fun TvFrame(title: String, subtitle: String? = null, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize().padding(horizontal = 48.dp, vertical = 27.dp)) {
        TvText(title, style = TvTextStyle.HEADLINE)
        subtitle?.let { TvText(it, muted = true) }
        Spacer(Modifier.height(16.dp))
        content()
    }
}

/** Requests focus on first composition (guarded: the node may not be attached yet). */
@Composable
fun rememberInitialFocus(key: Any? = Unit): FocusRequester {
    val r = remember { FocusRequester() }
    LaunchedEffect(key) { runCatching { r.requestFocus() } }
    return r
}

@Composable
fun HomeScreen(
    model: TvHomeModel,
    onRecord: () -> Unit,
    onScreenshot: () -> Unit,
    onLibrary: () -> Unit,
    onCompanion: () -> Unit,
) {
    val first = rememberInitialFocus()
    TvFrame("CaptureOne for TV", model.headline) {
        Row(horizontalArrangement = Arrangement.spacedBy(32.dp), modifier = Modifier.fillMaxSize()) {
            Column(Modifier.width(360.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (model.canRecord) {
                    TvButton("Record this TV", onRecord, Modifier.fillMaxWidth().focusRequester(first), primary = true)
                    TvButton("Take a screenshot", onScreenshot, Modifier.fillMaxWidth())
                    TvButton("Companion mode (desktop hub)", onCompanion, Modifier.fillMaxWidth())
                } else {
                    TvButton("Companion mode (desktop hub)", onCompanion, Modifier.fillMaxWidth().focusRequester(first), primary = true)
                }
                TvButton("Library on this TV", onLibrary, Modifier.fillMaxWidth())
            }
            TvPanel(Modifier.weight(1f).fillMaxHeight()) {
                val tierText = when (model.tier) {
                    NativeCaptureTier.SUPPORTED -> "This device exposes Android's screen-capture (MediaProjection) consent and internal audio capture."
                    NativeCaptureTier.PARTIAL -> "Screen capture works with limitations on this device."
                    NativeCaptureTier.NOT_AVAILABLE -> "Use companion mode to control a CaptureOne desktop hub, or record this TV's HDMI output with a capture card connected to the desktop app."
                }
                TvText(tierText, style = TvTextStyle.TITLE)
                Spacer(Modifier.height(12.dp))
                Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (r in model.reasons) TvText("• $r", muted = true)
                    if (model.reasons.none { it.contains("FLAG_SECURE") || it.contains("DRM") }) TvText("• $DRM_NOTICE", muted = true)
                }
            }
        }
    }
}

/** Native recording on the TV (same :capture engine as the phone app). */
class TvRecordModel(
    val qualityLabel: String,
    val fps: Int,
    val countdownSec: Int,
    val systemAudioAvailable: Boolean,
    val systemAudioOn: Boolean,
    val systemAudioNote: String,
    val maxDurationMin: Int?,
    val status: RecordingStatus?,
    val message: String?,
)

class TvRecordActions(
    val onCycleQuality: () -> Unit,
    val onCycleFps: () -> Unit,
    val onCycleCountdown: () -> Unit,
    val onToggleSystemAudio: (Boolean) -> Unit,
    val onCycleMaxDuration: () -> Unit,
    val onStart: () -> Unit,
    val onPause: () -> Unit,
    val onResume: () -> Unit,
    val onStop: () -> Unit,
    val onCancel: () -> Unit,
)

@Composable
fun RecordScreen(model: TvRecordModel, actions: TvRecordActions) {
    val st = model.status?.state
    val busy = st == RecordingState.COUNTDOWN || st == RecordingState.STARTING || st == RecordingState.RECORDING ||
        st == RecordingState.PAUSED || st == RecordingState.STOPPING || st == RecordingState.FINALIZING
    val first = rememberInitialFocus(busy)
    TvFrame("Record this TV", "Android asks for consent each time. Return to CaptureOne to stop – many TVs have no notification shade.") {
        Row(horizontalArrangement = Arrangement.spacedBy(32.dp)) {
            Column(Modifier.width(420.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                if (busy) {
                    val s = model.status
                    TvText(
                        when (st) {
                            RecordingState.COUNTDOWN -> "Starting in ${s?.countdownRemainingSec ?: 0}…"
                            RecordingState.PAUSED -> "Paused · ${Format.duration(s?.elapsedMs ?: 0.0)}"
                            RecordingState.RECORDING -> "Recording · ${Format.duration(s?.elapsedMs ?: 0.0)}"
                            else -> "Saving…"
                        },
                        style = TvTextStyle.DISPLAY,
                    )
                    s?.let { TvText("${Format.bytes(it.bytesWritten)} · ${it.encoder ?: ""}", muted = true) }
                    when (st) {
                        RecordingState.RECORDING -> {
                            TvButton("Pause", actions.onPause, Modifier.fillMaxWidth().focusRequester(first))
                            TvButton("Stop and save", actions.onStop, Modifier.fillMaxWidth(), primary = true)
                        }
                        RecordingState.PAUSED -> {
                            TvButton("Resume", actions.onResume, Modifier.fillMaxWidth().focusRequester(first), primary = true)
                            TvButton("Stop and save", actions.onStop, Modifier.fillMaxWidth())
                        }
                        RecordingState.COUNTDOWN -> TvButton("Cancel", actions.onCancel, Modifier.fillMaxWidth().focusRequester(first))
                        else -> Unit
                    }
                } else {
                    TvToggle("Quality", false, { actions.onCycleQuality() }, Modifier.focusRequester(first), note = model.qualityLabel)
                    TvToggle("Frame rate", false, { actions.onCycleFps() }, note = "${model.fps} fps")
                    TvToggle("Countdown", false, { actions.onCycleCountdown() }, note = if (model.countdownSec == 0) "Off" else "${model.countdownSec} s")
                    TvToggle("Stop automatically", false, { actions.onCycleMaxDuration() }, note = model.maxDurationMin?.let { "after $it min" } ?: "Never")
                    TvToggle("Internal audio", model.systemAudioOn, actions.onToggleSystemAudio, enabled = model.systemAudioAvailable, note = model.systemAudioNote)
                    Spacer(Modifier.height(8.dp))
                    TvButton("Start recording", actions.onStart, Modifier.fillMaxWidth(), primary = true)
                }
            }
            TvPanel(Modifier.weight(1f)) {
                model.message?.let { TvText(it, isError = true) }
                model.status?.warnings?.takeLast(3)?.forEach { TvText(it, muted = true) }
                TvText(DRM_NOTICE, muted = true)
            }
        }
    }
}

class TvRow(val id: String, val title: String, val subtitle: String)

@Composable
fun MediaListScreen(title: String, subtitle: String?, rows: List<TvRow>, emptyText: String, onOpen: (String) -> Unit, header: @Composable () -> Unit = {}) {
    val first = rememberInitialFocus(rows.firstOrNull()?.id)
    TvFrame(title, subtitle) {
        header()
        if (rows.isEmpty()) {
            TvText(emptyText, muted = true)
        } else {
            LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp), modifier = Modifier.fillMaxWidth()) {
                items(rows, key = { it.id }) { r ->
                    TvListRow(r.title, r.subtitle, { onOpen(r.id) }, if (r == rows.first()) Modifier.focusRequester(first) else Modifier)
                }
            }
        }
    }
}

/** Player/viewer page; [player] is supplied by the Android layer (media3 / image). */
@Composable
fun MediaDetailScreen(title: String, subtitle: String, player: @Composable (Modifier) -> Unit, actions: List<Pair<String, () -> Unit>>) {
    val first = rememberInitialFocus()
    Row(Modifier.fillMaxSize().padding(horizontal = 48.dp, vertical = 27.dp), horizontalArrangement = Arrangement.spacedBy(24.dp)) {
        Box(Modifier.weight(1f).fillMaxHeight(), contentAlignment = Alignment.Center) { player(Modifier.fillMaxSize()) }
        Column(Modifier.width(300.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            TvText(title, style = TvTextStyle.TITLE)
            TvText(subtitle, muted = true)
            for ((i, a) in actions.withIndex()) {
                TvButton(a.first, a.second, if (i == 0) Modifier.fillMaxWidth().focusRequester(first) else Modifier.fillMaxWidth())
            }
        }
    }
}

/** Hub address + pairing code entry with an on-screen numeric pad (no keyboard needed). */
@Composable
fun CompanionConnectScreen(
    pad: PadState,
    savedHub: String?,
    busy: Boolean,
    error: String?,
    onKey: (PadKey) -> Unit,
    onSelectField: (PadField) -> Unit,
    onConnect: () -> Unit,
    onReconnectSaved: () -> Unit,
) {
    val first = rememberInitialFocus()
    TvFrame("Companion mode", "Pair with the CaptureOne desktop app: enable the hub there and enter its address and the 6-digit code it shows.") {
        Row(horizontalArrangement = Arrangement.spacedBy(40.dp)) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                for ((r, row) in NumericPad.LAYOUT.withIndex()) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for ((c, key) in row.withIndex()) {
                            TvKey(key.label, { onKey(key) }, if (r == 0 && c == 0) Modifier.focusRequester(first) else Modifier)
                        }
                    }
                }
            }
            Column(Modifier.width(460.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                TvListRow(
                    "Hub address" + if (pad.field == PadField.ADDRESS) "  ◀ typing" else "",
                    pad.address.ifEmpty { "e.g. 192.168.1.20 (port 47800 by default)" },
                    { onSelectField(PadField.ADDRESS) },
                    selected = pad.field == PadField.ADDRESS,
                )
                pad.addressError?.let { TvText(it, isError = true) }
                TvListRow(
                    "Pairing code" + if (pad.field == PadField.CODE) "  ◀ typing" else "",
                    if (pad.code.isEmpty()) "6 digits" else pad.code.padEnd(6, '•'),
                    { onSelectField(PadField.CODE) },
                    selected = pad.field == PadField.CODE,
                )
                TvButton(if (busy) "Connecting…" else "Pair and connect", onConnect, Modifier.fillMaxWidth(), enabled = pad.canConnect && !busy, primary = true)
                if (savedHub != null) TvButton("Reconnect to $savedHub", onReconnectSaved, Modifier.fillMaxWidth(), enabled = !busy)
                error?.let { TvText(it, isError = true) }
                TvText("The hub only accepts paired devices. Traffic stays on your local network (HTTP, token-authenticated).", muted = true)
            }
        }
    }
}

@Composable
fun HubScreen(
    hubName: String,
    statusText: String,
    status: RecordingStatus?,
    rows: List<TvRow>,
    lastResult: String?,
    onCommand: (RemoteCommand) -> Unit,
    onOpen: (String) -> Unit,
    onRefresh: () -> Unit,
    onForget: () -> Unit,
) {
    val first = rememberInitialFocus()
    val st = status?.state
    TvFrame("Hub · $hubName", statusText) {
        Row(horizontalArrangement = Arrangement.spacedBy(32.dp)) {
            Column(Modifier.width(320.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                val recording = st == RecordingState.RECORDING
                val paused = st == RecordingState.PAUSED
                TvButton(
                    if (recording || paused) "Stop recording" else "Start recording",
                    { onCommand(if (recording || paused) RemoteCommand.StopRecording else RemoteCommand.StartRecording()) },
                    Modifier.fillMaxWidth().focusRequester(first), primary = true,
                )
                TvButton(if (paused) "Resume" else "Pause", { onCommand(if (paused) RemoteCommand.ResumeRecording else RemoteCommand.PauseRecording) }, Modifier.fillMaxWidth(), enabled = recording || paused)
                TvButton("Screenshot on hub", { onCommand(RemoteCommand.Screenshot) }, Modifier.fillMaxWidth())
                TvButton("Record HDMI capture device", { onCommand(RemoteCommand.StartRecording(dev.captureone.capture.contract.StartPreset.CAPTURE_DEVICE)) }, Modifier.fillMaxWidth(), enabled = !(recording || paused))
                TvButton("Refresh recordings", onRefresh, Modifier.fillMaxWidth())
                TvButton("Forget this hub", onForget, Modifier.fillMaxWidth())
                lastResult?.let { TvText(it, muted = true) }
            }
            Column(Modifier.weight(1f)) {
                TvText("Recordings on the hub", style = TvTextStyle.TITLE)
                Spacer(Modifier.height(8.dp))
                if (rows.isEmpty()) {
                    TvText("No recordings yet.", muted = true)
                } else {
                    LazyColumn(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        items(rows, key = { it.id }) { r -> TvListRow(r.title, r.subtitle, { onOpen(r.id) }) }
                    }
                }
            }
        }
    }
}
