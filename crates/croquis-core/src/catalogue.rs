//! The component catalogue: the data the rest of the core reads.
//!
//! Fields and defaults are ported from the TypeScript catalogue. Icons are not
//! included yet: the art source is still an open Phase 0 decision, so the Rust
//! registry deliberately carries semantics, not palette drawings.
//!
//! Display text does not live here. A part or a field carries a *key* into
//! `assets/locales/<tag>.json`, and the UI resolves it in the reader's language.
//! The keys are either derived (`part.<kind>`) or named constants
//! ([`field_key`]), so a part cannot be added without a key and a field cannot
//! name one that does not exist.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::diagram::TagMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PartKind {
    Uno,
    Led,
    RgbLed,
    Resistor,
    Pushbutton,
    Potentiometer,
    SlidePotentiometer,
    PhotoresistorSensor,
    NtcTemperatureSensor,
    AnalogJoystick,
    MembraneKeypad,
    Dht22,
    TiltSwitch,
    PirMotionSensor,
    FlameSensor,
    GasSensor,
    SoundSensor,
    HcSr04,
    IrReceiver,
    IrRemote,
    Buzzer,
    Relay,
    Servo,
    StepperMotor,
    ShiftRegister74hc595,
    SevenSegment,
    SevenSegment4,
    Lcd1602,
    LedBarGraph,
    Breadboard,
    BreadboardFull,
}

impl PartKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uno => "uno",
            Self::Led => "led",
            Self::RgbLed => "rgb-led",
            Self::Resistor => "resistor",
            Self::Pushbutton => "pushbutton",
            Self::Potentiometer => "potentiometer",
            Self::SlidePotentiometer => "slide-potentiometer",
            Self::PhotoresistorSensor => "photoresistor-sensor",
            Self::NtcTemperatureSensor => "ntc-temperature-sensor",
            Self::AnalogJoystick => "analog-joystick",
            Self::MembraneKeypad => "membrane-keypad",
            Self::Dht22 => "dht22",
            Self::TiltSwitch => "tilt-switch",
            Self::PirMotionSensor => "pir-motion-sensor",
            Self::FlameSensor => "flame-sensor",
            Self::GasSensor => "gas-sensor",
            Self::SoundSensor => "sound-sensor",
            Self::HcSr04 => "hc-sr04",
            Self::IrReceiver => "ir-receiver",
            Self::IrRemote => "ir-remote",
            Self::Buzzer => "buzzer",
            Self::Relay => "relay",
            Self::Servo => "servo",
            Self::StepperMotor => "stepper-motor",
            Self::ShiftRegister74hc595 => "74hc595",
            Self::SevenSegment => "seven-segment",
            Self::SevenSegment4 => "seven-segment-4",
            Self::Lcd1602 => "lcd1602",
            Self::LedBarGraph => "led-bar-graph",
            Self::Breadboard => "breadboard",
            Self::BreadboardFull => "breadboard-full",
        }
    }

    /// The key this kind's display name lives under.
    ///
    /// Derived rather than stored so the two cannot drift: a new variant gets a
    /// key the moment it gets a spelling, and `every_part_has_a_translation` is
    /// what proves the catalogue file agrees.
    pub fn label_key(self) -> String {
        format!("part.{}", self.as_str())
    }
}

/// The keys a [`PartField`] may name.
///
/// A field is not free to invent a label: it picks one of these. A misspelling
/// is then a compile error rather than a string nobody translates, and the same
/// key can be shared by two parts that mean the same thing — the potentiometer
/// and the slide potentiometer both say `Position`.
pub mod field_key {
    pub const COLOUR: &str = "field.colour";
    pub const DISTANCE: &str = "field.distance";
    pub const FLAME: &str = "field.flame";
    pub const GAS: &str = "field.gas";
    pub const HORIZONTAL: &str = "field.horizontal";
    pub const HUMIDITY: &str = "field.humidity";
    pub const LIGHT: &str = "field.light";
    pub const MOTION: &str = "field.motion";
    pub const NOISE: &str = "field.noise";
    pub const POSITION: &str = "field.position";
    pub const RESISTANCE: &str = "field.resistance";
    pub const STEPS: &str = "field.steps";
    pub const TEMPERATURE: &str = "field.temperature";
    pub const TEMPERATURE_C: &str = "field.temperature_c";
    pub const THRESHOLD: &str = "field.threshold";
    pub const TILTED: &str = "field.tilted";
    pub const VERTICAL: &str = "field.vertical";

    /// Every key above, so a test can check the catalogue files carry them all.
    pub const ALL: &[&str] = &[
        COLOUR,
        DISTANCE,
        FLAME,
        GAS,
        HORIZONTAL,
        HUMIDITY,
        LIGHT,
        MOTION,
        NOISE,
        POSITION,
        RESISTANCE,
        STEPS,
        TEMPERATURE,
        TEMPERATURE_C,
        THRESHOLD,
        TILTED,
        VERTICAL,
    ];
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldOption {
    pub value: String,
    /// The key this option's display name lives under, derived from the value so
    /// an option can never show a half-translated list.
    pub label_key: String,
    pub swatch: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResistanceUnit {
    pub label: &'static str,
    pub factor: f64,
}

pub const RESISTANCE_UNITS: &[ResistanceUnit] = &[
    ResistanceUnit {
        label: "pΩ",
        factor: 1e-12,
    },
    ResistanceUnit {
        label: "nΩ",
        factor: 1e-9,
    },
    ResistanceUnit {
        label: "µΩ",
        factor: 1e-6,
    },
    ResistanceUnit {
        label: "mΩ",
        factor: 1e-3,
    },
    ResistanceUnit {
        label: "Ω",
        factor: 1.0,
    },
    ResistanceUnit {
        label: "kΩ",
        factor: 1e3,
    },
    ResistanceUnit {
        label: "MΩ",
        factor: 1e6,
    },
    ResistanceUnit {
        label: "GΩ",
        factor: 1e9,
    },
    ResistanceUnit {
        label: "TΩ",
        factor: 1e12,
    },
];

#[derive(Clone, Debug, PartialEq)]
pub enum FieldControl {
    Select(Vec<FieldOption>),
    Resistance {
        units: Vec<ResistanceUnit>,
        min: f64,
        max: f64,
    },
    Range {
        min: f64,
        max: f64,
        step: f64,
    },
    Toggle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PartField {
    /// The attribute this field writes, which is data and never translated.
    pub name: &'static str,
    /// The key its label lives under, one of [`field_key`].
    pub label_key: &'static str,
    pub control: FieldControl,
}

#[derive(Clone, Debug)]
pub struct PartDef {
    pub kind: PartKind,
    /// The key this part's display name lives under, derived from the kind.
    pub label_key: String,
    pub tag: &'static str,
    pub diagram_type: Option<&'static str>,
    pub attrs: HashMap<String, String>,
    pub fields: Vec<PartField>,
    pub live: bool,
    pub no_pins: bool,
    pub skip_dom_pins: bool,
    pub pin_node: Option<fn(&str) -> String>,
}

fn attrs(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

fn range(
    name: &'static str,
    label_key: &'static str,
    min: f64,
    max: f64,
    step: f64,
) -> PartField {
    PartField {
        name,
        label_key,
        control: FieldControl::Range { min, max, step },
    }
}

fn toggle(name: &'static str, label_key: &'static str) -> PartField {
    PartField {
        name,
        label_key,
        control: FieldControl::Toggle,
    }
}

fn select(name: &'static str, label_key: &'static str, values: &[&str], swatches: bool) -> PartField {
    PartField {
        name,
        label_key,
        control: FieldControl::Select(
            values
                .iter()
                .map(|value| FieldOption {
                    value: (*value).to_string(),
                    label_key: format!("value.{value}"),
                    swatch: swatches.then(|| (*value).to_string()),
                })
                .collect(),
        ),
    }
}

fn resistance(name: &'static str, label_key: &'static str) -> PartField {
    PartField {
        name,
        label_key,
        control: FieldControl::Resistance {
            units: RESISTANCE_UNITS.to_vec(),
            min: 1e-12,
            max: 1e12,
        },
    }
}

fn def(kind: PartKind, tag: &'static str) -> PartDef {
    PartDef {
        kind,
        label_key: kind.label_key(),
        tag,
        diagram_type: None,
        attrs: HashMap::new(),
        fields: Vec::new(),
        live: false,
        no_pins: false,
        skip_dom_pins: false,
        pin_node: None,
    }
}

fn breadboard_pin_node(pin: &str) -> String {
    let bytes = pin.as_bytes();
    if bytes.len() >= 2
        && (b'a'..=b'j').contains(&bytes[0])
        && bytes[1..].iter().all(u8::is_ascii_digit)
    {
        let bank = if bytes[0] <= b'e' { "top" } else { "bottom" };
        return format!("{bank}{}", &pin[1..]);
    }
    if bytes.len() >= 3
        && (bytes[0] == b't' || bytes[0] == b'b')
        && (bytes[1] == b'p' || bytes[1] == b'n')
    {
        let mut index = 2;
        let mut segment = "";
        if bytes.get(index).is_some_and(u8::is_ascii_uppercase) {
            segment = &pin[index..index + 1];
            index += 1;
        }
        if index < bytes.len() && bytes[index..].iter().all(u8::is_ascii_digit) {
            return format!("rail{}{}{}", bytes[0] as char, bytes[1] as char, segment);
        }
    }
    pin.to_string()
}

fn pushbutton_pin_node(pin: &str) -> String {
    if pin.starts_with('1') {
        "1".into()
    } else {
        "2".into()
    }
}

fn build_parts() -> Vec<PartDef> {
    use field_key as key;
    let mut parts = Vec::new();

    parts.push(def(PartKind::Uno, "wokwi-arduino-uno"));

    let mut led = def(PartKind::Led, "wokwi-led");
    led.attrs = attrs(&[("color", "red")]);
    led.fields = vec![select(
        "color",
        key::COLOUR,
        &[
            "red", "green", "blue", "yellow", "orange", "white", "purple",
        ],
        true,
    )];
    parts.push(led);

    let mut rgb = def(PartKind::RgbLed, "wokwi-rgb-led");
    rgb.live = true;
    parts.push(rgb);

    let mut resistor = def(PartKind::Resistor, "wokwi-resistor");
    resistor.attrs = attrs(&[("value", "220")]);
    resistor.fields = vec![resistance("value", key::RESISTANCE)];
    parts.push(resistor);

    let mut button = def(PartKind::Pushbutton, "wokwi-pushbutton");
    button.pin_node = Some(pushbutton_pin_node);
    parts.push(button);

    let mut pot = def(PartKind::Potentiometer, "wokwi-potentiometer");
    pot.attrs = attrs(&[("value", "0")]);
    pot.fields = vec![range("value", key::POSITION, 0.0, 1023.0, 1.0)];
    pot.live = true;
    parts.push(pot);

    let mut slide = def(PartKind::SlidePotentiometer, "wokwi-slide-potentiometer");
    slide.attrs = attrs(&[("value", "0")]);
    slide.fields = vec![range("value", key::POSITION, 0.0, 100.0, 1.0)];
    slide.live = true;
    parts.push(slide);

    let mut photo = def(PartKind::PhotoresistorSensor, "wokwi-photoresistor-sensor");
    photo.attrs = attrs(&[("light", "50"), ("threshold", "50")]);
    photo.fields = vec![
        range("light", key::LIGHT, 0.0, 100.0, 1.0),
        range("threshold", key::THRESHOLD, 0.0, 100.0, 1.0),
    ];
    photo.live = true;
    parts.push(photo);

    let mut ntc = def(
        PartKind::NtcTemperatureSensor,
        "wokwi-ntc-temperature-sensor",
    );
    ntc.attrs = attrs(&[("temperature", "25")]);
    ntc.fields = vec![range("temperature", key::TEMPERATURE, -15.0, 135.0, 1.0)];
    ntc.live = true;
    parts.push(ntc);

    let mut joystick = def(PartKind::AnalogJoystick, "wokwi-analog-joystick");
    joystick.attrs = attrs(&[("x", "50"), ("y", "50")]);
    joystick.fields = vec![
        range("x", key::HORIZONTAL, 0.0, 100.0, 1.0),
        range("y", key::VERTICAL, 0.0, 100.0, 1.0),
    ];
    joystick.live = true;
    parts.push(joystick);

    let mut keypad = def(PartKind::MembraneKeypad, "wokwi-membrane-keypad");
    keypad.attrs = attrs(&[("columns", "4"), ("connector", "true")]);
    keypad.live = true;
    parts.push(keypad);

    let mut dht = def(PartKind::Dht22, "wokwi-dht22");
    dht.attrs = attrs(&[("temperature", "25"), ("humidity", "50")]);
    dht.fields = vec![
        range("temperature", key::TEMPERATURE_C, -40.0, 80.0, 1.0),
        range("humidity", key::HUMIDITY, 0.0, 100.0, 1.0),
    ];
    parts.push(dht);

    let mut tilt = def(PartKind::TiltSwitch, "wokwi-tilt-switch");
    tilt.attrs = attrs(&[("tilted", "0")]);
    tilt.fields = vec![toggle("tilted", key::TILTED)];
    parts.push(tilt);

    let mut pir = def(PartKind::PirMotionSensor, "wokwi-pir-motion-sensor");
    pir.attrs = attrs(&[("motion", "0")]);
    pir.fields = vec![toggle("motion", key::MOTION)];
    parts.push(pir);

    let mut flame = def(PartKind::FlameSensor, "wokwi-flame-sensor");
    flame.attrs = attrs(&[("flame", "0")]);
    flame.fields = vec![range("flame", key::FLAME, 0.0, 100.0, 1.0)];
    flame.live = true;
    parts.push(flame);

    let mut gas = def(PartKind::GasSensor, "wokwi-gas-sensor");
    gas.attrs = attrs(&[("gas", "0")]);
    gas.fields = vec![range("gas", key::GAS, 0.0, 100.0, 1.0)];
    gas.live = true;
    parts.push(gas);

    let mut sound = def(PartKind::SoundSensor, "wokwi-small-sound-sensor");
    sound.attrs = attrs(&[("noise", "0")]);
    sound.fields = vec![range("noise", key::NOISE, 0.0, 100.0, 1.0)];
    sound.live = true;
    parts.push(sound);

    let mut ultrasonic = def(PartKind::HcSr04, "wokwi-hc-sr04");
    ultrasonic.attrs = attrs(&[("distance", "100")]);
    ultrasonic.fields = vec![range("distance", key::DISTANCE, 2.0, 400.0, 1.0)];
    ultrasonic.live = true;
    parts.push(ultrasonic);

    parts.push(def(PartKind::IrReceiver, "wokwi-ir-receiver"));

    let mut remote = def(PartKind::IrRemote, "wokwi-ir-remote");
    remote.no_pins = true;
    parts.push(remote);

    let mut buzzer = def(PartKind::Buzzer, "wokwi-buzzer");
    buzzer.live = true;
    parts.push(buzzer);

    parts.push(def(PartKind::Relay, "wokwi-ks2e-m-dc5"));

    let mut servo = def(PartKind::Servo, "wokwi-servo");
    servo.live = true;
    parts.push(servo);

    let mut stepper = def(PartKind::StepperMotor, "wokwi-stepper-motor");
    stepper.attrs = attrs(&[("steps", "200")]);
    stepper.fields = vec![range("steps", key::STEPS, 20.0, 400.0, 1.0)];
    stepper.live = true;
    parts.push(stepper);

    parts.push(def(PartKind::ShiftRegister74hc595, "wokwi-74hc595"));

    let mut seven = def(PartKind::SevenSegment, "wokwi-7segment");
    seven.live = true;
    parts.push(seven);

    let mut four = def(PartKind::SevenSegment4, "wokwi-7segment-4");
    four.live = true;
    parts.push(four);

    let mut lcd = def(PartKind::Lcd1602, "wokwi-lcd1602");
    lcd.live = true;
    parts.push(lcd);

    let mut bar = def(PartKind::LedBarGraph, "wokwi-led-bar-graph");
    bar.attrs = attrs(&[("color", "red")]);
    bar.fields = vec![select(
        "color",
        key::COLOUR,
        &["red", "green", "yellow", "blue", "white"],
        true,
    )];
    bar.live = true;
    parts.push(bar);

    let mut breadboard = def(PartKind::Breadboard, "sim-breadboard");
    breadboard.pin_node = Some(breadboard_pin_node);
    breadboard.skip_dom_pins = true;
    parts.push(breadboard);

    let mut full = def(PartKind::BreadboardFull, "sim-breadboard-full");
    full.pin_node = Some(breadboard_pin_node);
    full.skip_dom_pins = true;
    parts.push(full);

    parts
}

static PARTS: OnceLock<Vec<PartDef>> = OnceLock::new();

pub fn parts() -> &'static [PartDef] {
    PARTS.get_or_init(build_parts)
}

pub fn part_by_kind(kind: PartKind) -> &'static PartDef {
    parts()
        .iter()
        .find(|part| part.kind == kind)
        .expect("every PartKind has a catalogue entry")
}

pub fn kind_for_tag(tag: &str) -> Option<PartKind> {
    for part in parts() {
        if part.tag == tag || part.diagram_type == Some(tag) {
            return Some(part.kind);
        }
    }
    None
}

pub fn parse_kind(kind: &str) -> Option<PartKind> {
    parts()
        .iter()
        .find(|part| part.kind.as_str() == kind)
        .map(|part| part.kind)
}

pub fn attrs_of(kind: PartKind) -> HashMap<String, String> {
    part_by_kind(kind).attrs.clone()
}

pub fn is_breadboard(kind: PartKind) -> bool {
    matches!(kind, PartKind::Breadboard | PartKind::BreadboardFull)
}

/// The static registry as a `diagram::TagMap`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StaticCatalogue;

pub const CATALOGUE: StaticCatalogue = StaticCatalogue;

impl TagMap for StaticCatalogue {
    fn tag_for_kind(&self, kind: &str) -> Option<&str> {
        parse_kind(kind).map(|kind| part_by_kind(kind).tag)
    }

    fn kind_for_tag(&self, tag: &str) -> Option<&str> {
        kind_for_tag(tag).map(PartKind::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_catalogue_is_complete_and_has_unique_tags() {
        assert_eq!(parts().len(), 31);
        let mut tags = HashSet::new();
        for part in parts() {
            assert!(tags.insert(part.tag), "duplicate tag {}", part.tag);
        }
    }

    #[test]
    fn every_kind_round_trips_through_its_string() {
        for part in parts() {
            assert_eq!(parse_kind(part.kind.as_str()), Some(part.kind));
        }
    }

    #[test]
    fn tags_resolve_and_the_four_digit_tag_is_distinct() {
        assert_eq!(kind_for_tag("wokwi-led"), Some(PartKind::Led));
        assert_eq!(kind_for_tag("wokwi-7segment"), Some(PartKind::SevenSegment));
        assert_eq!(
            kind_for_tag("wokwi-7segment-4"),
            Some(PartKind::SevenSegment4)
        );
        assert_eq!(kind_for_tag("wokwi-unknown"), None);
    }

    #[test]
    fn the_static_registry_satisfies_the_diagram_tag_map() {
        assert_eq!(CATALOGUE.tag_for_kind("led"), Some("wokwi-led"));
        assert_eq!(CATALOGUE.kind_for_tag("sim-breadboard"), Some("breadboard"));
        assert_eq!(CATALOGUE.tag_for_kind("nope"), None);
    }

    #[test]
    fn defaults_and_fields_match_the_catalogue_contract() {
        assert_eq!(attrs_of(PartKind::Led)["color"], "red");
        assert_eq!(attrs_of(PartKind::Resistor)["value"], "220");
        assert_eq!(attrs_of(PartKind::Potentiometer)["value"], "0");
        assert_eq!(part_by_kind(PartKind::Potentiometer).fields.len(), 1);
        assert!(part_by_kind(PartKind::IrRemote).no_pins);
        assert!(part_by_kind(PartKind::Breadboard).skip_dom_pins);
        assert!(!part_by_kind(PartKind::Led).live);
        assert!(part_by_kind(PartKind::RgbLed).live);
    }

    #[test]
    fn every_part_carries_the_key_its_name_lives_under() {
        // Derived from the kind, so the only way to break this is to break
        // `label_key` itself — which is the point of deriving it.
        for part in parts() {
            assert_eq!(part.label_key, format!("part.{}", part.kind.as_str()));
            assert!(
                part.label_key.starts_with("part."),
                "{} is not namespaced",
                part.label_key
            );
        }
    }

    #[test]
    fn every_field_names_a_known_label_key() {
        // No field may invent a string: it has to pick one of `field_key`'s
        // constants, and those constants have to be listed in `ALL`, which is
        // what the locale files are checked against.
        let known: HashSet<&str> = field_key::ALL.iter().copied().collect();
        assert_eq!(known.len(), field_key::ALL.len(), "ALL has a duplicate");
        for part in parts() {
            for field in &part.fields {
                assert!(
                    known.contains(field.label_key),
                    "{}'s field `{}` names `{}`, which is not in field_key::ALL",
                    part.kind.as_str(),
                    field.name,
                    field.label_key
                );
                if let FieldControl::Select(options) = &field.control {
                    for option in options {
                        assert_eq!(option.label_key, format!("value.{}", option.value));
                    }
                }
            }
        }
    }

    #[test]
    fn breadboard_strips_and_rails_share_the_expected_nodes() {
        assert_eq!(breadboard_pin_node("a1"), "top1");
        assert_eq!(breadboard_pin_node("e30"), "top30");
        assert_eq!(breadboard_pin_node("f1"), "bottom1");
        assert_eq!(breadboard_pin_node("tp1"), "railtp");
        assert_eq!(breadboard_pin_node("tnB25"), "railtnB");
        assert_eq!(breadboard_pin_node("bp2"), "railbp");
        assert_eq!(breadboard_pin_node("weird"), "weird");
    }

    #[test]
    fn pushbutton_legs_are_two_nodes() {
        let button = part_by_kind(PartKind::Pushbutton);
        let node = button.pin_node.unwrap();
        assert_eq!(node("1"), "1");
        assert_eq!(node("1.2"), "1");
        assert_eq!(node("2"), "2");
        assert_eq!(node("2.2"), "2");
    }
}
