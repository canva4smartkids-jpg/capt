@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package dev.captureone.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import dev.captureone.android.logic.AudioToggleModel
import dev.captureone.android.logic.ControlsModel
import dev.captureone.android.logic.EstimateModel
import dev.captureone.android.logic.OptionsModel
import dev.captureone.android.logic.PermissionCardModel
import dev.captureone.android.logic.RecordOptions
import dev.captureone.capture.contract.AudioTrackMode
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.PermissionKind
import dev.captureone.capture.contract.Size
import dev.captureone.capture.settings.CaptureSettings
import dev.captureone.capture.util.Format

/** Everything the Record screen shows. Built by the Android layer from engine state. */
data class RecordScreenModel(
    val captureAvailable: Boolean,
    val captureUnavailableReason: String?,
    val permissions: List<PermissionCardModel>,
    val settings: CaptureSettings,
    val options: OptionsModel,
    val display: Size?,
    val mic: AudioToggleModel,
    val system: AudioToggleModel,
    val estimate: EstimateModel?,
    val controls: ControlsModel?,
    val message: String?,
    val messageIsError: Boolean,
    val lastMedia: MediaMetadata?,
    val notes: List<String>,
)

class RecordActions(
    val onRequestPermission: (PermissionCardModel) -> Unit,
    val onSettingsChange: (CaptureSettings) -> Unit,
    val onStartRecording: () -> Unit,
    val onScreenshot: () -> Unit,
    val onPause: () -> Unit,
    val onResume: () -> Unit,
    val onStop: () -> Unit,
    val onCancel: () -> Unit,
    val onScreenshotDuringRecording: () -> Unit,
    val onDismissMessage: () -> Unit,
    val onOpenLibrary: () -> Unit,
)

@Composable
fun RecordScreen(model: RecordScreenModel, actions: RecordActions, wide: Boolean, modifier: Modifier = Modifier) {
    val left: @Composable () -> Unit = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            model.controls?.let { RecordingControls(it, actions) }
            StartCard(model, actions)
            model.message?.let {
                Notice(it, isError = model.messageIsError)
                TextButton(onClick = actions.onDismissMessage) { Text("Dismiss") }
            }
            model.lastMedia?.let { LastResultCard(it, actions.onOpenLibrary) }
            PermissionsSection(model.permissions, actions.onRequestPermission)
        }
    }
    val right: @Composable () -> Unit = {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            VideoSection(model, actions.onSettingsChange)
            AudioSection(model, actions.onSettingsChange)
            for (n in model.notes) Notice(n)
        }
    }
    if (wide) {
        Row(
            modifier.fillMaxSize().padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) { left() }
            Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) { right() }
        }
    } else {
        Column(
            modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            left()
            right()
        }
    }
}

@Composable
private fun StartCard(model: RecordScreenModel, actions: RecordActions) {
    val busy = model.controls != null
    SectionCard("Capture") {
        if (!model.captureAvailable) {
            Notice(model.captureUnavailableReason ?: "Screen capture is not available on this device.", isError = true)
        }
        ChoiceChips(
            label = "Countdown",
            options = model.options.countdowns,
            selected = model.settings.countdownSec,
            optionLabel = { if (it == 0) "Off" else "$it s" },
            onSelect = { actions.onSettingsChange(model.settings.copy(countdownSec = it)) },
            enabled = !busy,
        )
        model.estimate?.let {
            Text(it.summary, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Button(
                onClick = actions.onStartRecording,
                enabled = !busy && model.captureAvailable,
                colors = ButtonDefaults.buttonColors(),
            ) {
                RecordDot(12, MaterialTheme.colorScheme.onPrimary)
                Spacer(Modifier.width(8.dp))
                Text("Start recording")
            }
            OutlinedButton(onClick = actions.onScreenshot, enabled = !busy && model.captureAvailable) { Text("Screenshot") }
        }
        Text(
            "Android asks for screen-capture consent each time. The countdown starts after you accept.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun RecordingControls(c: ControlsModel, actions: RecordActions) {
    SectionCard(c.title) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (c.canPause) RecordDot(16)
            Spacer(Modifier.width(10.dp))
            Text(c.countdown?.toString() ?: c.timer, style = MaterialTheme.typography.displayMedium, fontWeight = FontWeight.Light)
        }
        if (c.detail.isNotEmpty()) Text(c.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (c.canPause) FilledTonalButton(onClick = actions.onPause) { Text("Pause") }
            if (c.canResume) FilledTonalButton(onClick = actions.onResume) { Text("Resume") }
            if (c.canStop) {
                Button(onClick = actions.onStop) {
                    StopGlyph()
                    Spacer(Modifier.width(8.dp))
                    Text("Stop")
                }
            }
            if (c.canScreenshot) OutlinedButton(onClick = actions.onScreenshotDuringRecording) { Text("Screenshot") }
            if (c.canCancel) OutlinedButton(onClick = actions.onCancel) { Text("Cancel") }
        }
        for (w in c.warnings.takeLast(3)) Notice(w)
    }
}

@Composable
private fun PermissionsSection(cards: List<PermissionCardModel>, onRequest: (PermissionCardModel) -> Unit) {
    if (cards.isEmpty()) return
    SectionCard("Permissions") {
        for (card in cards) {
            Column(Modifier.fillMaxWidth().padding(vertical = 4.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(card.title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                    AssistChip(onClick = {}, label = { Text(stateLabel(card)) })
                }
                Text(card.explanation, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                card.actionLabel?.let { label ->
                    Spacer(Modifier.height(4.dp))
                    TextButton(onClick = { onRequest(card) }) { Text(label) }
                }
            }
        }
    }
}

private fun stateLabel(card: PermissionCardModel): String = when (card.state) {
    dev.captureone.capture.contract.PermissionState.GRANTED -> "Granted"
    dev.captureone.capture.contract.PermissionState.DENIED -> "Denied"
    dev.captureone.capture.contract.PermissionState.PROMPT -> "Not asked yet"
    dev.captureone.capture.contract.PermissionState.RESTRICTED -> "Unavailable"
    dev.captureone.capture.contract.PermissionState.NOT_APPLICABLE -> "Not needed"
    dev.captureone.capture.contract.PermissionState.PER_SESSION -> if (card.kind == PermissionKind.SCREEN_CAPTURE) "Asked per recording" else "Per session"
}

@Composable
private fun VideoSection(model: RecordScreenModel, onChange: (CaptureSettings) -> Unit) {
    val s = model.settings
    val busy = model.controls != null
    SectionCard("Video") {
        ChoiceChips("Codec", model.options.codecs, s.video.codec, RecordOptions::label, { onChange(s.copy(video = s.video.copy(codec = it))) }, !busy)
        ChoiceChips("Resolution", model.options.resolutions, s.video.resolution, { RecordOptions.label(it, model.display) }, { onChange(s.copy(video = s.video.copy(resolution = it))) }, !busy)
        ChoiceChips("Frame rate", model.options.frameRates, s.video.fps, { "$it fps" }, { onChange(s.copy(video = s.video.copy(fps = it))) }, !busy)
        ChoiceChips("Quality", model.options.qualities, s.video.quality, RecordOptions::label, { onChange(s.copy(video = s.video.copy(quality = it, bitrateKbps = null))) }, !busy)
    }
}

@Composable
private fun AudioSection(model: RecordScreenModel, onChange: (CaptureSettings) -> Unit) {
    val s = model.settings
    val busy = model.controls != null
    SectionCard("Audio") {
        SwitchRow("Microphone", model.mic.note, model.mic.checked, model.mic.available && !busy) {
            onChange(s.copy(audio = s.audio.copy(microphone = s.audio.microphone.copy(enabled = it))))
        }
        SwitchRow("Internal audio", model.system.note, model.system.checked, model.system.available && !busy) {
            onChange(s.copy(audio = s.audio.copy(system = s.audio.system.copy(enabled = it))))
        }
        if (model.mic.checked && model.system.checked) {
            ChoiceChips(
                "Tracks", AudioTrackMode.entries.toList(), s.audio.trackMode,
                { if (it == AudioTrackMode.MIXED) "One mixed track" else "Separate tracks" },
                { onChange(s.copy(audio = s.audio.copy(trackMode = it))) }, !busy,
            )
        }
    }
}

@Composable
private fun LastResultCard(media: MediaMetadata, onOpenLibrary: () -> Unit) {
    val what = if (media.kind == MediaKind.VIDEO) "Recording saved" else "Screenshot saved"
    SectionCard(what) {
        Text(media.fileName, style = MaterialTheme.typography.bodyLarge)
        val facts = buildList {
            media.durationMs?.let { add(Format.duration(it)) }
            add("${media.width}×${media.height}")
            add(Format.bytes(media.sizeBytes))
            media.validation?.let { add(if (it.valid) "validated (${it.tool.substringAfterLast('.')})" else "validation failed") }
        }
        Text(facts.joinToString(" · "), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        TextButton(onClick = onOpenLibrary) { Text("Open library") }
    }
}
