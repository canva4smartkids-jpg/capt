package dev.captureone.tv.logic

import dev.captureone.capture.capability.NativeCaptureAssessment
import dev.captureone.capture.capability.NativeCaptureTier
import dev.captureone.capture.companion.HubAddress
import dev.captureone.capture.contract.MediaKind
import dev.captureone.capture.contract.MediaMetadata
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingStatus
import dev.captureone.capture.util.Format

/* Platform-neutral TV presentation logic (no android.*): unit-tested on the JVM. */

enum class PadField { ADDRESS, CODE }

sealed interface PadKey {
    data class Digit(val digit: Char) : PadKey { init { require(digit in '0'..'9') } }
    data object Dot : PadKey
    data object Colon : PadKey
    data object Backspace : PadKey
    data object Clear : PadKey
    data object NextField : PadKey

    val label: String
        get() = when (this) {
            is Digit -> digit.toString()
            Dot -> "."
            Colon -> ":"
            Backspace -> "⌫"
            Clear -> "Clear"
            NextField -> "Next"
        }
}

/** State of the on-screen numeric pad used to type the hub address and the 6-digit pairing code. */
data class PadState(val address: String = "", val code: String = "", val field: PadField = PadField.ADDRESS) {
    val parsedAddress: HubAddress? get() = HubAddress.parse(address)
    val codeComplete: Boolean get() = code.length == 6
    val canConnect: Boolean get() = parsedAddress != null && codeComplete
    val addressError: String?
        get() = if (address.isNotEmpty() && parsedAddress == null && !NumericPad.isPrefixOfValidAddress(address)) "Not a valid IPv4 address" else null
}

object NumericPad {
    /** Grid shown on screen (D-pad navigable), row by row. */
    val LAYOUT: List<List<PadKey>> = listOf(
        listOf(PadKey.Digit('1'), PadKey.Digit('2'), PadKey.Digit('3')),
        listOf(PadKey.Digit('4'), PadKey.Digit('5'), PadKey.Digit('6')),
        listOf(PadKey.Digit('7'), PadKey.Digit('8'), PadKey.Digit('9')),
        listOf(PadKey.Dot, PadKey.Digit('0'), PadKey.Colon),
        listOf(PadKey.Backspace, PadKey.Clear, PadKey.NextField),
    )

    private const val MAX_ADDRESS = 21 // "255.255.255.255:65535"

    /** True while [s] can still become a valid `a.b.c.d[:port]`. */
    fun isPrefixOfValidAddress(s: String): Boolean {
        val hostPart = s.substringBefore(':')
        val hasPort = s.contains(':')
        val groups = hostPart.split('.')
        if (groups.size > 4) return false
        for ((i, g) in groups.withIndex()) {
            if (g.isEmpty() && i != groups.lastIndex) return false
            if (g.length > 3) return false
            if (g.isNotEmpty() && g.toInt() > 255) return false
            if (g.length > 1 && g.startsWith("0")) return false
        }
        if (hasPort) {
            if (groups.size != 4 || groups.last().isEmpty()) return false
            val port = s.substringAfter(':')
            if (port.contains(':') || port.length > 5) return false
            if (port.isNotEmpty() && (port.toInt() > 65535)) return false
        }
        return true
    }

    fun reduce(state: PadState, key: PadKey): PadState = when (key) {
        PadKey.NextField -> state.copy(field = if (state.field == PadField.ADDRESS) PadField.CODE else PadField.ADDRESS)
        PadKey.Clear -> if (state.field == PadField.ADDRESS) state.copy(address = "") else state.copy(code = "")
        PadKey.Backspace -> if (state.field == PadField.ADDRESS) state.copy(address = state.address.dropLast(1)) else state.copy(code = state.code.dropLast(1))
        else -> {
            val ch = when (key) {
                is PadKey.Digit -> key.digit
                PadKey.Dot -> '.'
                PadKey.Colon -> ':'
                else -> null
            }
            if (ch == null) {
                state
            } else if (state.field == PadField.CODE) {
                if (ch.isDigit() && state.code.length < 6) state.copy(code = state.code + ch) else state
            } else {
                val next = state.address + ch
                if (next.length <= MAX_ADDRESS && isPrefixOfValidAddress(next)) state.copy(address = next) else state
            }
        }
    }
}

/** Texts for the capability screen shown on launch. */
data class TvHomeModel(val headline: String, val tier: NativeCaptureTier, val reasons: List<String>, val canRecord: Boolean) {
    companion object {
        fun from(a: NativeCaptureAssessment): TvHomeModel = TvHomeModel(
            headline = when (a.tier) {
                NativeCaptureTier.SUPPORTED -> "Native capture supported"
                NativeCaptureTier.PARTIAL -> "Native capture partially supported"
                NativeCaptureTier.NOT_AVAILABLE -> "Native capture not available on this device"
            },
            tier = a.tier,
            reasons = a.reasons,
            canRecord = a.tier != NativeCaptureTier.NOT_AVAILABLE,
        )
    }
}

object TvText {
    fun hubStatus(status: RecordingStatus?, connected: Boolean): String = when {
        !connected -> "Not connected to the hub's event stream"
        status == null -> "Hub is idle"
        else -> when (status.state) {
            RecordingState.RECORDING -> "Recording · ${Format.duration(status.elapsedMs)} · ${Format.bytes(status.bytesWritten)}"
            RecordingState.PAUSED -> "Paused · ${Format.duration(status.elapsedMs)}"
            RecordingState.COUNTDOWN -> "Starting in ${status.countdownRemainingSec ?: 0} s"
            RecordingState.STARTING -> "Starting…"
            RecordingState.STOPPING, RecordingState.FINALIZING -> "Saving…"
            RecordingState.COMPLETED -> "Last recording completed"
            RecordingState.FAILED -> "Last recording failed: ${status.error?.message ?: "unknown error"}"
            RecordingState.IDLE -> "Hub is idle"
        }
    }

    fun mediaLine(m: MediaMetadata): String = buildList {
        add(if (m.kind == MediaKind.VIDEO) "Video" else "Screenshot")
        m.durationMs?.let { add(Format.duration(it)) }
        add("${m.width}×${m.height}")
        add(Format.bytes(m.sizeBytes))
        add(m.createdAt.take(16).replace('T', ' '))
    }.joinToString(" · ")
}
