package dev.captureone.capture

import dev.captureone.capture.capability.CapabilityClassifier
import dev.captureone.capture.capability.DeviceFacts
import dev.captureone.capture.capability.FormFactor
import dev.captureone.capture.companion.CompanionClient
import dev.captureone.capture.companion.HubAddress
import dev.captureone.capture.companion.HubEventListener
import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.DeviceKind
import dev.captureone.capture.contract.HubEvent
import dev.captureone.capture.contract.HubInfo
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.PairResponse
import dev.captureone.capture.contract.Platform
import dev.captureone.capture.contract.RecordingListResponse
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.contract.RemoteCommand
import dev.captureone.capture.contract.Size
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

class CompanionClientTest {
    private lateinit var server: MockWebServer
    private val token = "tok_" + "x".repeat(40)

    private val report = CapabilityClassifier.report(
        DeviceFacts(34, "14", "Acme", "Desk", "0.1.0", FormFactor.PHONE, false, true, true, emptyList(), Size(1920, 1080), null, "2026-11-04T00:00:00Z"),
    ).copy(platform = Platform.LINUX, osName = "Linux")
    private val hub = HubInfo(1, "hub-1", "Studio PC", Platform.LINUX, "0.1.0", report)

    @Before fun setUp() {
        server = MockWebServer()
        server.start()
    }

    @After fun tearDown() = server.shutdown()

    private fun client(withToken: Boolean = true) =
        CompanionClient(HubAddress(server.hostName, server.port), if (withToken) token else null)

    private fun json(body: String, code: Int = 200) = MockResponse().setResponseCode(code).setHeader("Content-Type", "application/json").setBody(body)

    @Test
    fun pairPostsCodeAndStoresToken() {
        server.enqueue(json(CaptureJson.encodeToString(PairResponse.serializer(), PairResponse(token, hub))))
        val c = client(withToken = false)
        val res = c.pair("123456", "Living room TV", DeviceKind.ANDROID_TV, null)
        assertEquals("Studio PC", res.hub.name)
        assertEquals(token, c.token)
        val req = server.takeRequest()
        assertEquals("POST", req.method)
        assertEquals("/api/v1/pair", req.path)
        assertNull("pair is unauthenticated", req.getHeader("Authorization"))
        val body = Json.parseToJsonElement(req.body.readUtf8()).jsonObject
        assertEquals("123456", body.getValue("code").jsonPrimitive.content)
        assertEquals("android-tv", body.getValue("deviceKind").jsonPrimitive.content)
    }

    @Test
    fun pairRejectsBadCodesLocally() {
        try {
            client(false).pair("12a456", "TV", DeviceKind.ANDROID_TV)
            fail()
        } catch (e: CaptureException) {
            assertEquals(CaptureErrorCode.INVALID_CONFIG, e.code)
        }
        assertEquals(0, server.requestCount)
    }

    @Test
    fun wrongCodeMapsToPermissionDenied() {
        server.enqueue(json("""{"error":"invalid pairing code"}""", 401))
        try {
            client(false).pair("000000", "TV", DeviceKind.ANDROID_TV)
            fail()
        } catch (e: CaptureException) {
            assertEquals(CaptureErrorCode.PERMISSION_DENIED, e.code)
        }
    }

    @Test
    fun recordingsSendBearerAndDecode() {
        val m = MediaMetadata("r1", MediaKind.VIDEO, "/home/u/Videos/a.mp4", "a.mp4", "a", "2026-11-04T00:00:00Z", 100, 1920, 1080, durationMs = 1000.0, container = "mp4")
        server.enqueue(json(CaptureJson.encodeToString(RecordingListResponse.serializer(), RecordingListResponse(listOf(m)))))
        val items = client().recordings()
        assertEquals(listOf(m), items)
        val req = server.takeRequest()
        assertEquals("/api/v1/recordings", req.path)
        assertEquals("Bearer $token", req.getHeader("Authorization"))
    }

    @Test
    fun commandBodyMatchesContract() {
        server.enqueue(json("""{"ok":true,"sessionId":"s-1"}"""))
        val r = client().command(RemoteCommand.StartRecording())
        assertTrue(r.ok)
        assertEquals("s-1", r.sessionId)
        val req = server.takeRequest()
        assertEquals("/api/v1/command", req.path)
        assertEquals("""{"type":"start-recording","preset":"default"}""", req.body.readUtf8())
        server.enqueue(json("""{"ok":false,"error":"not recording"}"""))
        assertEquals("not recording", client().command(RemoteCommand.PauseRecording).error)
        assertEquals("""{"type":"pause-recording"}""", server.takeRequest().body.readUtf8())
    }

    @Test
    fun statusMayBeNull() {
        server.enqueue(json("null"))
        assertNull(client().status())
        server.enqueue(json(CaptureJson.encodeToString(RecordingStatus.serializer(), RecordingStatus("s", RecordingState.RECORDING, 1500.0, null, 10, 3, 0))))
        assertEquals(RecordingState.RECORDING, client().status()?.state)
    }

    @Test
    fun unpairedClientRefusesProtectedRoutes() {
        try {
            client(false).recordings()
            fail()
        } catch (e: CaptureException) {
            assertEquals(CaptureErrorCode.PERMISSION_DENIED, e.code)
        }
    }

    @Test
    fun unreachableHubIsNetworkError() {
        val port = server.port
        server.shutdown()
        try {
            CompanionClient(HubAddress("127.0.0.1", port), token).recordings()
            fail()
        } catch (e: CaptureException) {
            assertEquals(CaptureErrorCode.NETWORK, e.code)
            assertTrue(e.error.recoverable)
        }
    }

    @Test
    fun fileUrlAndHeadersForStreaming() {
        val c = client()
        assertEquals("http://${server.hostName}:${server.port}/api/v1/recordings/abc/file", c.recordingFileUrl("abc"))
        assertEquals(mapOf("Authorization" to "Bearer $token"), c.authHeaders())
    }

    @Test
    fun eventsWebSocketDeliversHubEvents() {
        server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                webSocket.send(CaptureJson.encodeToString(HubEvent.serializer(), HubEvent.Hello(hub)))
                webSocket.send("""{"type":"future-event","x":1}""")
                webSocket.send(CaptureJson.encodeToString(HubEvent.serializer(), HubEvent.RecordingStatusChanged(RecordingStatus("s", RecordingState.PAUSED, 2.0, null, 1, 1, 0))))
                webSocket.send("""{"type":"recording-status","status":null}""")
                webSocket.send("""{"type":"library-changed"}""")
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(1000, null)
            }
        }))
        val got = CopyOnWriteArrayList<HubEvent>()
        val latch = CountDownLatch(4)
        val closed = CountDownLatch(1)
        val ws = client().events(object : HubEventListener {
            override fun onEvent(event: HubEvent) {
                got += event
                latch.countDown()
            }

            override fun onClosed(code: Int, reason: String) = closed.countDown()
        })
        assertTrue(latch.await(5, TimeUnit.SECONDS))
        ws.close(1000, null)
        assertTrue("socket closed cleanly", closed.await(5, TimeUnit.SECONDS))
        assertTrue(got[0] is HubEvent.Hello)
        assertEquals(RecordingState.PAUSED, (got[1] as HubEvent.RecordingStatusChanged).status?.state)
        assertNull((got[2] as HubEvent.RecordingStatusChanged).status)
        assertEquals(HubEvent.LibraryChanged, got[3])
        assertEquals("Bearer $token", server.takeRequest().getHeader("Authorization"))
    }

    @Test
    fun hubAddressParsing() {
        assertEquals(HubAddress("192.168.1.20", 47800), HubAddress.parse("192.168.1.20"))
        assertEquals(HubAddress("10.0.0.2", 8080), HubAddress.parse(" 10.0.0.2:8080 "))
        assertEquals(HubAddress("studio.local", 47800), HubAddress.parse("studio.local"))
        assertNull(HubAddress.parse("192.168.1.256"))
        assertNull(HubAddress.parse("192.168.01.2"))
        assertNull(HubAddress.parse("192.168.1"))
        assertNull(HubAddress.parse("192.168.1.2:0"))
        assertNull(HubAddress.parse("192.168.1.2:99999"))
        assertNull(HubAddress.parse(""))
        assertEquals("http://192.168.1.20:47800/", HubAddress("192.168.1.20").baseUrl.toString())
    }
}
