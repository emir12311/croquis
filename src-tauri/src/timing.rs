//! Turn a pin's edge stream into PWM timing.
//!
//! `simavr` notifies the engine on every pin transition. For a button that is two
//! events and then silence; for `analogWrite` it is the PWM waveform. Forwarding
//! every edge to the UI would flood it and still not say what an LED should look
//! like, so this keeps the latest high pulse and the period between two rising
//! edges, in simulated CPU cycles.
//!
//! This module is deliberately free of FFI: the engine's pin callback calls
//! `edge`, and a 20 ms tick will call `timing` and emit one `PinTiming` event per
//! pin. That keeps the measurement testable without a simulator.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// High time as thousandths of the period (0..=1000).
    pub duty_per_mille: u16,
    /// High time in simulated CPU cycles.
    pub high_cycles: u64,
    /// Time between the last two rising edges, in simulated CPU cycles.
    pub period_cycles: u64,
}

impl Timing {
    /// Pulse width in microseconds. The engine uses 16 MHz on the Uno.
    pub fn high_micros(&self, f_cpu: u64) -> u64 {
        if f_cpu == 0 {
            return 0;
        }
        ((self.high_cycles as u128 * 1_000_000) / f_cpu as u128) as u64
    }

    pub fn period_micros(&self, f_cpu: u64) -> u64 {
        if f_cpu == 0 {
            return 0;
        }
        ((self.period_cycles as u128 * 1_000_000) / f_cpu as u128) as u64
    }
}

/// The latest timing seen on one pin.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PinTiming {
    rise: Option<u64>,
    last_edge: Option<u64>,
    /// The last level seen, so a repeated notification for the same value does not
    /// restart a pulse. simavr can deliver more than one notification per edge.
    level: Option<bool>,
    high_cycles: u64,
    period_cycles: u64,
}

/// Ignore transitions closer than this to the previous one. simavr emits
/// zero-width and ~6 µs glitches around a timer compare edge; at 16 MHz, 40
/// cycles is 2.5 µs, below any real PWM pulse we care about.
const MIN_EDGE_GAP: u64 = 40;

impl PinTiming {
    /// Feed one edge. `cycle` is `sim_shim_cycle` at the transition.
    pub fn edge(&mut self, cycle: u64, high: bool) {
        if self.level == Some(high) {
            return;
        }
        if self
            .last_edge
            .is_some_and(|last| cycle.saturating_sub(last) < MIN_EDGE_GAP)
        {
            return;
        }
        self.level = Some(high);
        self.last_edge = Some(cycle);
        if high {
            if let Some(previous_rise) = self.rise {
                let period = cycle.saturating_sub(previous_rise);
                if period > 0 {
                    self.period_cycles = period;
                }
            }
            self.rise = Some(cycle);
        } else if let Some(rise) = self.rise {
            self.high_cycles = cycle.saturating_sub(rise);
        }
    }

    /// The latest complete period, or `None` until two rising edges have been
    /// seen. A slow button never produces a period, which is what keeps it out of
    /// the timing stream.
    pub fn timing(&self) -> Option<Timing> {
        if self.period_cycles == 0 || self.high_cycles > self.period_cycles {
            return None;
        }
        let duty = (self.high_cycles as u128 * 1000) / self.period_cycles as u128;
        Some(Timing {
            duty_per_mille: duty.min(1000) as u16,
            high_cycles: self.high_cycles,
            period_cycles: self.period_cycles,
        })
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The cycle of the most recent edge, for deciding whether a timing is stale.
    pub fn last_edge(&self) -> Option<u64> {
        self.last_edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F_CPU: u64 = 16_000_000;
    const ONE_MS: u64 = F_CPU / 1000;
    const ONE_US: u64 = F_CPU / 1_000_000;

    #[test]
    fn measures_half_duty_at_one_khz() {
        let mut pin = PinTiming::default();
        pin.edge(0, true);
        pin.edge(ONE_MS / 2, false);
        pin.edge(ONE_MS, true);

        let timing = pin.timing().expect("a full period was seen");
        assert_eq!(timing.duty_per_mille, 500);
        assert_eq!(timing.high_cycles, ONE_MS / 2);
        assert_eq!(timing.period_cycles, ONE_MS);
        assert_eq!(timing.high_micros(F_CPU), 500);
        assert_eq!(timing.period_micros(F_CPU), 1000);
    }

    #[test]
    fn measures_quarter_duty() {
        let mut pin = PinTiming::default();
        let period = 32_000; // ~500 Hz
        pin.edge(0, true);
        pin.edge(8_000, false);
        pin.edge(period, true);

        let timing = pin.timing().unwrap();
        assert_eq!(timing.duty_per_mille, 250);
        assert_eq!(timing.high_micros(F_CPU), 500);
        assert_eq!(timing.period_micros(F_CPU), 2_000);
    }

    #[test]
    fn measures_a_servo_pulse() {
        let mut pin = PinTiming::default();
        // 1.5 ms high every 20 ms — the middle of an SG90's travel.
        pin.edge(0, true);
        pin.edge(1_500 * ONE_US, false);
        pin.edge(20_000 * ONE_US, true);

        let timing = pin.timing().unwrap();
        assert_eq!(timing.high_micros(F_CPU), 1_500);
        assert_eq!(timing.period_micros(F_CPU), 20_000);
        assert_eq!(timing.duty_per_mille, 75);
    }

    #[test]
    fn waits_for_a_second_rising_edge() {
        let mut pin = PinTiming::default();
        pin.edge(0, true);
        pin.edge(ONE_MS, false);
        assert!(pin.timing().is_none(), "a pulse is not a period yet");

        pin.edge(2 * ONE_MS, true);
        assert!(pin.timing().is_some(), "the second rise completes the period");
    }

    #[test]
    fn ignores_a_held_level_and_starts_cleanly_on_a_low_edge() {
        let mut pin = PinTiming::default();
        pin.edge(0, false);
        assert!(pin.timing().is_none());

        // Falling first is a pin that started low; the first rise begins the pulse.
        pin.edge(1_000, true);
        pin.edge(2_000, false);
        pin.edge(3_000, true);
        let timing = pin.timing().unwrap();
        assert_eq!(timing.high_cycles, 1_000);
        assert_eq!(timing.period_cycles, 2_000);
        assert_eq!(timing.duty_per_mille, 500);
    }

    #[test]
    fn ignores_repeated_levels() {
        let mut pin = PinTiming::default();
        pin.edge(0, true);
        pin.edge(100, true); // a duplicate notification, not a new pulse
        pin.edge(ONE_MS / 2, false);
        pin.edge(ONE_MS, true);

        let timing = pin.timing().unwrap();
        assert_eq!(timing.high_cycles, ONE_MS / 2);
        assert_eq!(timing.period_cycles, ONE_MS);
        assert_eq!(timing.duty_per_mille, 500);
    }

    #[test]
    fn ignores_contact_glitches() {
        let mut pin = PinTiming::default();
        pin.edge(0, true);
        pin.edge(ONE_MS / 2, false);
        pin.edge(ONE_MS / 2, true); // zero-width glitch at the real edge
        pin.edge(ONE_MS / 2 + 100, false); // and its short low
        pin.edge(ONE_MS, true);

        let timing = pin.timing().unwrap();
        assert_eq!(timing.high_cycles, ONE_MS / 2);
        assert_eq!(timing.period_cycles, ONE_MS);
        assert_eq!(timing.duty_per_mille, 500);
    }

    #[test]
    fn reset_forgets_the_previous_pin_owner() {
        let mut pin = PinTiming::default();
        pin.edge(0, true);
        pin.edge(ONE_MS / 2, false);
        pin.edge(ONE_MS, true);
        assert!(pin.timing().is_some());

        pin.reset();
        assert!(pin.timing().is_none());
    }
}
