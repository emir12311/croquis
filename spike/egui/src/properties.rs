//! The properties strip: the selected part's own fields, edited in place.
//!
//! The catalogue already declares what each part lets you change (`PartDef::
//! fields`), so this panel only has to draw the right control per field and
//! write the value back into the part's attributes — which is where the netlist
//! reads it. v0.2 has the same strip; without it a sensor's `tilted`, a
//! potentiometer's position or a resistor's value can only be changed by editing
//! the diagram file.

use eframe::egui::{self, ComboBox, DragValue, Slider};
use croquis_core::catalogue::{parse_kind, part_by_kind, FieldControl, PartField};

use crate::scene::Part;

/// What the strip is allowed to use. The panel it lives in is 220 wide; this
/// leaves the margins and keeps every control inside it.
const STRIP_WIDTH: f32 = 196.0;

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
pub fn show(ui: &mut egui::Ui, part: &mut Part) -> bool {
    let fields: Vec<PartField> = parse_kind(&part.kind)
        .map(|kind| part_by_kind(kind).fields.clone())
        .unwrap_or_default();
    ui.label(format!("{} · {}", part.name, part.id));
    if fields.is_empty() {
        ui.weak("This part has nothing to set.");
        return false;
    }
    // Fixed widths, not `available_width`: inside a scroll area that reads as
    // far more than the strip has, so a slider was built wider than the panel
    // and the panel grew to fit it — which narrowed the canvas beside it and
    // shifted every part when a part was selected.
    ui.set_max_width(STRIP_WIDTH);
    ui.spacing_mut().slider_width = 110.0;
    let mut changed = false;
    for field in &fields {
        changed |= edit(ui, part, field);
    }
    changed
}

fn edit(ui: &mut egui::Ui, part: &mut Part, field: &PartField) -> bool {
    match field.control.clone() {
        FieldControl::Toggle => {
            let mut on = field_editor(part, field) == EditorValue::Toggle(true);
            if ui.checkbox(&mut on, field.label).changed() {
                return field_commit(part, field, EditorValue::Toggle(on));
            }
            false
        }
        FieldControl::Range { min, max, step } => {
            let EditorValue::Number(mut number) = field_editor(part, field) else {
                return false;
            };
            ui.label(field.label);
            let slider = Slider::new(&mut number, min..=max)
                .step_by(step.max(f64::EPSILON))
                .max_decimals(0);
            if ui.add(slider).changed() {
                return field_commit(part, field, EditorValue::Number(number));
            }
            false
        }
        FieldControl::Resistance { min, max, .. } => {
            let EditorValue::Number(mut number) = field_editor(part, field) else {
                return false;
            };
            ui.label(field.label);
            // A resistance spans twelve decades, so a slider is useless: type it,
            // and let the drag speed grow with the value.
            let speed = number.max(1.0) / 50.0;
            let drag = DragValue::new(&mut number)
                .range(min..=max)
                .speed(speed)
                .suffix(" Ω");
            if ui.add_sized([140.0, 18.0], drag).changed() {
                return field_commit(part, field, EditorValue::Number(number));
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
                .map(|option| option.label.clone())
                .unwrap_or_else(|| current.clone());
            ui.label(field.label);
            let mut chosen = current.clone();
            ComboBox::from_id_salt(format!("field-{}-{}", part.id, field.name))
                .width(140.0)
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for option in &options {
                        ui.selectable_value(&mut chosen, option.value.clone(), &option.label);
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
