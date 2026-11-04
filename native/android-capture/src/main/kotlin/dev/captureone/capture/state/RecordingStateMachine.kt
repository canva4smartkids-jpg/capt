package dev.captureone.capture.state

import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.contract.RecordingState.COMPLETED
import dev.captureone.capture.contract.RecordingState.COUNTDOWN
import dev.captureone.capture.contract.RecordingState.FAILED
import dev.captureone.capture.contract.RecordingState.FINALIZING
import dev.captureone.capture.contract.RecordingState.IDLE
import dev.captureone.capture.contract.RecordingState.PAUSED
import dev.captureone.capture.contract.RecordingState.RECORDING
import dev.captureone.capture.contract.RecordingState.STARTING
import dev.captureone.capture.contract.RecordingState.STOPPING

/**
 * Canonical recording state machine — exact copy of `RECORDING_TRANSITIONS` in
 * packages/capture-contracts/src/recording.ts (contract-tested against
 * packages/capture-contracts/schema/recording-transitions.json).
 */
object RecordingTransitions {
    val TABLE: Map<RecordingState, List<RecordingState>> = mapOf(
        IDLE to listOf(COUNTDOWN, STARTING),
        COUNTDOWN to listOf(STARTING, IDLE),
        STARTING to listOf(RECORDING, FAILED),
        RECORDING to listOf(PAUSED, STOPPING, FAILED),
        PAUSED to listOf(RECORDING, STOPPING, FAILED),
        STOPPING to listOf(FINALIZING, FAILED),
        FINALIZING to listOf(COMPLETED, FAILED),
        COMPLETED to listOf(IDLE),
        FAILED to listOf(IDLE),
    )

    fun canTransition(from: RecordingState, to: RecordingState): Boolean = TABLE[from]?.contains(to) == true
}

/** Thread-safe holder of one session's state. Every change goes through [transition]. */
class RecordingStateMachine(initial: RecordingState = IDLE, private val onChange: (RecordingState, RecordingState) -> Unit = { _, _ -> }) {
    @Volatile
    var state: RecordingState = initial
        private set

    /** Moves to [to] or throws `INVALID_STATE`. */
    fun transition(to: RecordingState) {
        val from: RecordingState
        synchronized(this) {
            from = state
            if (!RecordingTransitions.canTransition(from, to)) {
                throw CaptureException(CaptureErrorCode.INVALID_STATE, "Invalid transition ${from.wireName} -> ${to.wireName}")
            }
            state = to
        }
        onChange(from, to)
    }

    /** Like [transition] but returns false instead of throwing (used on teardown paths). */
    fun tryTransition(to: RecordingState): Boolean = try {
        transition(to); true
    } catch (_: CaptureException) {
        false
    }
}
