package dev.captureone.capture

import dev.captureone.capture.contract.CaptureErrorCode
import dev.captureone.capture.contract.CaptureException
import dev.captureone.capture.contract.CaptureJson
import dev.captureone.capture.contract.RecordingState
import dev.captureone.capture.state.RecordingStateMachine
import dev.captureone.capture.state.RecordingTransitions
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/** State machine vs. packages/capture-contracts/schema/recording-transitions.json. */
class StateMachineContractTest {
    private val fixture = Json.parseToJsonElement(RepoFiles.text("packages/capture-contracts/schema/recording-transitions.json")).jsonObject

    private fun state(wire: String) = CaptureJson.decodeFromString(RecordingState.serializer(), "\"$wire\"")

    @Test
    fun tableEqualsFixtureExactly() {
        assertEquals(fixture.keys.map(::state).toSet(), RecordingTransitions.TABLE.keys)
        assertEquals(RecordingState.entries.toSet(), RecordingTransitions.TABLE.keys)
        for ((from, targets) in fixture) {
            val expected = targets.jsonArray.map { state(it.jsonPrimitive.content) }
            assertEquals("transitions from $from", expected, RecordingTransitions.TABLE.getValue(state(from)))
        }
    }

    @Test
    fun canTransitionAgreesForEveryPair() {
        for (from in RecordingState.entries) for (to in RecordingState.entries) {
            val allowed = fixture.getValue(from.wireName).jsonArray.any { it.jsonPrimitive.content == to.wireName }
            assertEquals("$from -> $to", allowed, RecordingTransitions.canTransition(from, to))
        }
    }

    @Test
    fun machineWalksTheHappyPathWithPause() {
        val seen = ArrayList<Pair<RecordingState, RecordingState>>()
        val m = RecordingStateMachine { a, b -> seen += a to b }
        listOf(
            RecordingState.COUNTDOWN, RecordingState.STARTING, RecordingState.RECORDING, RecordingState.PAUSED,
            RecordingState.RECORDING, RecordingState.STOPPING, RecordingState.FINALIZING, RecordingState.COMPLETED, RecordingState.IDLE,
        ).forEach(m::transition)
        assertEquals(9, seen.size)
        assertEquals(RecordingState.IDLE, m.state)
    }

    @Test
    fun illegalTransitionThrowsInvalidState() {
        val m = RecordingStateMachine()
        try {
            m.transition(RecordingState.PAUSED)
            fail("expected INVALID_STATE")
        } catch (e: CaptureException) {
            assertEquals(CaptureErrorCode.INVALID_STATE, e.code)
        }
        assertEquals(RecordingState.IDLE, m.state)
        assertFalse(m.tryTransition(RecordingState.COMPLETED))
        assertTrue(m.tryTransition(RecordingState.STARTING))
        assertFalse("countdown is not reachable from starting", m.tryTransition(RecordingState.COUNTDOWN))
    }

    @Test
    fun activeStates() {
        assertEquals(setOf(RecordingState.RECORDING, RecordingState.PAUSED), RecordingState.entries.filter { it.isActive }.toSet())
    }
}
