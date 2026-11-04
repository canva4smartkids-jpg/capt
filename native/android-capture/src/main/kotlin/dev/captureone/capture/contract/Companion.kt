@file:OptIn(ExperimentalSerializationApi::class)

package dev.captureone.capture.contract

import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonClassDiscriminator

/** Mirror of packages/capture-contracts/src/companion.ts (hub ⇄ TV / mobile companions). */
object CompanionProtocol {
    const val VERSION: Int = 1
    const val DEFAULT_PORT: Int = 47800

    const val INFO = "/api/v1/info"
    const val PAIR = "/api/v1/pair"
    const val RECORDINGS = "/api/v1/recordings"
    const val STATUS = "/api/v1/status"
    const val COMMAND = "/api/v1/command"
    const val EVENTS = "/api/v1/events"
    fun recordingFile(id: String) = "/api/v1/recordings/$id/file"
    fun recordingThumbnail(id: String) = "/api/v1/recordings/$id/thumbnail"
}

@Serializable
enum class DeviceKind {
    @SerialName("desktop") DESKTOP,
    @SerialName("android-phone") ANDROID_PHONE,
    @SerialName("android-tablet") ANDROID_TABLET,
    @SerialName("android-tv") ANDROID_TV,
    @SerialName("google-tv") GOOGLE_TV,
    @SerialName("samsung-tizen-tv") SAMSUNG_TIZEN_TV,
    @SerialName("lg-webos-tv") LG_WEBOS_TV,
    @SerialName("other-tv") OTHER_TV,
    @SerialName("unknown") UNKNOWN,
}

@Serializable
enum class ControlProtocol {
    @SerialName("captureone-companion") CAPTUREONE_COMPANION,
    @SerialName("lg-ssap") LG_SSAP,
    @SerialName("samsung-remote-ws") SAMSUNG_REMOTE_WS,
    @SerialName("android-tv-remote") ANDROID_TV_REMOTE,
    @SerialName("dial") DIAL,
    @SerialName("upnp-media-renderer") UPNP_MEDIA_RENDERER,
}

@Serializable
enum class DiscoveryMethod {
    @SerialName("ssdp") SSDP,
    @SerialName("mdns") MDNS,
    @SerialName("manual") MANUAL,
}

@Serializable
data class DiscoveredDevice(
    val id: String,
    val name: String,
    val kind: DeviceKind,
    val manufacturer: String? = null,
    val model: String? = null,
    val address: String,
    val port: Int? = null,
    val discoveredVia: List<DiscoveryMethod>,
    val controlProtocols: List<ControlProtocol>,
    val services: List<String>,
    val lastSeen: String,
)

@Serializable
data class HubInfo(
    val protocolVersion: Int,
    val hubId: String,
    val name: String,
    val platform: Platform,
    val appVersion: String,
    val capabilities: CapabilityReport,
)

@Serializable
data class PairRequest(
    /** Exactly six digits. */
    val code: String,
    val deviceName: String,
    val deviceKind: DeviceKind,
    val capabilities: CapabilityReport? = null,
) {
    init {
        require(PAIRING_CODE.matches(code)) { "pairing code must be 6 digits" }
        require(deviceName.length in 1..64) { "deviceName must be 1..64 characters" }
    }

    companion object {
        val PAIRING_CODE = Regex("^\\d{6}$")
    }
}

@Serializable
data class PairResponse(val token: String, val hub: HubInfo)

@Serializable
enum class StartPreset {
    @SerialName("default") DEFAULT,
    @SerialName("capture-device") CAPTURE_DEVICE,
}

@Serializable
@JsonClassDiscriminator("type")
sealed interface RemoteCommand {
    @Serializable @SerialName("start-recording")
    data class StartRecording(val preset: StartPreset = StartPreset.DEFAULT) : RemoteCommand

    @Serializable @SerialName("pause-recording")
    data object PauseRecording : RemoteCommand

    @Serializable @SerialName("resume-recording")
    data object ResumeRecording : RemoteCommand

    @Serializable @SerialName("stop-recording")
    data object StopRecording : RemoteCommand

    @Serializable @SerialName("screenshot")
    data object Screenshot : RemoteCommand
}

@Serializable
data class RemoteCommandResult(
    val ok: Boolean,
    val sessionId: String? = null,
    val media: MediaMetadata? = null,
    val error: String? = null,
)

@Serializable
@JsonClassDiscriminator("type")
sealed interface HubEvent {
    @Serializable @SerialName("hello")
    data class Hello(val hub: HubInfo) : HubEvent

    /** `status` is null when the hub is not recording. */
    @Serializable @SerialName("recording-status")
    data class RecordingStatusChanged(val status: RecordingStatus?) : HubEvent

    @Serializable @SerialName("library-changed")
    data object LibraryChanged : HubEvent
}

@Serializable
data class RecordingListResponse(val items: List<MediaMetadata>)
