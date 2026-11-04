@file:OptIn(ExperimentalTvMaterial3Api::class)

package dev.captureone.tv.tvkit

import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.unit.dp
import androidx.tv.material3.Button
import androidx.tv.material3.ExperimentalTvMaterial3Api
import androidx.tv.material3.ListItem
import androidx.tv.material3.MaterialTheme
import androidx.tv.material3.OutlinedButton
import androidx.tv.material3.Surface
import androidx.tv.material3.SurfaceDefaults
import androidx.tv.material3.Text
import androidx.tv.material3.darkColorScheme

/*
 * Thin adapter over Compose for TV (androidx.tv.material3). The TV screens in dev.captureone.tv.ui
 * use only these components, which gives them TV focus visuals (scale/border/glow on focus) and lets
 * the screens be type-checked in host mode against a stand-in implementation (src/hostStub).
 */

enum class TvTextStyle { DISPLAY, HEADLINE, TITLE, BODY, LABEL }

private val Scheme = darkColorScheme(
    primary = Color(0xFFFF8A80),
    onPrimary = Color(0xFF3B0907),
    secondary = Color(0xFFBEC2FF),
    background = Color(0xFF121216),
    surface = Color(0xFF1B1B1F),
    surfaceVariant = Color(0xFF2A2A30),
    error = Color(0xFFFFB4AB),
)

@Composable
fun TvTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = Scheme, content = content)
}

@Composable
fun TvBackground(content: @Composable BoxScope.() -> Unit) {
    Surface(
        modifier = Modifier.fillMaxSize(),
        shape = RectangleShape,
        colors = SurfaceDefaults.colors(containerColor = MaterialTheme.colorScheme.background),
        content = content,
    )
}

@Composable
fun TvText(text: String, modifier: Modifier = Modifier, style: TvTextStyle = TvTextStyle.BODY, muted: Boolean = false, isError: Boolean = false) {
    val t = MaterialTheme.typography
    val textStyle = when (style) {
        TvTextStyle.DISPLAY -> t.displaySmall
        TvTextStyle.HEADLINE -> t.headlineMedium
        TvTextStyle.TITLE -> t.titleLarge
        TvTextStyle.BODY -> t.bodyLarge
        TvTextStyle.LABEL -> t.labelLarge
    }
    val color = when {
        isError -> MaterialTheme.colorScheme.error
        muted -> MaterialTheme.colorScheme.onSurfaceVariant
        else -> MaterialTheme.colorScheme.onSurface
    }
    Text(text = text, modifier = modifier, style = textStyle, color = color)
}

@Composable
fun TvButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, primary: Boolean = false) {
    if (primary) {
        Button(onClick = onClick, modifier = modifier, enabled = enabled) { Text(text) }
    } else {
        OutlinedButton(onClick = onClick, modifier = modifier, enabled = enabled) { Text(text) }
    }
}

@Composable
fun TvListRow(title: String, subtitle: String?, onClick: () -> Unit, modifier: Modifier = Modifier, selected: Boolean = false, enabled: Boolean = true) {
    ListItem(
        selected = selected,
        onClick = onClick,
        headlineContent = { Text(title) },
        modifier = modifier.fillMaxWidth(),
        enabled = enabled,
        supportingContent = subtitle?.let { s -> { Text(s) } },
    )
}

@Composable
fun TvToggle(text: String, checked: Boolean, onCheckedChange: (Boolean) -> Unit, modifier: Modifier = Modifier, enabled: Boolean = true, note: String? = null) {
    ListItem(
        selected = checked,
        onClick = { onCheckedChange(!checked) },
        headlineContent = { Text(text) },
        modifier = modifier.fillMaxWidth(),
        enabled = enabled,
        supportingContent = note?.let { n -> { Text(n) } },
        trailingContent = { Text(if (checked) "On" else "Off") },
    )
}

@Composable
fun TvPanel(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
    Surface(
        modifier = modifier,
        shape = RoundedCornerShape(12.dp),
        colors = SurfaceDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceVariant),
    ) {
        Column(Modifier.padding(20.dp), content = content)
    }
}

/** One key of the on-screen numeric pad. */
@Composable
fun TvKey(label: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Button(onClick = onClick, modifier = modifier.size(width = 112.dp, height = 56.dp)) { Text(label) }
}
