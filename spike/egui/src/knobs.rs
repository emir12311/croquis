//! Turning the two potentiometers by hand: the angle a value shows, and the
//! value a pointer asks for.
//!
//! The potentiometer's knob is a CSS angle in the element's own 20 x 20 drawing;
//! the slide pot's tip is a translate in its 55 x 29 one. Both mappings are the
//! elements': a knob sweeps -135° to +135° over its range, a tip runs 30 units
//! from -15, inside the drawing's 5,5 offset.

use eframe::egui::{Pos2, Vec2};

use crate::scene::Part;

const POT_VIEW: Vec2 = Vec2::new(20.0, 20.0);
/// Where the shaft sits in that drawing.
const POT_CENTRE: Vec2 = Vec2::new(9.91, 8.18);
/// The element's `startDegree` and `endDegree` are -135 and 135.
const POT_SWEEP: f64 = 135.0;
/// The potentiometer's `max`.
const POT_MAX: f64 = 1023.0;

/// The knob's own radius, so a grab on it turns rather than moves.
const POT_KNOB_RADIUS: f32 = 7.4;

const SLIDE_VIEW: Vec2 = Vec2::new(55.0, 29.0);
/// The slide pot's handle, in the drawing's own units: a 12 x 19 rect that
/// starts at 16.5 across and spans the full height.
const SLIDE_HANDLE_X: f32 = 16.5;
const SLIDE_HANDLE_WIDTH: f32 = 12.0;
const SLIDE_HANDLE_HEIGHT: f32 = 19.0;

/// The joystick's stick, in its 27.2 x 31.8 drawing.
const STICK_VIEW: Vec2 = Vec2::new(27.2, 31.8);
const STICK_CENTRE: Vec2 = Vec2::new(13.6, 13.6);
const STICK_RADIUS: f32 = 10.6;
/// The tip group's own offset inside the drawing, and its travel.
const SLIDE_ORIGIN: f32 = 5.0;
const SLIDE_BASE: f32 = -15.0;
const SLIDE_TRAVEL: f32 = 30.0;
const SLIDE_MAX: f64 = 100.0;

/// Whether a grab landed on the potentiometer's knob rather than its body.
///
/// A press on the body moves the part; only the knob itself turns.
pub fn on_pot_knob(part: &Part, world: Pos2) -> bool {
    let bounds = part.bounds();
    let unit = bounds.width() / POT_VIEW.x;
    let centre = bounds.min + POT_CENTRE * unit;
    world.distance(centre) <= POT_KNOB_RADIUS * unit
}

/// Whether a grab landed on the slide pot's handle rather than its body.
pub fn on_slide_handle(part: &Part, world: Pos2) -> bool {
    let bounds = part.bounds();
    let unit = bounds.width() / SLIDE_VIEW.x;
    let local = (world - bounds.min) / unit;
    let offset = slide_offset(attr_value(part));
    let left = SLIDE_ORIGIN + SLIDE_HANDLE_X + offset;
    let right = left + SLIDE_HANDLE_WIDTH;
    let top = SLIDE_ORIGIN;
    local.x >= left && local.x <= right && local.y >= top && local.y <= top + SLIDE_HANDLE_HEIGHT
}

/// Whether a grab landed on the joystick's stick rather than its body.
pub fn on_stick(part: &Part, world: Pos2) -> bool {
    let bounds = part.bounds();
    let unit = bounds.width() / STICK_VIEW.x;
    let centre = bounds.min + STICK_CENTRE * unit;
    world.distance(centre) <= STICK_RADIUS * unit
}

/// The part's own `value` attribute, or zero.
fn attr_value(part: &Part) -> f64 {
    part.attrs
        .get("value")
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0)
}

/// The knob angle a value shows, in degrees.
pub fn pot_angle(value: f64) -> f64 {
    -POT_SWEEP + (value / POT_MAX).clamp(0.0, 1.0) * 2.0 * POT_SWEEP
}

/// The value a pointer asks of the knob.
///
/// The element measures the angle from twelve o'clock, clockwise, and clamps it
/// to the sweep, so a pointer beyond either end just pins the value there. This
/// is v0.2's `knobValue` without the part-rotation step: nothing in this app
/// rotates a part yet.
pub fn pot_value(part: &Part, world: Pos2) -> Option<f64> {
    let bounds = part.bounds();
    let unit = bounds.width() / POT_VIEW.x;
    let centre = bounds.min + POT_CENTRE * unit;
    let dx = f64::from(world.x - centre.x);
    let dy = f64::from(world.y - centre.y);
    if (dx * dx + dy * dy).sqrt() < 0.5 {
        return None;
    }
    let (x, y) = (-dx, -dy);
    let mut degrees = y.atan2(x).to_degrees().round();
    if degrees < 0.0 {
        degrees += 360.0;
    }
    degrees -= 90.0;
    if x > 0.0 && y <= 0.0 && degrees > 0.0 {
        degrees -= 360.0;
    }
    let degrees = degrees.clamp(-POT_SWEEP, POT_SWEEP);
    Some((((degrees + POT_SWEEP) / (2.0 * POT_SWEEP)) * POT_MAX).round())
}

/// The tip offset a value shows, in the element's units.
pub fn slide_offset(value: f64) -> f32 {
    SLIDE_BASE + (value / SLIDE_MAX).clamp(0.0, 1.0) as f32 * SLIDE_TRAVEL
}

/// The value a pointer asks of the slide pot: its position along the track.
pub fn slide_value(part: &Part, world: Pos2) -> f64 {
    let bounds = part.bounds();
    let unit = bounds.width() / SLIDE_VIEW.x;
    let local = (world.x - bounds.min.x) / unit - SLIDE_ORIGIN;
    let fraction = ((local - SLIDE_BASE) / SLIDE_TRAVEL).clamp(0.0, 1.0);
    (f64::from(fraction) * SLIDE_MAX).round()
}

#[cfg(test)]
mod tests {
    use super::*;
    use croquis_core::catalogue::PartKind;

    fn part(kind: PartKind) -> Part {
        crate::scene::part_from_kind(kind, Pos2::ZERO, "p".into()).expect("part")
    }

    #[test]
    fn the_knob_sweeps_its_own_range() {
        assert_eq!(pot_angle(0.0), -135.0);
        assert_eq!(pot_angle(POT_MAX), 135.0);
        assert!((pot_angle(POT_MAX / 2.0) - 0.0).abs() < 1e-9, "half is upright");
    }

    #[test]
    fn the_pointer_asks_for_the_angle_it_points_at() {
        let pot = part(PartKind::Potentiometer);
        let bounds = pot.bounds();
        let unit = bounds.width() / POT_VIEW.x;
        let centre = bounds.min + POT_CENTRE * unit;
        let at = |dx: f32, dy: f32| {
            pot_value(&pot, centre + Vec2::new(dx * unit, dy * unit)).expect("a value")
        };

        // Straight up is the middle: the sweep runs from 7:30 round to 4:30.
        let up = at(0.0, -3.0);
        assert!(
            (up - POT_MAX / 2.0).abs() <= 1.0,
            "upright is half scale, got {up}"
        );
        // Down and to the left is the bottom of the sweep…
        assert_eq!(at(-3.0, 3.0), 0.0);
        // …and down and to the right is the top.
        assert_eq!(at(3.0, 3.0), POT_MAX);
        // Up and to the left is short of the middle, not at the bottom: this is
        // a 270 degree sweep, not a full circle.
        let up_left = at(-3.0, -2.0);
        assert!(
            (200.0..400.0).contains(&up_left),
            "up-left sits below half, got {up_left}"
        );
        // A pointer on the shaft itself asks for nothing.
        assert_eq!(pot_value(&pot, centre), None);
    }

    #[test]
    fn only_the_knob_turns_and_a_grab_on_the_body_moves_the_part() {
        let pot = part(PartKind::Potentiometer);
        let bounds = pot.bounds();
        let unit = bounds.width() / POT_VIEW.x;
        let centre = bounds.min + POT_CENTRE * unit;
        assert!(on_pot_knob(&pot, centre), "the middle of the knob");
        assert!(
            !on_pot_knob(&pot, bounds.min + Vec2::splat(1.0 * unit)),
            "a corner of the body is not the knob"
        );
    }

    #[test]
    fn the_slide_pot_handle_is_where_the_tip_is_drawn() {
        let mut slide = part(PartKind::SlidePotentiometer);
        let bounds = slide.bounds();
        let unit = bounds.width() / SLIDE_VIEW.x;
        let at = |local_x: f32| {
            bounds.min + Vec2::new(SLIDE_ORIGIN + local_x, SLIDE_ORIGIN + 9.0) * unit
        };
        // At zero the handle sits left of the track…
        assert!(on_slide_handle(&slide, at(SLIDE_HANDLE_X + -15.0 + 6.0)));
        assert!(!on_slide_handle(&slide, at(SLIDE_HANDLE_X + -15.0 + 20.0)));
        // …and at full it has moved 30 units right with it.
        slide.attrs.insert("value".into(), "100".into());
        assert!(on_slide_handle(&slide, at(SLIDE_HANDLE_X + 15.0 + 6.0)));
        assert!(!on_slide_handle(&slide, at(SLIDE_HANDLE_X + -15.0 + 6.0)));
    }

    #[test]
    fn the_stick_is_the_only_thing_that_moves_it() {
        let stick = part(PartKind::AnalogJoystick);
        let bounds = stick.bounds();
        let unit = bounds.width() / STICK_VIEW.x;
        assert!(on_stick(&stick, bounds.min + STICK_CENTRE * unit));
        assert!(
            !on_stick(&stick, bounds.min + Vec2::splat(1.0 * unit)),
            "the corner of the module is body"
        );
    }

    #[test]
    fn the_tip_runs_the_whole_track() {
        assert_eq!(slide_offset(0.0), -15.0);
        assert_eq!(slide_offset(100.0), 15.0);
        assert_eq!(slide_offset(50.0), 0.0);
    }

    #[test]
    fn a_pointer_along_the_slide_asks_for_that_position() {
        let slide = part(PartKind::SlidePotentiometer);
        let bounds = slide.bounds();
        let unit = bounds.width() / SLIDE_VIEW.x;
        let at = |local_x: f32| {
            slide_value(
                &slide,
                bounds.min + Vec2::new(SLIDE_ORIGIN + local_x, 10.0) * unit,
            )
        };
        assert_eq!(at(SLIDE_BASE), 0.0);
        assert_eq!(at(SLIDE_BASE + SLIDE_TRAVEL), 100.0);
        assert_eq!(at(SLIDE_BASE + SLIDE_TRAVEL / 2.0), 50.0);
        assert_eq!(at(-50.0), 0.0, "past the low end pins at zero");
        assert_eq!(at(50.0), 100.0, "and past the high end at full");
    }
}
