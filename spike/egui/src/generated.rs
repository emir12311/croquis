//! Parts Croquis draws itself, generated from a spec rather than loaded as SVG.
//!
//! `@wokwi/elements` has no breadboard and no logic parts, so v0.2 drew both
//! from code (`src/lib/breadboard.ts`, `src/lib/hc595-element.ts`). This is that
//! code, ported: one spec produces the drawing *and* the pin list, so a wire
//! cannot end up half a hole off the copper it is meant to touch.

use croquis_core::catalogue::PartKind;

/// The hole spacing: 2.54 mm at 96 dpi, and seven of them make the 67 px leg
/// spacing of the pushbutton — the worst-fitting part — land exactly.
const PITCH: f32 = 67.0 / 7.0;
const ROWS_PER_SIDE: usize = 5;
const ROWS_TOP: [char; 5] = ['a', 'b', 'c', 'd', 'e'];
const ROWS_BOTTOM: [char; 5] = ['f', 'g', 'h', 'i', 'j'];
/// Holes in each power-rail segment; a full board splits every rail in two.
const RAIL_HOLES: usize = 25;
/// Columns skipped where a full board's rails break in the middle.
const RAIL_BREAK: usize = 3;

const PAD: f32 = 16.0;
const RAIL_GAP: f32 = PITCH;
/// The centre trench, one pitch wide, so a two-row part can straddle it.
const TRENCH: f32 = PITCH;
const HOLE_R: f32 = 2.1;

const BODY: &str = "#f2f1ec";
const BODY_EDGE: &str = "#c9c6bc";
const TRENCH_FILL: &str = "#e4e2d9";
const HOLE: &str = "#3a3a3c";
const PLUS: &str = "#d0483f";
const MINUS: &str = "#3f6fd0";
const INK: &str = "#8d8a80";

/// The 74HC595's package, in its own pixels.
const HC595_WIDTH: f32 = 76.0;
const HC595_HEIGHT: f32 = 38.0;
const HC595_PIN_X: [f32; 8] = [4.8, 14.4, 24.0, 33.6, 43.2, 52.8, 62.4, 72.0];
const HC595_BOTTOM: [&str; 8] = ["Q1", "Q2", "Q3", "Q4", "Q5", "Q6", "Q7", "GND"];
const HC595_TOP: [&str; 8] = ["VCC", "Q0", "DS", "OE", "STCP", "SHCP", "MR", "Q7S"];

/// A generated part: its size, its pins in its own pixels, and its drawing.
pub struct Built {
    pub width: f32,
    pub height: f32,
    pub pins: Vec<(String, f32, f32)>,
    pub svg: String,
}

/// The generator for a kind the Wokwi assets have nothing for.
pub fn for_kind(kind: PartKind) -> Option<Built> {
    match kind {
        PartKind::Breadboard => Some(breadboard(30, 1)),
        PartKind::BreadboardFull => Some(breadboard(63, 2)),
        PartKind::ShiftRegister74hc595 => Some(hc595()),
        _ => None,
    }
}

/// A breadboard: a half board is 30 columns and one rail segment each side, a
/// full board 63 columns with the rails split in the middle as a real one is.
fn breadboard(columns: usize, rail_segments: usize) -> Built {
    let column_x = |column: usize| PAD + (column as f32 - 1.0) * PITCH;
    let rail_top = PAD + 6.0;
    let label_top = rail_top + RAIL_GAP + 11.0;
    let grid_top = label_top + 6.0;
    let row_y_top = |index: usize| grid_top + index as f32 * PITCH;
    let trench_top = row_y_top(ROWS_PER_SIDE - 1) + PITCH * 0.5;
    let row_y_bottom = |index: usize| trench_top + TRENCH + PITCH * 0.5 + index as f32 * PITCH;

    let mut pins: Vec<(String, f32, f32)> = Vec::new();
    for row in 0..ROWS_PER_SIDE {
        for column in 1..=columns {
            pins.push((
                format!("{}{column}", ROWS_TOP[row]),
                column_x(column),
                row_y_top(row),
            ));
            pins.push((
                format!("{}{column}", ROWS_BOTTOM[row]),
                column_x(column),
                row_y_bottom(row),
            ));
        }
    }

    let span = RAIL_HOLES * rail_segments + RAIL_BREAK * rail_segments.saturating_sub(1);
    let start_column = (columns as f32 - span as f32) / 2.0;
    let first_column = start_column.floor() as usize + 1;
    let segment_start = |segment: usize| first_column + segment * (RAIL_HOLES + RAIL_BREAK);
    let rail_bottom = row_y_bottom(ROWS_PER_SIDE - 1) + 15.0;
    let rail_rows = [
        ("tp", rail_top, PLUS),
        ("tn", rail_top + RAIL_GAP, MINUS),
        ("bp", rail_bottom, PLUS),
        ("bn", rail_bottom + RAIL_GAP, MINUS),
    ];

    struct Stripe {
        from: f32,
        to: f32,
        y: f32,
        colour: &'static str,
    }
    let mut stripes: Vec<Stripe> = Vec::new();
    for (name, y, colour) in rail_rows {
        for segment in 0..rail_segments {
            let suffix = if rail_segments > 1 {
                char::from(b'A' + segment as u8).to_string()
            } else {
                String::new()
            };
            let first = column_x(segment_start(segment));
            let last = column_x(segment_start(segment) + RAIL_HOLES - 1);
            stripes.push(Stripe {
                from: first - PITCH * 0.6,
                to: last + PITCH * 0.6,
                y,
                colour,
            });
            for hole in 1..=RAIL_HOLES {
                pins.push((
                    format!("{name}{suffix}{hole}"),
                    column_x(segment_start(segment) + hole - 1),
                    y,
                ));
            }
        }
    }

    let width = PAD * 2.0 + (columns as f32 - 1.0) * PITCH;
    let height = pins
        .iter()
        .map(|(_, _, y)| *y)
        .fold(0.0_f32, f32::max)
        + PAD;

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.1}\" height=\"{height:.1}\" viewBox=\"0 0 {width:.1} {height:.1}\">"
    ));
    svg.push_str(&format!(
        "<rect x=\"0.5\" y=\"0.5\" width=\"{:.1}\" height=\"{:.1}\" rx=\"5\" fill=\"{BODY}\" stroke=\"{BODY_EDGE}\"/>",
        width - 1.0,
        height - 1.0
    ));
    svg.push_str(&format!(
        "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{TRENCH}\" fill=\"{TRENCH_FILL}\"/>",
        PAD - 6.0,
        row_y_top(ROWS_PER_SIDE - 1) + PITCH * 0.5,
        (columns as f32 - 1.0) * PITCH + 12.0
    ));
    for stripe in &stripes {
        svg.push_str(&format!(
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"{}\" stroke-width=\"{:.1}\" stroke-linecap=\"round\"/>",
            stripe.from, stripe.y, stripe.to, stripe.y, stripe.colour, PITCH * 0.34
        ));
    }

    // Column numbers: 1, every fifth, and the last — unless it would crowd the
    // label before it, which a 63-column board would otherwise do.
    let mut wanted = vec![1usize];
    let mut column = 5;
    while column <= columns {
        wanted.push(column);
        column += 5;
    }
    if columns % 5 != 0 {
        wanted.push(columns);
    }
    let mut previous: Option<usize> = None;
    for column in wanted {
        if previous.is_some_and(|last| column - last <= 3) {
            continue;
        }
        previous = Some(column);
        svg.push_str(&label(
            column_x(column),
            row_y_top(0) - 5.0,
            &column.to_string(),
        ));
    }
    for row in 0..ROWS_PER_SIDE {
        svg.push_str(&label(
            PAD - 9.0,
            row_y_top(row) + 2.5,
            &ROWS_TOP[row].to_string(),
        ));
        svg.push_str(&label(
            PAD - 9.0,
            row_y_bottom(row) + 2.5,
            &ROWS_BOTTOM[row].to_string(),
        ));
    }
    for (_, x, y) in &pins {
        svg.push_str(&format!(
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"{HOLE_R}\" fill=\"{HOLE}\"/>"
        ));
    }
    svg.push_str("</svg>");

    Built {
        width,
        height,
        pins,
        svg,
    }
}

fn label(x: f32, y: f32, text: &str) -> String {
    format!(
        "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"sans-serif\" font-size=\"7\" fill=\"{INK}\" text-anchor=\"middle\">{text}</text>"
    )
}

/// A 74HC595 in a DIP-16 package: pin 1 bottom-left, pin 16 top-left.
fn hc595() -> Built {
    let mut pins: Vec<(String, f32, f32)> = Vec::new();
    for (index, x) in HC595_PIN_X.iter().enumerate() {
        pins.push((
            HC595_BOTTOM[index].to_string(),
            *x,
            HC595_HEIGHT,
        ));
    }
    for (index, x) in HC595_PIN_X.iter().enumerate() {
        pins.push((HC595_TOP[index].to_string(), *x, 0.0));
    }

    let mut svg = String::new();
    svg.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{HC595_WIDTH}\" height=\"{HC595_HEIGHT}\" viewBox=\"0 0 {HC595_WIDTH} {HC595_HEIGHT}\">"
    ));
    for x in HC595_PIN_X {
        for y in [0.0, 30.0] {
            svg.push_str(&format!(
                "<rect x=\"{:.1}\" y=\"{y}\" width=\"5.5\" height=\"8\" fill=\"#c8c8c8\" stroke=\"#8f8f8f\" stroke-width=\"0.6\"/>",
                x - 2.75
            ));
        }
    }
    svg.push_str(
        "<rect x=\"4\" y=\"6\" width=\"68\" height=\"26\" rx=\"1.5\" fill=\"#1c1c1c\" stroke=\"#000\" stroke-width=\"0.8\"/>",
    );
    svg.push_str("<circle cx=\"8.6\" cy=\"29.4\" r=\"1.2\" fill=\"#8a8a8a\"/>");
    svg.push_str(
        "<text x=\"38\" y=\"17.4\" text-anchor=\"middle\" font-family=\"monospace\" font-size=\"9.5\" font-weight=\"700\" fill=\"#d8cf3f\">74HC</text>",
    );
    svg.push_str(
        "<text x=\"38\" y=\"27\" text-anchor=\"middle\" font-family=\"monospace\" font-size=\"9.5\" font-weight=\"700\" fill=\"#d8cf3f\">595</text>",
    );
    svg.push_str("</svg>");

    Built {
        width: HC595_WIDTH,
        height: HC595_HEIGHT,
        pins,
        svg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_half_breadboard_has_four_hundred_tie_points() {
        let board = breadboard(30, 1);
        // 30 columns x 5 rows x 2 banks, plus four rails of 25 holes.
        assert_eq!(board.pins.len(), 30 * 5 * 2 + 4 * 25);
        assert_eq!(board.pins.len(), 400);
    }

    #[test]
    fn a_full_breadboard_has_eight_hundred_and_thirty() {
        let board = breadboard(63, 2);
        // 63 x 5 x 2, plus four rails split in two, 25 holes each.
        assert_eq!(board.pins.len(), 63 * 5 * 2 + 4 * 2 * 25);
        assert_eq!(board.pins.len(), 830);
    }

    #[test]
    fn holes_sit_one_pitch_apart_and_the_banks_are_separated() {
        let board = breadboard(30, 1);
        let at = |name: &str| {
            board
                .pins
                .iter()
                .find(|(pin, _, _)| pin == name)
                .map(|(_, x, y)| (*x, *y))
                .unwrap_or_else(|| panic!("no pin {name}"))
        };
        let (a1x, a1y) = at("a1");
        let (a2x, a2y) = at("a2");
        assert!((a2x - a1x - PITCH).abs() < 1e-4, "columns are one pitch apart");
        assert!((a2y - a1y).abs() < 1e-4, "and level with each other");

        let (_, e1y) = at("e1");
        let (_, f1y) = at("f1");
        assert!(
            f1y - e1y > PITCH * 1.5,
            "the trench keeps the two banks apart, got {}",
            f1y - e1y
        );

        // A full board splits every rail in two, so the segments get a letter and
        // the two of them are separate nets even though they share a line. A
        // half board has no split, hence no letter.
        let full = breadboard(63, 2);
        let rail = |name: &str| {
            full.pins
                .iter()
                .find(|(pin, _, _)| pin == name)
                .map(|(_, x, y)| (*x, *y))
                .unwrap_or_else(|| panic!("no pin {name}"))
        };
        let (tp_a1x, tp_a1y) = rail("tpA1");
        let (tp_b1x, tp_b1y) = rail("tpB1");
        let (_, tn_a1y) = rail("tnA1");
        assert!((tp_b1y - tp_a1y).abs() < 1e-4, "the two + segments are level");
        assert!(tp_b1x > tp_a1x, "and the second one starts further right");
        assert!(tn_a1y > tp_a1y, "the − rail is its own line below the +");
    }

    #[test]
    fn the_shift_register_has_sixteen_pins_on_two_sides() {
        let part = hc595();
        assert_eq!(part.pins.len(), 16);
        assert_eq!((part.width, part.height), (76.0, 38.0));
        let top = part
            .pins
            .iter()
            .filter(|(_, _, y)| *y == 0.0)
            .count();
        let bottom = part
            .pins
            .iter()
            .filter(|(_, _, y)| *y == 38.0)
            .count();
        assert_eq!((top, bottom), (8, 8));
        assert!(part.svg.contains("74HC") && part.svg.contains("595"));
    }
}
