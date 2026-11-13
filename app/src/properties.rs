//! The properties strip: the selected part's own fields, edited in place.
//!
//! The catalogue already declares what each part lets you change (`PartDef::
//! fields`), so this panel only has to draw the right control per field and
//! write the value back into the part's attributes — which is where the netlist
//! reads it. v0.2 has the same strip; without it a sensor's `tilted`, a
//! potentiometer's position or a resistor's value can only be changed by editing
//! the diagram file.

use eframe::egui::{self, ComboBox, DragValue, Slider};
use croquis_core::catalogue::{
    parse_kind, part_by_kind, FieldControl, PartField, ResistanceUnit,
};

use crate::locales::Texts;
use crate::scene::Part;

/// Split ohms into a number and the unit that reads best for it.
///
/// The largest unit that does not push the number below 1, so 10 000 Ω reads as
/// `10 kΩ` and 0.5 Ω as `500 mΩ` rather than `0.5 Ω`. A value that is zero, unset
/// or not a number keeps the plain ohm unit instead of falling to pΩ.
fn split_resistance(ohms: f64, units: &[ResistanceUnit]) -> (f64, usize) {
    let plain = units
        .iter()
        .position(|unit| unit.factor == 1.0)
        .unwrap_or(0);
    if !ohms.is_finite() || ohms <= 0.0 {
        return (ohms, plain);
    }
    let mut best: Option<usize> = None;
    for (index, unit) in units.iter().enumerate() {
        if unit.factor <= ohms && best.is_none_or(|best| unit.factor > units[best].factor) {
            best = Some(index);
        }
    }
    match best {
        Some(index) => (ohms / units[index].factor, index),
        None => (ohms, plain),
    }
}


/// What an editor currently shows for a field.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorValue {
    Toggle(bool),
    Number(f64),
    Text(String),
}

/// The editor state for a field, read from the part's attributes.
///
/// A missing attribute means the field has not been set yet, so it shows the
/// control's own floor rather than inventing a value.
pub fn field_editor(part: &Part, field: &PartField) -> EditorValue {
    let text = part.attrs.get(field.name).cloned().unwrap_or_default();
    match &field.control {
        FieldControl::Toggle => EditorValue::Toggle(text == "1"),
        FieldControl::Range { min, .. } => EditorValue::Number(
            text.parse::<f64>()
                .unwrap_or(*min)
                .clamp(*min, range_max(field)),
        ),
        FieldControl::Resistance { min, max, .. } => EditorValue::Number(
            text.parse::<f64>()
                .unwrap_or(10_000.0)
                .clamp(*min, *max),
        ),
        FieldControl::Select(options) => {
            let value = if text.is_empty() {
                options
                    .first()
                    .map(|option| option.value.clone())
                    .unwrap_or_default()
            } else {
                text
            };
            EditorValue::Text(value)
        }
    }
}

fn range_max(field: &PartField) -> f64 {
    match &field.control {
        FieldControl::Range { max, .. } => *max,
        _ => f64::MAX,
    }
}

/// Write an editor's value back into the part, answering whether it changed.
pub fn field_commit(part: &mut Part, field: &PartField, value: EditorValue) -> bool {
    let text = match value {
        EditorValue::Toggle(on) => {
            if on {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        EditorValue::Number(number) => {
            // Whole numbers are how the diagram files spell these, and how the
            // netlist's parsers expect a resistance or a position.
            if (number - number.round()).abs() < 1e-9 {
                format!("{}", number.round() as i64)
            } else {
                format!("{number}")
            }
        }
        EditorValue::Text(text) => text,
    };
    if part.attrs.get(field.name) == Some(&text) {
        return false;
    }
    part.attrs.insert(field.name.to_string(), text);
    true
}

/// Draw the strip for one part. Returns true when something was edited, so the
/// caller can re-solve the circuit.
/// The properties strip, docked under the circuit bar as v0.2 has it.
///
/// One row, always present, so the canvas does not jump when the selection
/// changes. Fields sit inline: v0.2's `.props` is a single flex row.
pub fn row(ui: &mut egui::Ui, part: &mut Part, texts: &Texts, editable: bool) -> bool {
    /// One height for every control in the strip.
    ///
    /// Each widget otherwise sizes itself: a drag box, a slider and a dropdown all
    /// default differently, and in a centred row that reads as a skewed line. The
    /// number beside a unit dropdown is where it showed worst.
    const ROW_HEIGHT: f32 = 20.0;

    if !editable {
        // While the circuit runs the strip still names the part and shows what it
        // is set to, but a change here would edit the circuit underneath the
        // simulator. Greying them out says so; hiding the strip would not.
        ui.disable();
    }
    let fields: Vec<PartField> = parse_kind(&part.kind)
        .map(|kind| part_by_kind(kind).fields.clone())
        .unwrap_or_default();
    let mut changed = false;
    ui.spacing_mut().slider_width = 92.0;
    ui.spacing_mut().item_spacing.x = 6.0;
    ui.spacing_mut().interact_size.y = ROW_HEIGHT;
    // A dropdown draws its label plus `button_padding` on top of `interact_size`,
    // so it comes out a couple of pixels taller than a box that was sized to the
    // row — and in a centred row that reads as a skew. Deriving the padding from
    // the row height and the text height is what actually puts them on one line.
    let body = egui::TextStyle::Body.resolve(ui.style()).size;
    let text_height = ui.fonts_mut(|fonts| fonts.row_height(&egui::FontId::proportional(body)));
    ui.spacing_mut().button_padding.y = ((ROW_HEIGHT - text_height) / 2.0).max(0.0);
    // The name reads as text, not as muted chrome: labels inherit the
    // non-interactive foreground, which here is the muted colour.
    ui.label(
        egui::RichText::new(texts.t(&part.name_key))
            .color(egui::Color32::from_rgb(0xe6, 0xe4, 0xe0))
            .strong(),
    );
    for field in &fields {
        ui.separator();
        changed |= edit(ui, part, field, texts);
    }
    changed
}

fn edit(ui: &mut egui::Ui, part: &mut Part, field: &PartField, texts: &Texts) -> bool {
    match field.control.clone() {
        FieldControl::Toggle => {
            let mut on = field_editor(part, field) == EditorValue::Toggle(true);
            if ui.checkbox(&mut on, texts.t(field.label_key)).changed() {
                return field_commit(part, field, EditorValue::Toggle(on));
            }
            false
        }
        FieldControl::Range { min, max, step } => {
            let EditorValue::Number(mut number) = field_editor(part, field) else {
                return false;
            };
            ui.label(texts.t(field.label_key));
            let slider = Slider::new(&mut number, min..=max)
                .step_by(step.max(f64::EPSILON))
                .max_decimals(0);
            if ui.add(slider).changed() {
                return field_commit(part, field, EditorValue::Number(number));
            }
            false
        }
        FieldControl::Resistance { units, min, max } => {
            let EditorValue::Number(ohms) = field_editor(part, field) else {
                return false;
            };
            ui.label(texts.t(field.label_key));
            // A number and its unit, as v0.2 has it: 10 kΩ reads as `10` beside a
            // `kΩ`, not as `10000 Ω`. The unit is chosen to keep the number near 1
            // and up, and changing it re-reads the value in the new unit — that is
            // the whole point of a unit selector.
            //
            // The parted value is recomputed from ohms every frame rather than
            // remembered, so the box and the dropdown cannot drift apart.
            let (mut number, mut unit_index) = split_resistance(ohms, &units);
            let factor = units[unit_index].factor;
            let speed = if number.abs() < 10.0 { 0.1 } else { 1.0 };
            let drag = DragValue::new(&mut number)
                .speed(speed)
                .range(min / factor..=max / factor);
            let typed = ui
                .add_sized([70.0, ui.spacing().interact_size.y], drag)
                .changed();
            let mut unit_changed = false;
            ComboBox::from_id_salt(format!("unit-{}-{}", part.id, field.name))
                .width(64.0)
                .selected_text(units[unit_index].label)
                .show_ui(ui, |ui| {
                    for (index, unit) in units.iter().enumerate() {
                        if ui
                            .selectable_value(&mut unit_index, index, unit.label)
                            .clicked()
                        {
                            unit_changed = true;
                        }
                    }
                });
            if typed || unit_changed {
                let ohms = number * units[unit_index].factor;
                return field_commit(part, field, EditorValue::Number(ohms));
            }
            false
        }
        FieldControl::Select(options) => {
            let EditorValue::Text(current) = field_editor(part, field) else {
                return false;
            };
            let label = options
                .iter()
                .find(|option| option.value == current)
                .map(|option| texts.t(&option.label_key).to_string())
                .unwrap_or_else(|| current.clone());
            ui.label(texts.t(field.label_key));
            let mut chosen = current.clone();
            ComboBox::from_id_salt(format!("field-{}-{}", part.id, field.name))
                .width(140.0)
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for option in &options {
                        ui.selectable_value(
                            &mut chosen,
                            option.value.clone(),
                            texts.t(&option.label_key),
                        );
                    }
                });
            if chosen != current {
                return field_commit(part, field, EditorValue::Text(chosen));
            }
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use croquis_core::catalogue::PartKind;
    use eframe::egui::Pos2;

    fn part(kind: PartKind) -> Part {
        crate::scene::part_from_kind(kind, Pos2::ZERO, "p".into()).expect("part")
    }

    fn field(kind: PartKind, name: &str) -> PartField {
        part_by_kind(kind)
            .fields
            .iter()
            .find(|field| field.name == name)
            .unwrap_or_else(|| panic!("no field {name}"))
            .clone()
    }

    #[test]
    fn a_toggle_writes_one_or_zero() {
        let mut tilt = part(PartKind::TiltSwitch);
        let tilted = field(PartKind::TiltSwitch, "tilted");
        assert_eq!(field_editor(&tilt, &tilted), EditorValue::Toggle(false));
        assert!(field_commit(&mut tilt, &tilted, EditorValue::Toggle(true)));
        assert_eq!(tilt.attrs["tilted"], "1");
        assert_eq!(field_editor(&tilt, &tilted), EditorValue::Toggle(true));
        assert!(
            !field_commit(&mut tilt, &tilted, EditorValue::Toggle(true)),
            "writing the same value again is not a change"
        );
        assert!(field_commit(&mut tilt, &tilted, EditorValue::Toggle(false)));
        assert_eq!(tilt.attrs["tilted"], "0");
    }

    #[test]
    fn a_range_keeps_whole_numbers_whole() {
        let mut pot = part(PartKind::Potentiometer);
        let position = field(PartKind::Potentiometer, "value");
        assert!(field_commit(&mut pot, &position, EditorValue::Number(512.0)));
        assert_eq!(pot.attrs["value"], "512");
        assert_eq!(
            field_editor(&pot, &position),
            EditorValue::Number(512.0),
            "and reads back as what it was set to"
        );
        assert!(field_commit(&mut pot, &position, EditorValue::Number(1023.0)));
        assert_eq!(
            field_editor(&pot, &position),
            EditorValue::Number(1023.0)
        );
    }

    #[test]
    fn a_select_defaults_to_its_first_option() {
        let mut led = part(PartKind::Led);
        let colour = field(PartKind::Led, "color");
        let first = match &colour.control {
            FieldControl::Select(options) => options[0].value.clone(),
            other => panic!("led colour is not a select: {other:?}"),
        };
        assert_eq!(field_editor(&led, &colour), EditorValue::Text(first));
        assert!(field_commit(
            &mut led,
            &colour,
            EditorValue::Text("blue".into())
        ));
        assert_eq!(led.attrs["color"], "blue");
    }

    #[test]
    fn a_resistance_picks_the_unit_that_reads_best() {
        use croquis_core::catalogue::RESISTANCE_UNITS;

        let (number, index) = split_resistance(10_000.0, RESISTANCE_UNITS);
        assert_eq!(number, 10.0, "10 kΩ reads as 10, not 10000");
        assert_eq!(RESISTANCE_UNITS[index].label, "kΩ");

        let (number, index) = split_resistance(220.0, RESISTANCE_UNITS);
        assert_eq!(number, 220.0);
        assert_eq!(RESISTANCE_UNITS[index].label, "Ω");

        let (number, index) = split_resistance(0.5, RESISTANCE_UNITS);
        assert_eq!(number, 500.0, "half an ohm reads as 500 mΩ");
        assert_eq!(RESISTANCE_UNITS[index].label, "mΩ");
    }

    #[test]
    fn a_resistance_reads_back_as_ohms() {
        let mut resistor = part(PartKind::Resistor);
        let resistance = field(PartKind::Resistor, "value");
        // Resistors come in at 220 Ω, so that is what the editor shows first.
        assert_eq!(
            field_editor(&resistor, &resistance),
            EditorValue::Number(220.0)
        );
        assert!(
            !field_commit(&mut resistor, &resistance, EditorValue::Number(220.0)),
            "the default is not an edit"
        );
        assert!(field_commit(
            &mut resistor,
            &resistance,
            EditorValue::Number(1000.0)
        ));
        assert_eq!(resistor.attrs["value"], "1000");
        assert_eq!(
            field_editor(&resistor, &resistance),
            EditorValue::Number(1000.0)
        );
    }
}
