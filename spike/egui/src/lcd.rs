//! The HD44780 side of the LCD1602: four data lines, an enable pulse, and the
//! handful of commands a sketch actually sends.
//!
//! This is the old app's `latchLcd`, in Rust. The display is driven in 4-bit
//! mode, so a byte arrives as two enable pulses: the high nibble first, then
//! the low one. Everything the sketch draws goes through here; the renderer
//! only reads `chars`.

/// A 16x2 display is 32 characters, row 0 first.
const CELLS: usize = 32;

#[derive(Clone, Debug)]
pub struct LcdState {
    /// The 32 character codes, row-major. Space is blank.
    pub chars: [u8; CELLS],
    /// Where the next character goes, in the controller's own address space.
    pub address: u8,
    pub entry_increment: bool,
    pub display_on: bool,
    pub cursor_on: bool,
    pub blink_on: bool,
    /// The high nibble waiting for its low half, and the register it came with.
    high: Option<u8>,
    rs: bool,
}

impl Default for LcdState {
    fn default() -> Self {
        Self {
            chars: [b' '; CELLS],
            address: 0,
            entry_increment: true,
            display_on: true,
            cursor_on: false,
            blink_on: false,
            high: None,
            rs: false,
        }
    }
}

impl LcdState {
    /// The two rows as text, for tests and for anything that wants to look.
    pub fn text(&self) -> [String; 2] {
        let row = |start: usize| {
            let row = &self.chars[start..start + 16];
            row.iter()
                .map(|code| char::from(*code))
                .collect::<String>()
                .trim_end()
                .to_string()
        };
        [row(0), row(16)]
    }
}

/// The cell an address writes to, or `None` for addresses the display lacks.
pub fn lcd_index(address: u8) -> Option<usize> {
    if address <= 0x0f {
        return Some(usize::from(address));
    }
    if (0x40..=0x4f).contains(&address) {
        return Some(16 + usize::from(address - 0x40));
    }
    None
}

/// Where the address counter moves after a write.
pub fn next_lcd_address(address: u8, increment: bool) -> u8 {
    if increment {
        if address >= 0x40 {
            if address >= 0x4f {
                0x00
            } else {
                address + 1
            }
        } else if address >= 0x0f {
            0x40
        } else {
            address + 1
        }
    } else if address >= 0x40 {
        if address <= 0x40 {
            0x0f
        } else {
            address - 1
        }
    } else if address == 0 {
        0x4f
    } else {
        address - 1
    }
}

/// Take one enable pulse. `data` is D4..D7; `rs` selects data over command.
///
/// Returns true when the pulse completed a byte.
pub fn latch(state: &mut LcdState, rs: bool, data: [bool; 4]) -> bool {
    let nibble = u8::from(data[0])
        | (u8::from(data[1]) << 1)
        | (u8::from(data[2]) << 2)
        | (u8::from(data[3]) << 3);
    let Some(high) = state.high.take() else {
        state.high = Some(nibble);
        state.rs = rs;
        return false;
    };
    let value = (high << 4) | nibble;
    if state.rs {
        if let Some(index) = lcd_index(state.address) {
            state.chars[index] = value;
        }
        state.address = next_lcd_address(state.address, state.entry_increment);
        return true;
    }
    // Commands, in the order the controller checks them.
    if value == 0x01 {
        state.chars = [b' '; CELLS];
        state.address = 0;
    } else if value == 0x02 {
        state.address = 0;
    } else if value & 0xfc == 0x04 {
        state.entry_increment = value & 0x02 != 0;
    } else if value & 0xf8 == 0x08 {
        state.display_on = value & 0x04 != 0;
        state.cursor_on = value & 0x02 != 0;
        state.blink_on = value & 0x01 != 0;
    } else if value & 0x80 != 0 {
        state.address = value & 0x7f;
    }
    true
}

/// Send one whole byte as two enable pulses, for tests and for anything that
/// already has the byte in hand.
pub fn write_byte(state: &mut LcdState, rs: bool, byte: u8) {
    let nibbles = |value: u8| {
        [
            value & 0x01 != 0,
            value & 0x02 != 0,
            value & 0x04 != 0,
            value & 0x08 != 0,
        ]
    };
    latch(state, rs, nibbles(byte >> 4));
    latch(state, rs, nibbles(byte & 0x0f));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_arrives_as_two_nibbles() {
        let mut state = LcdState::default();
        // RS high, 0x41 = 'A': high nibble 4, low nibble 1.
        assert!(!latch(&mut state, true, [false, false, true, false]));
        assert!(latch(&mut state, true, [true, false, false, false]));
        assert_eq!(state.text()[0], "A");
        assert_eq!(state.address, 1, "the counter moved on");
    }

    #[test]
    fn half_a_byte_writes_nothing() {
        let mut state = LcdState::default();
        assert!(!latch(&mut state, true, [true, false, true, false]));
        assert_eq!(state.text()[0], "", "a lone nibble is not a character");
        assert_eq!(state.address, 0);
    }

    #[test]
    fn clear_and_home_act_on_the_display() {
        let mut state = LcdState::default();
        write_byte(&mut state, true, b'x');
        write_byte(&mut state, false, 0x01);
        assert_eq!(state.text(), [String::new(), String::new()]);
        assert_eq!(state.address, 0);

        state.address = 0x45;
        write_byte(&mut state, false, 0x02);
        assert_eq!(state.address, 0, "home returns to the first cell");
    }

    #[test]
    fn the_address_command_places_the_next_character() {
        let mut state = LcdState::default();
        write_byte(&mut state, false, 0x80 | 0x40 | 3);
        write_byte(&mut state, true, b'?');
        assert_eq!(state.text()[1], "   ?", "row two, fourth cell");
    }

    #[test]
    fn entry_mode_decides_which_way_the_counter_walks() {
        let mut state = LcdState::default();
        write_byte(&mut state, false, 0x04); // entry mode, decrement
        assert!(!state.entry_increment);
        state.address = 2;
        write_byte(&mut state, true, b'a');
        assert_eq!(state.address, 1);
        assert_eq!(next_lcd_address(0, false), 0x4f);
        assert_eq!(next_lcd_address(0x40, false), 0x0f);
    }

    #[test]
    fn display_cursor_and_blink_bits_are_read() {
        let mut state = LcdState::default();
        write_byte(&mut state, false, 0x0f);
        assert!(state.display_on && state.cursor_on && state.blink_on);
        // 0x08 is display off, cursor off, blink off — the 0x04 bit is the
        // display itself, not "on by default".
        write_byte(&mut state, false, 0x08);
        assert!(!state.display_on && !state.cursor_on && !state.blink_on);
        // 0x0c is display on again with no cursor.
        write_byte(&mut state, false, 0x0c);
        assert!(state.display_on && !state.cursor_on);
    }

    #[test]
    fn both_rows_end_at_the_sixteenth_cell() {
        let mut state = LcdState::default();
        for index in 0..32u8 {
            let address = if index < 16 {
                index
            } else {
                0x40 + index - 16
            };
            write_byte(&mut state, false, 0x80 | address);
            write_byte(&mut state, true, b'0' + index % 10);
        }
        let [first, second] = state.text();
        assert_eq!(first.len(), 16);
        assert_eq!(second.len(), 16);
    }
}
