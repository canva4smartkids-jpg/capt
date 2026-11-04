package dev.captureone.tv

import android.app.Application
import android.os.Build
import android.os.Environment
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.captureone.capture.capability.CapabilityClassifier
import dev.captureone.capture.capability.DeviceFacts
import dev.captureone.capture.capability.DeviceFactsCollector
import dev.captureone.capture.capability.NativeCaptureAssessment
import dev.captureone.capture.companion.CompanionClient
import dev.captureone.capture.companion.HubAddress
import dev.captureone.capture.companion.HubEventListener
import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.CaptureEvent
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.FeatureKey
import dev.captureone.capture.contract.HubEvent
import dev.captureone.capture.contract.HubInfo
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.QualityPreset
import dev.captureone.capture.contract.RecordingConfig
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.RemoteCommand
import dev.captureone.capture.contract.toCaptureError
import dev.captureone.capture.library.LibraryItem
import dev.captureone.capture.library.LibraryRepository
import dev.captureone.capture.service.CaptureController
import dev.captureone.capture.settings.CaptureSettings
import dev.captureone.capture.settings.SettingsStore
import dev.captureone.tv.logic.NumericPad
import dev.captureone.tv.logic.PadField
import dev.captureone.tv.logic.PadKey
import dev.captureone.tv.logic.PadState
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import okhttp3.WebSocket
import java.io.File

/** State for the TV app: device capability, native recording settings, local library and companion mode. */
class TvViewModel(app: Application) : AndroidViewModel(app) {
    private val store = SettingsStore(app)
    private val library = LibraryRepository.get(app)

    private val _facts = MutableStateFlow<DeviceFacts?>(null)
    val facts: StateFlow<DeviceFacts?> = _facts.asStateFlow()
    private val _report = MutableStateFlow<CapabilityReport?>(null)
    val report: StateFlow<CapabilityReport?> = _report.asStateFlow()
    private val _assessment = MutableStateFlow<NativeCaptureAssessment?>(null)
    val assessment: StateFlow<NativeCaptureAssessment?> = _assessment.asStateFlow()

    private val _settings = MutableStateFlow(store.load().let { it.copy(countdownSec = if (it.countdownSec == 3) 5 else it.countdownSec) })
    val settings: StateFlow<CaptureSettings> = _settings.asStateFlow()

    private val _items = MutableStateFlow<List<LibraryItem>>(emptyList())
    val items: StateFlow<List<LibraryItem>> = _items.asStateFlow()

    private val _message = MutableStateFlow<String?>(null)
    val message: StateFlow<String?> = _message.asStateFlow()

    // companion mode
    private val _pad = MutableStateFlow(PadState())
    val pad: StateFlow<PadState> = _pad.asStateFlow()
    private val _savedHub = MutableStateFlow(store.companionHub()?.first)
    val savedHub: StateFlow<String?> = _savedHub.asStateFlow()
    private val _hub = MutableStateFlow<HubInfo?>(null)
    val hub: StateFlow<HubInfo?> = _hub.asStateFlow()
    private val _hubStatus = MutableStateFlow<RecordingStatus?>(null)
    val hubStatus: StateFlow<RecordingStatus?> = _hubStatus.asStateFlow()
    private val _hubConnected = MutableStateFlow(false)
    val hubConnected: StateFlow<Boolean> = _hubConnected.asStateFlow()
    private val _hubRecordings = MutableStateFlow<List<MediaMetadata>>(emptyList())
    val hubRecordings: StateFlow<List<MediaMetadata>> = _hubRecordings.asStateFlow()
    private val _companionBusy = MutableStateFlow(false)
    val companionBusy: StateFlow<Boolean> = _companionBusy.asStateFlow()
    private val _companionError = MutableStateFlow<String?>(null)
    val companionError: StateFlow<String?> = _companionError.asStateFlow()
    private val _lastCommandResult = MutableStateFlow<String?>(null)
    val lastCommandResult: StateFlow<String?> = _lastCommandResult.asStateFlow()

    @Volatile var client: CompanionClient? = null
        private set
    private var socket: WebSocket? = null

    init {
        viewModelScope.launch(Dispatchers.Default) {
            val f = DeviceFactsCollector.collect(app, DeviceFactsCollector.appVersion(app))
            _facts.value = f
            _report.value = CapabilityClassifier.report(f)
            _assessment.value = CapabilityClassifier.nativeCaptureAssessment(f)
        }
        refreshLibrary()
        viewModelScope.launch {
            CaptureController.events.collect { if (it is CaptureEvent.Completed || it is CaptureEvent.Screenshot) refreshLibrary() }
        }
        viewModelScope.launch { CaptureController.lastError.collect { _message.value = it?.message } }
    }

    // ------------------------------------------------------------------ native recording

    private fun save(s: CaptureSettings) {
        _settings.value = s
        store.save(s)
    }

    fun cycleQuality() {
        val order = listOf(QualityPreset.LOW, QualityPreset.MEDIUM, QualityPreset.HIGH, QualityPreset.ULTRA)
        val s = _settings.value
        save(s.copy(video = s.video.copy(quality = order[(order.indexOf(s.video.quality).coerceAtLeast(0) + 1) % order.size], bitrateKbps = null)))
    }

    fun cycleFps() {
        val s = _settings.value
        val list = listOf(24, 30, 60)
        save(s.copy(video = s.video.copy(fps = list[(list.indexOf(s.video.fps).coerceAtLeast(0) + 1) % list.size])))
    }

    fun cycleCountdown() {
        val s = _settings.value
        val list = CaptureSettings.COUNTDOWN_CHOICES
        save(s.copy(countdownSec = list[(list.indexOf(s.countdownSec).coerceAtLeast(0) + 1) % list.size]))
    }

    fun cycleMaxDuration() {
        val list = listOf<Int?>(null, 300, 900, 1800, 3600)
        val s = _settings.value
        save(s.copy(maxDurationSec = list[(list.indexOf(s.maxDurationSec).coerceAtLeast(0) + 1) % list.size]))
    }

    fun setSystemAudio(on: Boolean) {
        val s = _settings.value
        save(s.copy(audio = s.audio.copy(system = s.audio.system.copy(enabled = on), microphone = s.audio.microphone.copy(enabled = false))))
    }

    fun systemAudioAvailable(): Boolean = _report.value?.isUsable(FeatureKey.SYSTEM_AUDIO) == true && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q

    fun recordingConfig(): RecordingConfig {
        val app = getApplication<Application>()
        val dir = File(app.getExternalFilesDir(Environment.DIRECTORY_MOVIES) ?: app.filesDir, "recordings-tmp")
        var s = _settings.value
        if (s.audio.system.enabled && !systemAudioAvailable()) s = s.copy(audio = s.audio.copy(system = s.audio.system.copy(enabled = false)))
        if (_report.value?.isUsable(FeatureKey.H265) != true) s = s.copy(video = s.video.copy(codec = dev.captureone.capture.contract.VideoCodec.H264))
        return s.toRecordingConfig(dir.absolutePath)
    }

    fun clearMessage() {
        _message.value = null
        CaptureController.clearError()
    }

    fun showMessage(m: String) {
        _message.value = m
    }

    fun refreshLibrary() {
        viewModelScope.launch(Dispatchers.IO) {
            library.prune()
            _items.value = library.list()
        }
    }

    fun item(id: String): LibraryItem? = _items.value.firstOrNull { it.id == id }

    fun delete(id: String) {
        viewModelScope.launch(Dispatchers.IO) {
            val r = library.delete(id)
            if (r is dev.captureone.capture.library.LibraryActionResult.Failed) _message.value = r.message
            refreshLibrary()
        }
    }

    // ------------------------------------------------------------------ companion mode

    fun padKey(key: PadKey) {
        _pad.value = NumericPad.reduce(_pad.value, key)
    }

    fun selectField(f: PadField) {
        _pad.value = _pad.value.copy(field = f)
    }

    fun pairAndConnect(onConnected: () -> Unit) {
        val p = _pad.value
        val address = p.parsedAddress ?: return
        _companionBusy.value = true
        _companionError.value = null
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val c = CompanionClient(address)
                val facts = _facts.value
                val res = c.pair(
                    p.code,
                    deviceName = "${Build.MANUFACTURER} ${Build.MODEL}".trim().take(64),
                    deviceKind = facts?.let { CapabilityClassifier.deviceKind(it) } ?: dev.captureone.capture.contract.DeviceKind.ANDROID_TV,
                    capabilities = _report.value,
                )
                store.saveCompanionHub(address.toString(), res.token)
                _savedHub.value = address.toString()
                attach(c, res.hub)
                _pad.value = PadState(address = p.address)
                launch(Dispatchers.Main) { onConnected() }
            } catch (e: CaptureException) {
                _companionError.value = e.error.message
            } catch (e: Exception) {
                _companionError.value = e.toCaptureError().message
            } finally {
                _companionBusy.value = false
            }
        }
    }

    fun reconnectSaved(onConnected: () -> Unit) {
        val (addr, token) = store.companionHub() ?: return
        val address = HubAddress.parse(addr) ?: return
        _companionBusy.value = true
        _companionError.value = null
        viewModelScope.launch(Dispatchers.IO) {
            try {
                val c = CompanionClient(address, token)
                val info = c.info()
                c.recordings() // proves the token is still accepted
                attach(c, info)
                launch(Dispatchers.Main) { onConnected() }
            } catch (e: CaptureException) {
                _companionError.value = e.error.message
            } finally {
                _companionBusy.value = false
            }
        }
    }

    private fun attach(c: CompanionClient, info: HubInfo) {
        client = c
        _hub.value = info
        socket?.close(1000, null)
        socket = c.events(object : HubEventListener {
            override fun onOpen() { _hubConnected.value = true }
            override fun onEvent(event: HubEvent) {
                when (event) {
                    is HubEvent.Hello -> _hub.value = event.hub
                    is HubEvent.RecordingStatusChanged -> _hubStatus.value = event.status
                    HubEvent.LibraryChanged -> refreshHubRecordings()
                }
            }
            override fun onClosed(code: Int, reason: String) { _hubConnected.value = false }
            override fun onFailure(error: CaptureException) {
                _hubConnected.value = false
                _companionError.value = error.error.message
            }
        })
        refreshHubRecordings()
        viewModelScope.launch(Dispatchers.IO) { runCatching { _hubStatus.value = c.status() } }
    }

    fun refreshHubRecordings() {
        val c = client ?: return
        viewModelScope.launch(Dispatchers.IO) {
            try {
                _hubRecordings.value = c.recordings()
            } catch (e: CaptureException) {
                _companionError.value = e.error.message
            }
        }
    }

    fun command(cmd: RemoteCommand) {
        val c = client ?: return
        viewModelScope.launch(Dispatchers.IO) {
            _lastCommandResult.value = try {
                val r = c.command(cmd)
                if (r.ok) {
                    r.media?.let { "Done: ${it.fileName}" } ?: "Done"
                } else {
                    "Hub refused: ${r.error ?: "unknown reason"}"
                }
            } catch (e: CaptureException) {
                e.error.message
            }
        }
    }

    fun forgetHub() {
        socket?.close(1000, null)
        socket = null
        client = null
        _hub.value = null
        _hubStatus.value = null
        _hubRecordings.value = emptyList()
        _hubConnected.value = false
        store.clearCompanionHub()
        _savedHub.value = null
    }

    override fun onCleared() {
        socket?.close(1000, null)
        super.onCleared()
    }
}
