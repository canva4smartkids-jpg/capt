@file:OptIn(ExperimentalMaterial3Api::class)

package dev.captureone.android.ui

import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Share
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.captureone.android.logic.LibraryRowModel
import dev.captureone.android.logic.StorageUsageModel

class LibraryActions(
    val onSelect: (String?) -> Unit,
    val onRename: (String, String) -> Unit,
    val onDelete: (String) -> Unit,
    val onShare: (String) -> Unit,
    val onRefresh: () -> Unit,
)

/**
 * Library with list-detail on wide screens. Thumbnails and the media preview are slots supplied by
 * the Android layer (Bitmap loading, media3 player).
 */
@Composable
fun LibraryScreen(
    rows: List<LibraryRowModel>,
    usage: StorageUsageModel?,
    selectedId: String?,
    wide: Boolean,
    actions: LibraryActions,
    thumbnail: @Composable (id: String, modifier: Modifier) -> Unit,
    preview: @Composable (id: String, modifier: Modifier) -> Unit,
    modifier: Modifier = Modifier,
) {
    val selected = rows.firstOrNull { it.id == selectedId }
    if (wide) {
        Row(modifier.fillMaxSize()) {
            LibraryList(rows, usage, selectedId, actions, thumbnail, Modifier.width(380.dp).fillMaxHeight())
            Box(Modifier.weight(1f).fillMaxHeight().padding(16.dp)) {
                if (selected != null) {
                    LibraryDetail(selected, actions, preview, showBack = false)
                } else {
                    Text("Select a recording or screenshot", modifier = Modifier.align(Alignment.Center), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        }
    } else if (selected != null) {
        Box(modifier.fillMaxSize().padding(16.dp)) { LibraryDetail(selected, actions, preview, showBack = true) }
    } else {
        LibraryList(rows, usage, selectedId, actions, thumbnail, modifier.fillMaxSize())
    }
}

@Composable
private fun LibraryList(
    rows: List<LibraryRowModel>,
    usage: StorageUsageModel?,
    selectedId: String?,
    actions: LibraryActions,
    thumbnail: @Composable (String, Modifier) -> Unit,
    modifier: Modifier,
) {
    Column(modifier) {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(usage?.text ?: "", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
            IconButton(onClick = actions.onRefresh) { Icon(Icons.Filled.Refresh, contentDescription = "Refresh") }
        }
        if (rows.isEmpty()) {
            Text("No recordings yet. Start one from the Record tab.", modifier = Modifier.padding(16.dp))
            return@Column
        }
        LazyColumn(Modifier.fillMaxSize()) {
            items(rows, key = { it.id }) { row ->
                ListItem(
                    headlineContent = { Text(row.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    supportingContent = {
                        Column {
                            Text(row.subtitle, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            row.validationNote?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall) }
                        }
                    },
                    leadingContent = { thumbnail(row.id, Modifier.size(width = 96.dp, height = 64.dp)) },
                    overlineContent = { Text(if (row.isVideo) "Video" else "Screenshot") },
                    colors = if (row.id == selectedId) ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.secondaryContainer) else ListItemDefaults.colors(),
                    modifier = Modifier.clickable { actions.onSelect(row.id) },
                )
                HorizontalDivider()
            }
        }
    }
}

@Composable
private fun LibraryDetail(row: LibraryRowModel, actions: LibraryActions, preview: @Composable (String, Modifier) -> Unit, showBack: Boolean) {
    var renaming by remember(row.id) { mutableStateOf(false) }
    var confirmDelete by remember(row.id) { mutableStateOf(false) }
    Column(Modifier.fillMaxSize().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (showBack) TextButton(onClick = { actions.onSelect(null) }) { Text("‹ Library") }
        Text(row.title, style = MaterialTheme.typography.titleLarge)
        preview(row.id, Modifier.fillMaxWidth().height(400.dp))
        Text(row.subtitle, style = MaterialTheme.typography.bodyMedium)
        row.validationNote?.let { Notice(it, isError = !it.startsWith("Kept")) }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            OutlinedButton(onClick = { renaming = true }) {
                Icon(Icons.Filled.Edit, contentDescription = null)
                Spacer(Modifier.width(6.dp))
                Text("Rename")
            }
            OutlinedButton(onClick = { actions.onShare(row.id) }, enabled = row.shareable) {
                Icon(Icons.Filled.Share, contentDescription = null)
                Spacer(Modifier.width(6.dp))
                Text("Share")
            }
            OutlinedButton(onClick = { confirmDelete = true }) {
                Icon(Icons.Filled.Delete, contentDescription = null)
                Spacer(Modifier.width(6.dp))
                Text("Delete")
            }
        }
    }
    if (renaming) {
        RenameDialog(row.title, onDismiss = { renaming = false }, onConfirm = {
            renaming = false
            actions.onRename(row.id, it)
        })
    }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text("Delete?") },
            text = { Text("\"${row.title}\" will be deleted from this device.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = false
                    actions.onDelete(row.id)
                }) { Text("Delete") }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text("Cancel") } },
        )
    }
}

@Composable
fun RenameDialog(initial: String, onDismiss: () -> Unit, onConfirm: (String) -> Unit) {
    var text by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Rename") },
        text = { OutlinedTextField(value = text, onValueChange = { text = it.take(120) }, singleLine = true, label = { Text("Title") }) },
        confirmButton = { TextButton(onClick = { onConfirm(text) }, enabled = text.isNotBlank()) { Text("Save") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}
