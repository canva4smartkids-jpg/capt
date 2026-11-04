package dev.captureone.tv.tvkit

// HOST-MODE STAND-IN ONLY. Never packaged: the AGP build uses src/main/kotlin/.../tvkit/TvKit.kt
// (androidx.tv.material3). This file mirrors its signatures with plain Compose Material 3 so the TV
// screens can be type-checked on the JVM where Google Maven (tv-material) is unreachable.

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

enum class TvTextStyle { DISPLAY, HEADLINE, TITLE, BODY, LABEL }

@Composable
fun TvTheme(content: @Composable () -> Unit) = MaterialTheme(content = content)

@Composable
fun TvBackground(content: @Composable BoxScope.() -> Unit) {
    Surface(Modifier.fillMaxSize()) { Box(Modifier.fillMaxSize(), content = content) }
}

@Composable
fun TvText(text: String, modifier: Modifier = Modifier, style: TvTextStyle = TvTextStyle.BODY, muted: Boolean = false, isError: Boolean = false) {
    Text(text, modifier, style = if (style == TvTextStyle.BODY) MaterialTheme.typography.bodyLarge else MaterialTheme.typography.titleLarge)
}

@Composable
fun TvButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, primary: Boolean = false) {
    if (primary) Button(onClick, modifier, enabled) { Text(text) } else OutlinedButton(onClick, modifier, enabled) { Text(text) }
}

@Composable
fun TvListRow(title: String, subtitle: String?, onClick: () -> Unit, modifier: Modifier = Modifier, selected: Boolean = false, enabled: Boolean = true) {
    ListItem(headlineContent = { Text(title) }, supportingContent = subtitle?.let { s -> { Text(s) } }, modifier = modifier.fillMaxWidth().clickable(enabled = enabled, onClick = onClick))
}

@Composable
fun TvToggle(text: String, checked: Boolean, onCheckedChange: (Boolean) -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, note: String? = null) {
    ListItem(headlineContent = { Text(text) }, supportingContent = note?.let { n -> { Text(n) } }, trailingContent = { Text(if (checked) "On" else "Off") }, modifier = modifier.clickable(enabled = enabled) { onCheckedChange(!checked) })
}

@Composable
fun TvPanel(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Surface(modifier) { Column(Modifier.padding(20.dp), content = content) }
}

@Composable
fun TvKey(label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Button(onClick, modifier.size(width = 112.dp, height = 56.dp)) { Text(label) }
}
