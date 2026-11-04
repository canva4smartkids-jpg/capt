package dev.captureone.android

import android.app.Activity
import android.app.Application
import android.content.Intent
import android.content.IntentSender
import android.os.Environment
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.captureone.android.logic.RecordOptions
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.capability.PermissionInspector
import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.CaptureEvent
import dev.captureone.capture.contract.PermissionKind
import dev.captureone.capture.contract.PermissionStatus
import dev.captureone.capture.contract.RecordingConfig
import dev.captureone.capture.contract.Size
import dev.captureone.capture.engine.DisplayInfo
import dev.captureone.capture.library.LibraryActionResult
import dev.captureone.capture.library.LibraryItem
import dev.captureone.capture.library.LibraryRepository
import dev.captureone.capture.library.StorageUsage
import dev.captureone.capture.presets.EncodingPresets
import dev.captureone.capture.service.CaptureController
import dev.captureone.capture.service.CaptureService
import dev.captureone.capture.settings.CaptureSettings
import dev.captureone.capture.settings.SettingsStore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

/** Holds UI state for the phone/tablet app and talks to the :capture engine. */
class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val store = SettingsStore(app)
    private val library = LibraryRepository.get(app)

    private val _settings = MutableStateFlow(store.load())
    val settings: StateFlow<CaptureSettings> = _settings.asStateFlow()

    private val _report = MutableStateFlow<CapabilityReport?>(null)
    val report: StateFlow<CapabilityReport?> = _report.asStateFlow()

    private val _consentAvailable = MutableStateFlow(true)
    val consentAvailable: StateFlow<Boolean> = _consentAvailable.asStateFlow()

    val display: Size? = runCatching { DisplayInfo.of(app).size }.getOrNull()
    val presets: EncodingPresets? = runCatching { CaptureService.loadPresets(app) }.getOrNull()

    private val _permissions = MutableStateFlow<List<PermissionStatus>>(emptyList())
    val permissions: StateFlow<List<PermissionStatus>> = _permissions.asStateFlow()

    private val _items = MutableStateFlow<List<LibraryItem>>(emptyList())
    val items: StateFlow<List<LibraryItem>> = _items.asStateFlow()

    private val _usage = MutableStateFlow<StorageUsage?>(null)
    val usage: StateFlow<StorageUsage?> = _usage.asStateFlow()

    private val _message = MutableStateFlow<Pair<String, Boolean>?>(null)
    /** Message to show (text, isError). */
    val message: StateFlow<Pair<String, Boolean>?> = _message.asStateFlow()

    init {
        viewModelScope.launch(Dispatchers.Default) {
            _consentAvailable.value = DeviceFactsCollector.consentActivityAvailable(app)
            val r = DeviceFactsCollector.capabilityReport(app)
            _report.value = r
            val sanitized = RecordOptions.sanitize(_settings.value, r)
            if (sanitized != _settings.value) updateSettings(sanitized)
        }
        refreshLibrary()
        viewModelScope.launch {
            CaptureController.events.collect { e ->
                when (e) {
                    is CaptureEvent.Completed, is CaptureEvent.Screenshot -> refreshLibrary()
                    is CaptureEvent.Warning -> Unit
                    else -> Unit
                }
            }
        }
        viewModelScope.launch {
            CaptureController.lastError.collect { err -> if (err != null) _message.value = err.message to true }
        }
    }

    fun updateSettings(s: CaptureSettings) {
        _settings.value = s
        store.save(s)
    }

    fun refreshPermissions(activity: Activity) {
        _permissions.value = PermissionKind.entries.map { PermissionInspector.status(getApplication(), it, activity) }
    }

    fun markRequested(permission: String) = store.markRequested(permission)

    fun dismissMessage() {
        _message.value = null
        CaptureController.clearError()
    }

    fun showMessage(text: String, isError: Boolean) {
        _message.value = text to isError
    }

    /** Working directory for in-progress recordings (app-specific; published to MediaStore when done). */
    private fun tempDir(): File {
        val app = getApplication<Application>()
        return File(app.getExternalFilesDir(Environment.DIRECTORY_MOVIES) ?: app.filesDir, "recordings-tmp")
    }

    fun recordingConfig(): RecordingConfig = RecordOptions.sanitize(_settings.value, _report.value).toRecordingConfig(tempDir().absolutePath)

    fun refreshLibrary() {
        viewModelScope.launch(Dispatchers.IO) {
            library.prune()
            _items.value = library.list()
            _usage.value = library.storageUsage()
        }
    }

    fun item(id: String): LibraryItem? = _items.value.firstOrNull { it.id == id }

    suspend fun rename(id: String, title: String): IntentSender? = withContext(Dispatchers.IO) {
        when (val r = library.rename(id, title)) {
            is LibraryActionResult.NeedsConsent -> r.intentSender
            is LibraryActionResult.Failed -> { showMessage(r.message, true); null }
            LibraryActionResult.Done -> { refreshLibrary(); null }
        }
    }

    suspend fun delete(id: String): IntentSender? = withContext(Dispatchers.IO) {
        when (val r = library.delete(id)) {
            is LibraryActionResult.NeedsConsent -> r.intentSender
            is LibraryActionResult.Failed -> { showMessage(r.message, true); null }
            LibraryActionResult.Done -> { refreshLibrary(); null }
        }
    }

    fun completeDelete(id: String) {
        viewModelScope.launch(Dispatchers.IO) {
            library.completeDelete(id)
            refreshLibrary()
        }
    }

    fun shareIntent(id: String): Intent? = item(id)?.let { library.shareIntent(it) }
}
