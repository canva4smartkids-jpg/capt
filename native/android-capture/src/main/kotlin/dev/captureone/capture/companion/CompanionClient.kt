package dev.captureone.capture.companion

import dev.captureone.capture.contract.CapabilityReport
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.CompanionProtocol
import dev.captureone.capture.contract.DeviceKind
import dev.captureone.capture.contract.HubEvent
import dev.captureone.capture.contract.HubInfo
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.PairRequest
import dev.captureone.capture.contract.PairResponse
import dev.captureone.capture.contract.RecordingListResponse
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.RemoteCommand
import dev.captureone.capture.contract.RemoteCommandResult
import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerializationException
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.io.IOException
import java.util.concurrent.TimeUnit

/** A hub address typed by the user (`192.168.1.20` or `192.168.1.20:47800`). */
data class HubAddress(val host: String, val port: Int = CompanionProtocol.DEFAULT_PORT) {
    val baseUrl: HttpUrl get() = HttpUrl.Builder().scheme("http").host(host).port(port).build()

    override fun toString(): String = "$host:$port"

    companion object {
        private val IPV4 = Regex("^(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})\\.(\\d{1,3})$")
        private val HOSTNAME = Regex("^(?=.{1,253}$)([A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)(\\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)*$")

        fun isValidIpv4(s: String): Boolean {
            val m = IPV4.matchEntire(s) ?: return false
            return m.groupValues.drop(1).all { part -> part.toInt() in 0..255 && (part.length == 1 || !part.startsWith("0")) }
        }

        /** RFC 1918, loopback, link-local and CGNAT (Tailscale-style) IPv4 ranges. */
        fun isPrivateIpv4(s: String): Boolean {
            if (!isValidIpv4(s)) return false
            val (a, b) = s.split('.').map { it.toInt() }.take(2)
            return a == 10 || a == 127 || (a == 172 && b in 16..31) || (a == 192 && b == 168) || (a == 169 && b == 254) || (a == 100 && b in 64..127)
        }

        private val LAN_SUFFIXES = listOf(".local", ".lan", ".home", ".home.arpa", ".internal", ".localdomain")

        /** Single-label or mDNS/LAN-style names only: the pairing token travels in clear text. */
        fun isLanHostname(h: String): Boolean {
            val l = h.lowercase()
            return !l.contains('.') || LAN_SUFFIXES.any { l.endsWith(it) }
        }

        /**
         * Parses `host[:port]`. Returns null for anything that is not a private-network IPv4 address or
         * a LAN-style host name (protocol v1 is plain HTTP, so public hosts are refused).
         */
        fun parse(input: String): HubAddress? {
            val s = input.trim()
            if (s.isEmpty()) return null
            val idx = s.lastIndexOf(':')
            val host = if (idx >= 0) s.substring(0, idx) else s
            val port = if (idx >= 0) s.substring(idx + 1).toIntOrNull() ?: return null else CompanionProtocol.DEFAULT_PORT
            if (port !in 1..65535) return null
            val looksNumeric = host.all { it.isDigit() || it == '.' }
            if (looksNumeric) {
                if (!isPrivateIpv4(host)) return null
            } else if (!HOSTNAME.matches(host) || !isLanHostname(host)) {
                return null
            }
            return HubAddress(host, port)
        }
    }
}

/** Listener for the hub's `/api/v1/events` WebSocket. Callbacks run on OkHttp's thread. */
interface HubEventListener {
    fun onOpen() {}
    fun onEvent(event: HubEvent)
    fun onClosed(code: Int, reason: String) {}
    fun onFailure(error: CaptureException) {}
}

/**
 * HTTP+WebSocket client for the CaptureOne desktop hub (packages/capture-contracts/src/companion.ts).
 * Blocking calls — invoke from a background dispatcher. All routes except info/pair carry
 * `Authorization: Bearer <token>`.
 *
 * Protocol v1 is plain HTTP on the LAN, authenticated by the pairing token (the hub is off by default
 * and pairing needs the 6-digit code shown on the desktop).
 */
class CompanionClient(
    val address: HubAddress,
    token: String? = null,
    private val http: OkHttpClient = defaultHttpClient(),
) {
    @Volatile
    var token: String? = token
        private set

    private val jsonType = "application/json; charset=utf-8".toMediaType()

    fun info(): HubInfo = call(get(CompanionProtocol.INFO, auth = false), HubInfo.serializer())

    /** Pairs with the hub using the code shown on the desktop; stores and returns the bearer token. */
    fun pair(code: String, deviceName: String, deviceKind: DeviceKind, capabilities: CapabilityReport? = null): PairResponse {
        if (!PairRequest.PAIRING_CODE.matches(code)) {
            throw CaptureException(CaptureErrorCode.INVALID_CONFIG, "The pairing code must be exactly 6 digits.")
        }
        val body = CaptureJson.encodeToString(PairRequest.serializer(), PairRequest(code, deviceName.take(64).ifBlank { "Android device" }, deviceKind, capabilities))
        val req = Request.Builder().url(url(CompanionProtocol.PAIR)).post(body.toRequestBody(jsonType)).build()
        val res = call(req, PairResponse.serializer())
        if (res.token.length < 32) throw CaptureException(CaptureErrorCode.NETWORK, "Hub returned an invalid token.")
        if (res.hub.protocolVersion != CompanionProtocol.VERSION) {
            throw CaptureException(CaptureErrorCode.UNSUPPORTED, "Hub speaks companion protocol v${res.hub.protocolVersion}; this app supports v${CompanionProtocol.VERSION}.")
        }
        token = res.token
        return res
    }

    fun recordings(): List<MediaMetadata> = call(get(CompanionProtocol.RECORDINGS), RecordingListResponse.serializer()).items

    /** Status of the hub's current recording, or null when idle (`GET /api/v1/status` may return `null`). */
    fun status(): RecordingStatus? {
        val req = get(CompanionProtocol.STATUS)
        return execute(req) { body ->
            if (body.isBlank() || body.trim() == "null") null else CaptureJson.decodeFromString(RecordingStatus.serializer(), body)
        }
    }

    fun command(command: RemoteCommand): RemoteCommandResult {
        val body = CaptureJson.encodeToString(RemoteCommand.serializer(), command)
        val req = authorized(Request.Builder().url(url(CompanionProtocol.COMMAND)).post(body.toRequestBody(jsonType))).build()
        return call(req, RemoteCommandResult.serializer())
    }

    /** URL of a recording's file for streaming (send [authHeaders] with the request). */
    fun recordingFileUrl(id: String): String = url(CompanionProtocol.recordingFile(id)).toString()

    fun recordingThumbnailUrl(id: String): String = url(CompanionProtocol.recordingThumbnail(id)).toString()

    /** Headers to attach to media requests (e.g. media3 `DefaultHttpDataSource.Factory`). */
    fun authHeaders(): Map<String, String> = token?.let { mapOf("Authorization" to "Bearer $it") } ?: emptyMap()

    /** Opens the status WebSocket. Close the returned socket to stop. */
    fun events(listener: HubEventListener): WebSocket {
        val wsUrl = url(CompanionProtocol.EVENTS).toString().replaceFirst("http", "ws")
        val req = authorized(Request.Builder().url(wsUrl)).build()
        return http.newWebSocket(req, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) = listener.onOpen()

            override fun onMessage(webSocket: WebSocket, text: String) {
                val event = try {
                    CaptureJson.decodeFromString(HubEvent.serializer(), text)
                } catch (e: SerializationException) {
                    return // unknown event type from a newer hub: ignore
                } catch (e: IllegalArgumentException) {
                    return
                }
                listener.onEvent(event)
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = listener.onClosed(code, reason)

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                val err = if (response?.code == 401 || response?.code == 403) {
                    CaptureException(CaptureErrorCode.PERMISSION_DENIED, "Hub rejected the pairing token; pair again.", cause = t)
                } else {
                    CaptureException(CaptureErrorCode.NETWORK, "Event stream lost: ${t.message ?: t::class.simpleName}", recoverable = true, cause = t)
                }
                listener.onFailure(err)
            }
        })
    }

    // ------------------------------------------------------------------ internals

    private fun url(path: String): HttpUrl = (address.baseUrl.toString().trimEnd('/') + path).toHttpUrl()

    private fun authorized(b: Request.Builder): Request.Builder {
        val t = token ?: throw CaptureException(CaptureErrorCode.PERMISSION_DENIED, "Not paired with the hub.")
        return b.header("Authorization", "Bearer $t")
    }

    private fun get(path: String, auth: Boolean = true): Request {
        val b = Request.Builder().url(url(path)).get()
        return (if (auth) authorized(b) else b).build()
    }

    private fun <T> call(req: Request, serializer: KSerializer<T>): T = execute(req) { body ->
        CaptureJson.decodeFromString(serializer, body)
    }

    private fun <T> execute(req: Request, parse: (String) -> T): T {
        val response = try {
            http.newCall(req).execute()
        } catch (e: IOException) {
            throw CaptureException(CaptureErrorCode.NETWORK, "Cannot reach the hub at $address: ${e.message ?: e::class.simpleName}", recoverable = true, cause = e)
        }
        response.use { r ->
            val body = r.body?.string().orEmpty()
            if (!r.isSuccessful) throw httpError(r.code, body)
            return try {
                parse(body)
            } catch (e: SerializationException) {
                throw CaptureException(CaptureErrorCode.NETWORK, "Unexpected response from the hub: ${e.message}", cause = e)
            } catch (e: IllegalArgumentException) {
                throw CaptureException(CaptureErrorCode.NETWORK, "Unexpected response from the hub: ${e.message}", cause = e)
            }
        }
    }

    private fun httpError(code: Int, body: String): CaptureException {
        val detail = body.take(200).ifBlank { "HTTP $code" }
        return when (code) {
            401, 403 -> CaptureException(CaptureErrorCode.PERMISSION_DENIED, "The hub rejected the request (wrong code or expired pairing): $detail")
            404 -> CaptureException(CaptureErrorCode.SOURCE_NOT_FOUND, "Not found on the hub: $detail")
            400, 422 -> CaptureException(CaptureErrorCode.INVALID_CONFIG, "The hub rejected the request: $detail")
            409 -> CaptureException(CaptureErrorCode.INVALID_STATE, "The hub cannot do that right now: $detail", recoverable = true)
            429 -> CaptureException(CaptureErrorCode.DEVICE_BUSY, "Too many attempts; wait and retry: $detail", recoverable = true)
            else -> CaptureException(CaptureErrorCode.NETWORK, "Hub error HTTP $code: $detail", recoverable = code >= 500)
        }
    }

    companion object {
        fun defaultHttpClient(): OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(4, TimeUnit.SECONDS)
            .readTimeout(15, TimeUnit.SECONDS)
            .pingInterval(20, TimeUnit.SECONDS)
            .build()
    }
}
