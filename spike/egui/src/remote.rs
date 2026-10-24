//! The IR remote: where its buttons are, and the NEC frame each one sends.
//!
//! Taken from the element itself (`ir-remote-element.js`): the button table with
//! its NEC command bytes, the 151 x 316 drawing they sit in, and the frame
//! timing that `pulse_script` plays down the receiver's pin.

use eframe::egui::{Pos2, Vec2};

use crate::scene::Part;

/// The element's drawing.
pub const VIEW: Vec2 = Vec2::new(151.0, 316.0);
/// A button is a circle of this radius, centred on its `data-btn` position.
const BUTTON_RADIUS: f32 = 17.6;

/// id, NEC command, x, y — the element's own table, in its own pixels.
pub const BUTTONS: [(&str, u8, f32, f32); 20] = [
    ("power", 0xa2, 29.6, 37.9),
    ("menu", 0xe2, 121.4, 37.9),
    ("test", 0x22, 29.6, 75.2),
    ("plus", 0x02, 75.5, 75.2),
    ("back", 0xc2, 121.4, 75.2),
    ("prev", 0xe0, 29.6, 113.0),
    ("play", 0xa8, 75.5, 113.0),
    ("next", 0x90, 121.4, 113.0),
    ("0", 0x68, 29.6, 152.0),
    ("minus", 0x98, 75.5, 152.0),
    ("c", 0xb0, 121.4, 152.0),
    ("1", 0x30, 29.6, 190.0),
    ("2", 0x18, 75.5, 190.0),
    ("3", 0x7a, 121.4, 190.0),
    ("4", 0x10, 29.6, 228.0),
    ("5", 0x38, 75.5, 228.0),
    ("6", 0x5a, 121.4, 228.0),
    ("7", 0x42, 29.6, 266.0),
    ("8", 0x4a, 75.5, 266.0),
    ("9", 0x52, 121.4, 266.0),
];

/// The button under a canvas point, as its id and NEC command.
pub fn button_at(part: &Part, world: Pos2) -> Option<(&'static str, u8)> {
    let bounds = part.bounds();
    let local = (world - bounds.min) / (bounds.width() / VIEW.x);
    BUTTONS
        .iter()
        .find(|(_, _, x, y)| {
            let dx = local.x - x;
            let dy = local.y - y;
            dx * dx + dy * dy <= BUTTON_RADIUS * BUTTON_RADIUS
        })
        .map(|(id, command, _, _)| (*id, *command))
}

/// The button a part is showing at, for drawing the press.
pub fn button_centre(part: &Part, id: &str) -> Option<Pos2> {
    let bounds = part.bounds();
    let unit = bounds.width() / VIEW.x;
    BUTTONS
        .iter()
        .find(|(button, _, _, _)| *button == id)
        .map(|(_, _, x, y)| bounds.min + Vec2::new(*x, *y) * unit)
}

/// One NEC frame as burst lengths in microseconds, carrier stripped.
///
/// `pulse_script` plays these down the receiver's pin, which is the same thing
/// the element does when you press a button: 9 ms of burst, 4.5 ms of gap, then
/// address, its complement, command and its complement, least significant bit
/// first — a 560 µs burst for a zero and 1690 µs for a one.
pub fn nec_durations(command: u8) -> Vec<u64> {
    const ADDRESS: u8 = 0;
    let bytes = [ADDRESS, !ADDRESS, command, !command];
    let mut durations = vec![9000, 4500];
    for byte in bytes {
        for bit in 0..8 {
            durations.push(560);
            durations.push(if (byte >> bit) & 1 == 1 { 1690 } else { 560 });
        }
    }
    durations.push(560);
    durations
}

#[cfg(test)]
mod tests {
    use super::*;
    use croquis_core::catalogue::PartKind;

    fn remote() -> Part {
        crate::scene::part_from_kind(PartKind::IrRemote, Pos2::ZERO, "ir".into())
            .expect("ir remote")
    }

    #[test]
    fn a_frame_is_the_header_four_bytes_and_a_trailer() {
        let frame = nec_durations(0xa2);
        assert_eq!(frame.len(), 2 + 4 * 8 * 2 + 1);
        assert_eq!(&frame[..2], &[9000, 4500], "the header");
        assert_eq!(*frame.last().expect("trailer"), 560, "and the stop bit");
    }

    #[test]
    fn bits_go_out_least_significant_first() {
        // Command 0x01: the first bit is a one, the next seven are zeros.
        let frame = nec_durations(0x01);
        // Skip the header, then the address byte (all zeros, 8 pairs), then the
        // inverted address (all ones), landing on the command byte.
        let command_at = 2 + 8 * 2 + 8 * 2;
        assert_eq!(frame[command_at], 560, "a burst, then its space");
        assert_eq!(frame[command_at + 1], 1690, "bit 0 is set");
        assert_eq!(frame[command_at + 2], 560);
        assert_eq!(frame[command_at + 3], 560, "bit 1 is not");
    }

    #[test]
    fn the_power_button_sends_its_own_code() {
        let remote = remote();
        let bounds = remote.bounds();
        let unit = bounds.width() / VIEW.x;
        // The element puts power at (29.6, 37.9).
        let at = bounds.min + Vec2::new(29.6, 37.9) * unit;
        assert_eq!(button_at(&remote, at), Some(("power", 0xa2)));
        assert_eq!(button_centre(&remote, "power"), Some(at));
        // Between the power and test buttons there is nothing to press.
        let gap = bounds.min + Vec2::new(29.6, 56.0) * unit;
        assert_eq!(button_at(&remote, gap), None);
    }

    #[test]
    fn every_button_has_a_code_and_a_place() {
        for (id, command, x, y) in BUTTONS {
            assert!(!id.is_empty());
            assert!(command != 0, "{id} has no command");
            assert!((0.0..=VIEW.x).contains(&x) && (0.0..=VIEW.y).contains(&y));
        }
        // The ids the element offers, one each.
        let mut ids: Vec<&str> = BUTTONS.iter().map(|(id, _, _, _)| *id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), BUTTONS.len());
    }
}
