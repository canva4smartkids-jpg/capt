//! Recording state machine. Must match RECORDING_TRANSITIONS in packages/capture-contracts
//! (enforced by tests/contract.rs against schema/recording-transitions.json).

use crate::types::{CaptureError, RecordingState};
use RecordingState::*;

pub fn allowed_transitions(from: RecordingState) -> &'static [RecordingState] {
    match from {
        Idle => &[Countdown, Starting],
        Countdown => &[Starting, Idle],
        Starting => &[Recording, Failed],
        Recording => &[Paused, Stopping, Failed],
        Paused => &[Recording, Stopping, Failed],
        Stopping => &[Finalizing, Failed],
        Finalizing => &[Completed, Failed],
        Completed => &[Idle],
        Failed => &[Idle],
    }
}

pub fn can_transition(from: RecordingState, to: RecordingState) -> bool {
    allowed_transitions(from).contains(&to)
}

#[derive(Debug, Clone)]
pub struct StateMachine {
    state: RecordingState,
}

impl Default for StateMachine {
    fn default() -> Self {
        Self { state: Idle }
    }
}

impl StateMachine {
    pub fn state(&self) -> RecordingState {
        self.state
    }
    pub fn transition(&mut self, to: RecordingState) -> Result<RecordingState, CaptureError> {
        if can_transition(self.state, to) {
            let from = self.state;
            self.state = to;
            Ok(from)
        } else {
            Err(CaptureError::invalid_state(format!("cannot go from {:?} to {:?}", self.state, to)))
        }
    }
}
