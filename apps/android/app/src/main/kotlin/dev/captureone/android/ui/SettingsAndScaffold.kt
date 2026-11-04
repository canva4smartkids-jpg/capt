@file:OptIn(ExperimentalMaterial3Api::class)

package dev.captureone.android.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import dev.captureone.android.logic.OptionsModel
import dev.captureone.android.logic.RecordOptions
import dev.captureone.capture.contract.AudioTrackMode
import dev.captureone.capture.contract.ImageFormat
import dev.captureone.capture.contract.Size
import dev.captureone.capture.settings.CaptureSettings
import kotlin.math.roundToInt

enum class Destination(val label: String) { RECORD("Record"), LIBRARY("Library"), SETTINGS("Settings") }

private fun Destination.icon(): ImageVector = when (this) {
    Destination.RECORD -> Icons.Filled.Home
    Destination.LIBRARY -> Icons.AutoMirrored.Filled.List
    Destination.SETTINGS -> Icons.Filled.Settings
}

/**
 * Adaptive shell: bottom navigation on compact widths, navigation rail on medium/expanded (tablets,
 * foldables, landscape). [wide] is decided by the caller from the window width (>= 600 dp).
 */
@Composable
fun AppScaffold(
    destination: Destination,
    onNavigate: (Destination) -> Unit,
    wide: Boolean,
    content: @Composable (PaddingValues) -> Unit,
) {
    if (wide) {
        Row(Modifier.fillMaxSize()) {
            NavigationRail {
                for (d in Destination.entries) {
                    NavigationRailItem(
                        selected = d == destination,
                        onClick = { onNavigate(d) },
                        icon = { Icon(d.icon(), contentDescription = null) },
                        label = { Text(d.label) },
                    )
                }
            }
            Scaffold(topBar = { TopAppBar(title = { Text("CaptureOne · ${destination.label}") }) }) { pad -> content(pad) }
        }
    } else {
        Scaffold(
            topBar = { TopAppBar(title = { Text("CaptureOne") }) },
            bottomBar = {
                NavigationBar {
                    for (d in Destination.entries) {
                        NavigationBarItem(
                            selected = d == destination,
                            onClick = { onNavigate(d) },
                            icon = { Icon(d.icon(), contentDescription = null) },
                            label = { Text(d.label) },
                        )
                    }
                }
            },
        ) { pad -> content(pad) }
    }
}

@Composable
fun SettingsScreen(settings: CaptureSettings, options: OptionsModel, display: Size?, onChange: (CaptureSettings) -> Unit, footer: String, modifier: Modifier = Modifier) {
    val s = settings
    Column(
        modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        SectionCard("Default video") {
            ChoiceChips("Codec", options.codecs, s.video.codec, RecordOptions::label, { onChange(s.copy(video = s.video.copy(codec = it))) })
            ChoiceChips("Resolution", options.resolutions, s.video.resolution, { RecordOptions.label(it, display) }, { onChange(s.copy(video = s.video.copy(resolution = it))) })
            ChoiceChips("Frame rate", options.frameRates, s.video.fps, { "$it fps" }, { onChange(s.copy(video = s.video.copy(fps = it))) })
            ChoiceChips("Quality", options.qualities, s.video.quality, RecordOptions::label, { onChange(s.copy(video = s.video.copy(quality = it, bitrateKbps = null))) })
            ChoiceChips("Keyframe interval", listOf(1.0, 2.0, 4.0), s.video.keyframeIntervalSec, { "${it.roundToInt()} s" }, { onChange(s.copy(video = s.video.copy(keyframeIntervalSec = it))) })
        }
        SectionCard("Default audio") {
            ChoiceChips("Sample rate", listOf(44100, 48000), s.audio.sampleRate, { "${it / 1000.0} kHz" }, { onChange(s.copy(audio = s.audio.copy(sampleRate = it))) })
            ChoiceChips("AAC bitrate", listOf(96, 128, 160, 192, 256), s.audio.bitrateKbps, { "$it kbps" }, { onChange(s.copy(audio = s.audio.copy(bitrateKbps = it))) })
            ChoiceChips(
                "When both sources are on", AudioTrackMode.entries.toList(), s.audio.trackMode,
                { if (it == AudioTrackMode.MIXED) "Mix into one track" else "Separate tracks" },
                { onChange(s.copy(audio = s.audio.copy(trackMode = it))) },
            )
        }
        SectionCard("Recording") {
            ChoiceChips("Countdown", options.countdowns, s.countdownSec, { if (it == 0) "Off" else "$it s" }, { onChange(s.copy(countdownSec = it)) })
            ChoiceChips(
                "Stop automatically after", listOf<Int?>(null, 60, 300, 900, 1800, 3600), s.maxDurationSec,
                { if (it == null) "Never" else "${it / 60} min" }, { onChange(s.copy(maxDurationSec = it)) },
            )
            ChoiceChips(
                "Stop when the file reaches", listOf<Int?>(null, 500, 1024, 4096), s.maxFileSizeMb,
                { if (it == null) "No limit" else if (it >= 1024) "${it / 1024} GB" else "$it MB" }, { onChange(s.copy(maxFileSizeMb = it)) },
            )
            ChoiceChips("Keep free storage", listOf(250, 500, 1000, 2000), s.minFreeDiskMb, { "$it MB" }, { onChange(s.copy(minFreeDiskMb = it)) })
        }
        SectionCard("Screenshots") {
            ChoiceChips("Format", ImageFormat.entries.toList(), s.screenshotFormat, { it.name }, { onChange(s.copy(screenshotFormat = it)) })
            if (s.screenshotFormat == ImageFormat.JPEG) {
                Text("JPEG quality: ${s.jpegQuality}")
                Slider(value = s.jpegQuality.toFloat(), onValueChange = { onChange(s.copy(jpegQuality = it.roundToInt().coerceIn(1, 100))) }, valueRange = 50f..100f)
            }
        }
        Box(Modifier.padding(4.dp)) { Text(footer, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}
