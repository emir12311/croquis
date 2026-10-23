//! The 74HC595 shift register as a pure state machine.
//!
//! The chip remembers the bits clocked into it, so its state survives between
//! netlist solves. The transition rules live here; the netlist supplies wiring
//! and AVR levels.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShiftSignal {
    Ds,
    Shcp,
    Stcp,
    Mr,
    Oe,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShiftLevels {
    pub ds: bool,
    pub shcp: bool,
    pub stcp: bool,
    pub mr: bool,
    pub oe: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShiftWiring {
    pub powered: bool,
    /// Board pin key (`D3`) driving each input, when one reaches it.
    pub pins: HashMap<ShiftSignal, String>,
    /// Levels from the rails when no board pin drives the input.
    pub levels: ShiftLevels,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShiftState {
    /// Bits currently clocked in, LSB = Q0.
    pub shift: u8,
    /// Bits latched onto Q0…Q7, LSB = Q0.
    pub latch: u8,
    /// Input levels at the last step, for edge detection.
    pub levels: ShiftLevels,
}

pub fn level_of(signal: ShiftSignal, wiring: &ShiftWiring, avr: &HashMap<String, bool>) -> bool {
    wiring
        .pins
        .get(&signal)
        .map(|key| avr.get(key).copied().unwrap_or(false))
        .unwrap_or_else(|| match signal {
            ShiftSignal::Ds => wiring.levels.ds,
            ShiftSignal::Shcp => wiring.levels.shcp,
            ShiftSignal::Stcp => wiring.levels.stcp,
            ShiftSignal::Mr => wiring.levels.mr,
            ShiftSignal::Oe => wiring.levels.oe,
        })
}

/// Advance one solve: shift on rising SHCP, latch on rising STCP, and clear the
/// shift register while MR is low. MR deliberately leaves the latch untouched.
pub fn step_shift_register(
    state: ShiftState,
    wiring: &ShiftWiring,
    avr: &HashMap<String, bool>,
) -> ShiftState {
    let levels = ShiftLevels {
        ds: level_of(ShiftSignal::Ds, wiring, avr),
        shcp: level_of(ShiftSignal::Shcp, wiring, avr),
        stcp: level_of(ShiftSignal::Stcp, wiring, avr),
        mr: level_of(ShiftSignal::Mr, wiring, avr),
        oe: level_of(ShiftSignal::Oe, wiring, avr),
    };
    if !wiring.powered {
        return ShiftState {
            shift: 0,
            latch: 0,
            levels,
        };
    }
    let mut shift = state.shift;
    let mut latch = state.latch;
    if !levels.mr {
        shift = 0;
    } else if levels.shcp && !state.levels.shcp {
        shift = ((shift << 1) | u8::from(levels.ds)) & 0xff;
    }
    if levels.stcp && !state.levels.stcp {
        latch = shift;
    }
    ShiftState {
        shift,
        latch,
        levels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wiring(levels: ShiftLevels) -> ShiftWiring {
        ShiftWiring {
            powered: true,
            pins: HashMap::new(),
            levels,
        }
    }

    #[test]
    fn shifts_on_a_rising_clock_and_latches_on_a_rising_strobe() {
        let mut state = ShiftState::default();
        let low = wiring(ShiftLevels::default());
        state = step_shift_register(state, &low, &HashMap::new());

        let high = wiring(ShiftLevels {
            ds: true,
            shcp: true,
            mr: true,
            ..ShiftLevels::default()
        });
        state = step_shift_register(state, &high, &HashMap::new());
        assert_eq!(state.shift, 1);
        assert_eq!(state.latch, 0);

        let strobe = wiring(ShiftLevels {
            ds: true,
            shcp: true,
            stcp: true,
            mr: true,
            ..ShiftLevels::default()
        });
        state = step_shift_register(state, &strobe, &HashMap::new());
        assert_eq!(state.latch, 1);
    }

    #[test]
    fn master_reset_clears_shift_but_not_latch() {
        let state = ShiftState {
            shift: 0b1010_1010,
            latch: 0b0101_0101,
            levels: ShiftLevels {
                mr: true,
                ..ShiftLevels::default()
            },
        };
        let reset = wiring(ShiftLevels {
            mr: false,
            ..ShiftLevels::default()
        });
        let next = step_shift_register(state, &reset, &HashMap::new());
        assert_eq!(next.shift, 0);
        assert_eq!(next.latch, 0b0101_0101);
    }

    #[test]
    fn unpowered_chip_forgets_everything() {
        let state = ShiftState {
            shift: 7,
            latch: 9,
            levels: ShiftLevels::default(),
        };
        let mut wiring = wiring(ShiftLevels::default());
        wiring.powered = false;
        let next = step_shift_register(state, &wiring, &HashMap::new());
        assert_eq!(next.shift, 0);
        assert_eq!(next.latch, 0);
    }

    #[test]
    fn board_pins_win_over_rail_levels() {
        let mut wiring = wiring(ShiftLevels {
            ds: false,
            ..ShiftLevels::default()
        });
        wiring.pins.insert(ShiftSignal::Ds, "D7".into());
        let avr = HashMap::from([("D7".into(), true)]);
        assert!(level_of(ShiftSignal::Ds, &wiring, &avr));
    }
}
