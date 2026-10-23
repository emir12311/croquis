//! Stepper phase/angle state.
//!
//! The four coils are driven in a fixed sequence. Between solves only the phase
//! and accumulated shaft angle matter; the netlist supplies the current coil
//! levels and the wiring's step angle.

pub const PHASES: [[bool; 4]; 4] = [
    [true, false, true, false],
    [false, true, true, false],
    [false, true, false, true],
    [true, false, false, true],
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StepperState {
    pub angle: f64,
    pub phase: Option<usize>,
}

/// Advance one solve's worth of phase. An unrecognised coil pattern leaves the
/// state untouched; a jump of two phases is ambiguous and also leaves it alone.
pub fn step_stepper(state: StepperState, levels: [bool; 4], step_angle: f64) -> StepperState {
    let Some(phase) = PHASES.iter().position(|pattern| *pattern == levels) else {
        return state;
    };
    let Some(previous) = state.phase else {
        return StepperState {
            angle: state.angle,
            phase: Some(phase),
        };
    };
    if phase == previous {
        return state;
    }
    let delta = (phase + 4 - previous) % 4;
    let direction = match delta {
        1 => 1.0,
        3 => -1.0,
        _ => return state,
    };
    StepperState {
        angle: state.angle + direction * step_angle,
        phase: Some(phase),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_forward_through_the_phase_pattern() {
        let mut state = StepperState::default();
        state = step_stepper(state, PHASES[0], 1.8);
        assert_eq!(state.phase, Some(0));
        state = step_stepper(state, PHASES[1], 1.8);
        assert!((state.angle - 1.8).abs() < 1e-9);
        state = step_stepper(state, PHASES[2], 1.8);
        assert!((state.angle - 3.6).abs() < 1e-9);
    }

    #[test]
    fn steps_backward_and_ignores_unknown_or_ambiguous_patterns() {
        let mut state = StepperState {
            angle: 10.0,
            phase: Some(0),
        };
        state = step_stepper(state, PHASES[3], 1.8);
        assert!((state.angle - 8.2).abs() < 1e-9);
        let before = state;
        state = step_stepper(state, [false, false, false, false], 1.8);
        assert_eq!(state, before);
        state = step_stepper(state, PHASES[1], 1.8);
        assert_eq!(state, before);
    }
}
