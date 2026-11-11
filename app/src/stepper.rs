//! The stepper's four coils, and how moving between phases turns its shaft.
//!
//! Ported from v0.2: a driver energises two coils at a time in a four-step
//! sequence, and each change is one step of `360 / steps` degrees. The netlist
//! reports the wiring and the step angle; this is the part that remembers where
//! the shaft is, which the core deliberately does not.

/// The sequence a stepper driver writes: two coils energised at a time.
pub const PHASES: [[bool; 4]; 4] = [
    [true, false, true, false],
    [false, true, true, false],
    [false, true, false, true],
    [true, false, false, true],
];

/// Which phase those four coil levels are, if they are one at all.
pub fn phase_of(levels: [bool; 4]) -> Option<usize> {
    PHASES.iter().position(|pattern| *pattern == levels)
}

/// How far the shaft turns going from one phase to the next.
///
/// One step forward, one step back — and nothing for a skipped phase or a jump
/// to the opposite pattern, which is not a direction the driver wrote.
pub fn step_degrees(previous: usize, phase: usize, per_step: f64) -> f64 {
    match (phase + 4 - previous) % 4 {
        1 => per_step,
        3 => -per_step,
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_phases_are_recognised() {
        for (index, pattern) in PHASES.iter().enumerate() {
            assert_eq!(phase_of(*pattern), Some(index));
        }
        assert_eq!(phase_of([true, true, true, true]), None, "not a phase");
        assert_eq!(phase_of([false; 4]), None, "nor is everything off");
    }

    #[test]
    fn one_step_forward_and_back_is_a_step_each_way() {
        assert_eq!(step_degrees(0, 1, 1.8), 1.8);
        assert_eq!(step_degrees(1, 2, 1.8), 1.8);
        assert_eq!(step_degrees(0, 3, 1.8), -1.8, "backwards wraps the other way");
        assert_eq!(step_degrees(3, 0, 1.8), 1.8);
    }

    #[test]
    fn a_repeat_or_a_skip_turns_nothing() {
        assert_eq!(step_degrees(2, 2, 1.8), 0.0, "same phase");
        assert_eq!(step_degrees(0, 2, 1.8), 0.0, "opposite phase");
    }
}
