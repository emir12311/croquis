//! The foundational electrical model: connectivity, resistor paths, LEDs,
//! buttons and analogue dividers.
//!
//! This is deliberately the first layer of the TypeScript netlist, not the whole
//! thing. Outputs, displays, relays, protocol sensors and libraries arrive in
//! later slices; the contracts and the pure solving model start here.

use std::collections::{HashMap, HashSet};

use crate::catalogue::{is_breadboard, part_by_kind, PartKind, CATALOGUE};
use crate::diagram::parse_diagram;
use crate::document::PinRef;

const SUPPLY_V: f64 = 5.0;
const PIN_OHMS: f64 = 25.0;
const LED_BURN_MA: f64 = 25.0;
const LED_LIT_MA: f64 = 3.0;
const LED_DIM_MA: f64 = 0.2;
const LED_STEPS: f64 = 31.0;
const BREADBOARD_PITCH: f64 = 67.0 / 7.0;
const HOLE_REACH: f64 = BREADBOARD_PITCH * 0.45;

const UNO_DIGITAL: &[(&str, &str, u8)] = &[
    ("0", "D", 0),
    ("1", "D", 1),
    ("2", "D", 2),
    ("3", "D", 3),
    ("4", "D", 4),
    ("5", "D", 5),
    ("6", "D", 6),
    ("7", "D", 7),
    ("8", "B", 0),
    ("9", "B", 1),
    ("10", "B", 2),
    ("11", "B", 3),
    ("12", "B", 4),
    ("13", "B", 5),
    ("A0", "C", 0),
    ("A1", "C", 1),
    ("A2", "C", 2),
    ("A3", "C", 3),
    ("A4", "C", 4),
    ("A5", "C", 5),
];

const UNO_ANALOG_CHANNELS: &[(&str, u8)] = &[
    ("A0", 0),
    ("A1", 1),
    ("A2", 2),
    ("A3", 3),
    ("A4", 4),
    ("A5", 5),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnoPin {
    Mcu { port: char, pin: u8 },
    Rail(Rail),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rail {
    Gnd,
    Vcc,
}

pub fn uno_pin(pin: &str) -> Option<UnoPin> {
    if let Some((_, port, number)) = UNO_DIGITAL.iter().find(|(name, _, _)| *name == pin) {
        return Some(UnoPin::Mcu {
            port: port.chars().next().unwrap(),
            pin: *number,
        });
    }
    if pin.starts_with("GND") {
        return Some(UnoPin::Rail(Rail::Gnd));
    }
    if matches!(pin, "5V" | "3.3V" | "VIN") {
        return Some(UnoPin::Rail(Rail::Vcc));
    }
    None
}

fn node_name(kind: PartKind, pin: &str) -> String {
    part_by_kind(kind)
        .pin_node
        .map_or_else(|| pin.to_string(), |node| node(pin))
}

#[derive(Clone, Debug, PartialEq)]
pub struct PinPoint {
    pub name: String,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug)]
pub struct EvalPart {
    pub id: String,
    pub kind: PartKind,
    pub pins: Vec<PinPoint>,
    pub attrs: HashMap<String, String>,
}

impl EvalPart {
    pub fn new(id: impl Into<String>, kind: PartKind) -> Self {
        Self {
            id: id.into(),
            kind,
            pins: Vec::new(),
            attrs: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EvalWire {
    pub a: PinRef,
    pub b: PinRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LedState {
    Off,
    Dim,
    Lit,
    Burnt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drive {
    pub port: char,
    pub pin: u8,
    pub high: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalogReading {
    pub channel: u8,
    pub millivolts: u16,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RgbState {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorState {
    pub powered: bool,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputState {
    pub active: bool,
    pub values: Vec<u8>,
    pub frequency: Option<u16>,
    pub angle: Option<f64>,
    pub digit: Option<u8>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CircuitEval {
    pub leds: HashMap<String, LedState>,
    pub led_brightness: HashMap<String, f64>,
    pub rgb_leds: HashMap<String, RgbState>,
    pub sensors: HashMap<String, SensorState>,
    pub outputs: HashMap<String, OutputState>,
    pub drives: Vec<Drive>,
    pub warnings: HashMap<String, String>,
    pub analogs: Vec<AnalogReading>,
}

pub fn led_step(brightness: f64) -> f64 {
    (brightness.clamp(0.0, 1.0) * LED_STEPS).round() / LED_STEPS
}

struct UnionFind {
    parent: HashMap<String, String>,
}

impl UnionFind {
    fn new() -> Self {
        Self {
            parent: HashMap::new(),
        }
    }

    fn find(&mut self, key: &str) -> String {
        if !self.parent.contains_key(key) {
            self.parent.insert(key.to_string(), key.to_string());
        }
        let mut root = key.to_string();
        while self.parent[&root] != root {
            root = self.parent[&root].clone();
        }
        let mut current = key.to_string();
        while current != root {
            let next = self.parent[&current].clone();
            self.parent.insert(current, root.clone());
            current = next;
        }
        root
    }

    fn union(&mut self, a: &str, b: &str) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(ra, rb);
        }
    }
}

fn fraction_of(attrs: &HashMap<String, String>, key: &str, top: f64) -> f64 {
    let value = attrs
        .get(key)
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0);
    (value / top).clamp(0.0, 1.0)
}

type ActiveFn = fn(&HashMap<String, String>) -> bool;

struct AnalogSource {
    out: &'static str,
    symmetric: bool,
    top: f64,
    attr: &'static str,
    on_off: Option<(&'static str, ActiveFn)>,
}

fn over_half(attrs: &HashMap<String, String>, key: &str) -> bool {
    fraction_of(attrs, key, 100.0) >= 0.5
}

fn photo_active(attrs: &HashMap<String, String>) -> bool {
    fraction_of(attrs, "light", 100.0) >= fraction_of(attrs, "threshold", 100.0)
}

fn analog_source(kind: PartKind) -> Option<AnalogSource> {
    match kind {
        PartKind::Potentiometer => Some(AnalogSource {
            out: "SIG",
            symmetric: true,
            top: 1023.0,
            attr: "value",
            on_off: None,
        }),
        PartKind::SlidePotentiometer => Some(AnalogSource {
            out: "SIG",
            symmetric: true,
            top: 100.0,
            attr: "value",
            on_off: None,
        }),
        PartKind::PhotoresistorSensor => Some(AnalogSource {
            out: "AO",
            symmetric: false,
            top: 100.0,
            attr: "light",
            on_off: Some(("DO", photo_active)),
        }),
        PartKind::NtcTemperatureSensor => Some(AnalogSource {
            out: "OUT",
            symmetric: false,
            top: 150.0,
            attr: "temperature",
            on_off: None,
        }),
        PartKind::FlameSensor => Some(AnalogSource {
            out: "AOUT",
            symmetric: false,
            top: 100.0,
            attr: "flame",
            on_off: Some(("DOUT", |attrs| over_half(attrs, "flame"))),
        }),
        PartKind::GasSensor => Some(AnalogSource {
            out: "AOUT",
            symmetric: false,
            top: 100.0,
            attr: "gas",
            on_off: Some(("DOUT", |attrs| over_half(attrs, "gas"))),
        }),
        PartKind::SoundSensor => Some(AnalogSource {
            out: "AOUT",
            symmetric: false,
            top: 100.0,
            attr: "noise",
            on_off: Some(("DOUT", |attrs| over_half(attrs, "noise"))),
        }),
        _ => None,
    }
}

fn resistor_edges(parts: &[EvalPart], uf: &mut UnionFind) -> HashMap<String, Vec<(String, f64)>> {
    let mut edges: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for part in parts {
        if part.kind != PartKind::Resistor {
            continue;
        }
        let ohms = part
            .attrs
            .get("value")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(220.0);
        if !ohms.is_finite() || ohms <= 0.0 {
            continue;
        }
        let a = uf.find(&format!("{}:1", part.id));
        let b = uf.find(&format!("{}:2", part.id));
        edges.entry(a.clone()).or_default().push((b.clone(), ohms));
        edges.entry(b).or_default().push((a, ohms));
    }
    edges
}

fn reach_to(
    from: &str,
    targets: &HashSet<String>,
    edges: &HashMap<String, Vec<(String, f64)>>,
) -> Option<(String, f64)> {
    let mut seen: HashMap<String, Option<(String, f64)>> = HashMap::new();
    seen.insert(from.to_string(), None);
    let mut queue = vec![from.to_string()];
    let mut index = 0;
    while index < queue.len() {
        let net = queue[index].clone();
        index += 1;
        if targets.contains(&net) {
            let mut ohms = 0.0;
            let mut step = seen.get(&net).cloned().flatten();
            while let Some((previous, resistance)) = step {
                ohms += resistance;
                step = seen.get(&previous).cloned().flatten();
            }
            return Some((net, ohms));
        }
        for (next, ohms) in edges.get(&net).into_iter().flatten() {
            if seen.contains_key(next) {
                continue;
            }
            seen.insert(next.clone(), Some((net.clone(), *ohms)));
            queue.push(next.clone());
        }
    }
    None
}

fn resistance_to(
    from: &str,
    targets: &HashSet<String>,
    edges: &HashMap<String, Vec<(String, f64)>>,
) -> Option<f64> {
    reach_to(from, targets, edges).map(|(_, ohms)| ohms)
}

struct Rails {
    vcc: HashSet<String>,
    gnd: HashSet<String>,
    mcu_high: HashSet<String>,
    mcu_low: HashSet<String>,
    duty: HashMap<String, f64>,
    mcu_pins: HashMap<String, (char, u8)>,
}

fn collect_rails(
    parts: &[EvalPart],
    wires: &[EvalWire],
    uf: &mut UnionFind,
    avr: &HashMap<String, bool>,
    pin_duty: &HashMap<String, f64>,
) -> Rails {
    let mut rails = Rails {
        vcc: HashSet::new(),
        gnd: HashSet::new(),
        mcu_high: HashSet::new(),
        mcu_low: HashSet::new(),
        duty: HashMap::new(),
        mcu_pins: HashMap::new(),
    };
    for part in parts {
        if part.kind != PartKind::Uno {
            continue;
        }
        for (name, port, pin) in UNO_DIGITAL {
            let Some(UnoPin::Mcu { .. }) = uno_pin(name) else {
                continue;
            };
            let key = format!("{}{}", port, pin);
            let root = uf.find(&format!("{}:{}", part.id, node_name(part.kind, name)));
            let high = avr.get(&key).copied().unwrap_or(false);
            if high {
                rails.mcu_high.insert(root.clone());
                let duty = pin_duty.get(&key).copied().unwrap_or(1.0).clamp(0.0, 1.0);
                let current = rails.duty.get(&root).copied().unwrap_or(0.0);
                rails.duty.insert(root.clone(), current.max(duty));
            } else {
                rails.mcu_low.insert(root.clone());
            }
            rails.mcu_pins.insert(
                format!("{root}/{key}"),
                (port.chars().next().unwrap(), *pin),
            );
        }
    }
    for wire in wires {
        for reference in [&wire.a, &wire.b] {
            let Some(part) = parts.iter().find(|part| part.id == reference.instance_id) else {
                continue;
            };
            if part.kind != PartKind::Uno {
                continue;
            }
            let Some(UnoPin::Rail(rail)) = uno_pin(&reference.pin) else {
                continue;
            };
            let root = uf.find(&format!(
                "{}:{}",
                part.id,
                node_name(part.kind, &reference.pin)
            ));
            match rail {
                Rail::Vcc => {
                    rails.vcc.insert(root);
                }
                Rail::Gnd => {
                    rails.gnd.insert(root);
                }
            }
        }
    }
    rails
}

/// Evaluate connectivity, resistor paths, LEDs, buttons and analogue dividers.
pub fn evaluate_circuit(
    parts: &[EvalPart],
    wires: &[EvalWire],
    pressed: &HashMap<String, bool>,
    avr: &HashMap<String, bool>,
    pin_duty: &HashMap<String, f64>,
) -> CircuitEval {
    let mut uf = UnionFind::new();
    let key = |reference: &PinRef| {
        let kind = parts
            .iter()
            .find(|part| part.id == reference.instance_id)
            .map_or(PartKind::Led, |part| part.kind);
        format!(
            "{}:{}",
            reference.instance_id,
            node_name(kind, &reference.pin)
        )
    };
    for wire in wires {
        uf.union(&key(&wire.a), &key(&wire.b));
    }
    for part in parts {
        if part.kind == PartKind::Pushbutton && pressed.get(&part.id).copied().unwrap_or(false) {
            uf.union(&format!("{}:1", part.id), &format!("{}:2", part.id));
        }
    }

    let boards: Vec<&EvalPart> = parts
        .iter()
        .filter(|part| is_breadboard(part.kind) && !part.pins.is_empty())
        .collect();
    if !boards.is_empty() {
        let reach_sq = HOLE_REACH * HOLE_REACH;
        for part in parts {
            if is_breadboard(part.kind) || part.pins.is_empty() {
                continue;
            }
            for pin in &part.pins {
                let mut hit: Option<(String, f64)> = None;
                for board in &boards {
                    for hole in &board.pins {
                        let dx = hole.x - pin.x;
                        if dx.abs() > HOLE_REACH {
                            continue;
                        }
                        let dy = hole.y - pin.y;
                        if dy.abs() > HOLE_REACH {
                            continue;
                        }
                        let distance = dx * dx + dy * dy;
                        if distance <= reach_sq
                            && hit.as_ref().is_none_or(|(_, best)| distance < *best)
                        {
                            hit = Some((
                                format!(
                                    "{}:{}",
                                    board.id,
                                    node_name(PartKind::Breadboard, &hole.name)
                                ),
                                distance,
                            ));
                        }
                    }
                }
                if let Some((net, _)) = hit {
                    uf.union(
                        &format!("{}:{}", part.id, node_name(part.kind, &pin.name)),
                        &net,
                    );
                }
            }
        }
    }

    let rails = collect_rails(parts, wires, &mut uf, avr, pin_duty);
    let edges = resistor_edges(parts, &mut uf);
    let high_nets: HashSet<String> = rails.vcc.union(&rails.mcu_high).cloned().collect();
    let low_nets: HashSet<String> = rails.gnd.union(&rails.mcu_low).cloned().collect();
    let net_high = |root: &str| high_nets.contains(root);
    let net_low = |root: &str| low_nets.contains(root);

    let mut eval = CircuitEval::default();

    for part in parts {
        if part.kind != PartKind::Led {
            continue;
        }
        let a = uf.find(&format!("{}:A", part.id));
        let c = uf.find(&format!("{}:C", part.id));
        let supply = reach_to(&a, &high_nets, &edges);
        let back = reach_to(&c, &low_nets, &edges);
        let (Some((supply_net, supply_ohms)), Some((_, back_ohms))) = (supply, back) else {
            eval.leds.insert(part.id.clone(), LedState::Off);
            eval.led_brightness.insert(part.id.clone(), 0.0);
            if resistance_to(&a, &low_nets, &edges).is_some()
                && resistance_to(&c, &high_nets, &edges).is_some()
            {
                eval.warnings.insert(
                    part.id.clone(),
                    "Wired the wrong way round — swap its legs.".into(),
                );
            }
            continue;
        };
        let ohms = PIN_OHMS + supply_ohms + back_ohms;
        let forward = match part.attrs.get("color").map(String::as_str) {
            Some("red") => 1.8,
            Some("orange") => 2.0,
            Some("yellow") => 2.1,
            Some("green") => 2.2,
            Some("blue") => 3.0,
            Some("white") => 3.0,
            Some("purple") => 3.0,
            _ => 2.0,
        };
        let current = ((SUPPLY_V - forward) / ohms) * 1000.0;
        let duty = rails.duty.get(&supply_net).copied().unwrap_or(1.0);
        let average = current * duty;
        if average >= LED_BURN_MA {
            eval.leds.insert(part.id.clone(), LedState::Burnt);
            eval.led_brightness.insert(part.id.clone(), 0.0);
            eval.warnings.insert(
                part.id.clone(),
                format!(
                    "Over-driven — {} mA with nothing to limit it. Put a resistor in series.",
                    average.round()
                ),
            );
        } else {
            let state = if average >= LED_LIT_MA {
                LedState::Lit
            } else if average >= LED_DIM_MA {
                LedState::Dim
            } else {
                LedState::Off
            };
            let base = if current >= LED_LIT_MA {
                1.0
            } else {
                current / LED_LIT_MA
            };
            eval.leds.insert(part.id.clone(), state);
            eval.led_brightness.insert(
                part.id.clone(),
                if state == LedState::Off {
                    0.0
                } else {
                    (duty * base).min(1.0)
                },
            );
        }
    }

    const RGB_CHANNELS: [(&str, f64); 3] = [("R", 1.8), ("G", 2.2), ("B", 3.0)];
    for part in parts {
        if part.kind != PartKind::RgbLed {
            continue;
        }
        let common = uf.find(&format!("{}:COM", part.id));
        let back = reach_to(&common, &low_nets, &edges).map(|(_, ohms)| ohms);
        let Some(back) = back else {
            eval.rgb_leds.insert(part.id.clone(), RgbState::default());
            eval.warnings
                .insert(part.id.clone(), "Wire the common leg to GND.".into());
            continue;
        };
        let mut state = RgbState::default();
        let mut over_driven = false;
        for (pin, forward) in RGB_CHANNELS {
            let Some((supply_net, supply_ohms)) = reach_to(
                &uf.find(&format!("{}:{}", part.id, pin)),
                &high_nets,
                &edges,
            ) else {
                continue;
            };
            let ohms = PIN_OHMS + supply_ohms + back;
            let current = ((SUPPLY_V - forward) / ohms) * 1000.0;
            let duty = rails.duty.get(&supply_net).copied().unwrap_or(1.0);
            let average = current * duty;
            if average >= LED_BURN_MA {
                over_driven = true;
            }
            let base = if current >= LED_LIT_MA {
                1.0
            } else {
                current / LED_LIT_MA
            };
            let value = if average < LED_DIM_MA {
                0.0
            } else {
                (duty * base).min(1.0)
            };
            match pin {
                "R" => state.r = value,
                "G" => state.g = value,
                _ => state.b = value,
            }
        }
        if over_driven {
            eval.warnings.insert(
                part.id.clone(),
                "Over-driven — put a resistor on each colour.".into(),
            );
        }
        eval.rgb_leds.insert(part.id.clone(), state);
    }

    let mut analog_channels: HashMap<String, Vec<u8>> = HashMap::new();
    for part in parts {
        if part.kind != PartKind::Uno {
            continue;
        }
        for (pin, channel) in UNO_ANALOG_CHANNELS {
            let root = uf.find(&format!("{}:{}", part.id, node_name(PartKind::Uno, pin)));
            analog_channels.entry(root).or_default().push(*channel);
        }
    }

    for part in parts {
        let Some(source) = analog_source(part.kind) else {
            continue;
        };
        let high = uf.find(&format!("{}:VCC", part.id));
        let low = uf.find(&format!("{}:GND", part.id));
        let forward = net_high(&high) && net_low(&low);
        let reverse = source.symmetric && net_high(&low) && net_low(&high);
        if !forward && !reverse {
            let out_root = uf.find(&format!("{}:{}", part.id, source.out));
            let reaches = rails
                .mcu_pins
                .keys()
                .any(|label| label.starts_with(&format!("{out_root}/")))
                || source.on_off.is_some_and(|(pin, _)| {
                    let root = uf.find(&format!("{}:{}", part.id, pin));
                    rails
                        .mcu_pins
                        .keys()
                        .any(|label| label.starts_with(&format!("{root}/")))
                });
            if reaches {
                eval.warnings.insert(
                    part.id.clone(),
                    if source.symmetric {
                        "Put one end on 5V and the other on GND.".into()
                    } else {
                        "Its VCC pin needs 5V and its GND pin needs GND.".into()
                    },
                );
            }
            continue;
        }
        if let Some((_, active)) = source.on_off {
            eval.sensors.insert(
                part.id.clone(),
                SensorState {
                    powered: true,
                    active: active(&part.attrs),
                },
            );
        }
        let raw = if part.kind == PartKind::NtcTemperatureSensor {
            let temperature = part
                .attrs
                .get("temperature")
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(25.0);
            ((temperature + 15.0) / source.top).clamp(0.0, 1.0)
        } else {
            fraction_of(&part.attrs, source.attr, source.top)
        };
        let position = if forward { raw } else { 1.0 - raw };
        let millivolts = (SUPPLY_V * 1000.0 * position).round() as u16;
        let root = uf.find(&format!("{}:{}", part.id, source.out));
        for channel in analog_channels.get(&root).into_iter().flatten() {
            eval.analogs.push(AnalogReading {
                channel: *channel,
                millivolts,
            });
        }
    }

    for part in parts {
        let Some(source) = analog_source(part.kind) else {
            continue;
        };
        let Some((on_pin, _)) = source.on_off else {
            continue;
        };
        let Some(state) = eval.sensors.get(&part.id).copied() else {
            continue;
        };
        if !state.powered {
            continue;
        }
        let root = uf.find(&format!("{}:{}", part.id, on_pin));
        for (label, (port, pin)) in &rails.mcu_pins {
            if label.starts_with(&format!("{root}/")) {
                eval.drives.push(Drive {
                    port: *port,
                    pin: *pin,
                    high: state.active,
                });
            }
        }
    }

    for part in parts {
        let (out, attr) = match part.kind {
            PartKind::TiltSwitch => ("OUT", "tilted"),
            PartKind::PirMotionSensor => ("OUT", "motion"),
            _ => continue,
        };
        let high = uf.find(&format!("{}:VCC", part.id));
        let low = uf.find(&format!("{}:GND", part.id));
        if !(net_high(&high) && net_low(&low)) {
            let root = uf.find(&format!("{}:{}", part.id, out));
            if rails
                .mcu_pins
                .keys()
                .any(|label| label.starts_with(&format!("{root}/")))
            {
                eval.warnings.insert(
                    part.id.clone(),
                    "Its VCC pin needs 5V and its GND pin needs GND.".into(),
                );
            }
            continue;
        }
        let active = part.attrs.get(attr).is_some_and(|value| value == "1");
        eval.sensors.insert(
            part.id.clone(),
            SensorState {
                powered: true,
                active,
            },
        );
        let root = uf.find(&format!("{}:{}", part.id, out));
        for (label, (port, pin)) in &rails.mcu_pins {
            if label.starts_with(&format!("{root}/")) {
                eval.drives.push(Drive {
                    port: *port,
                    pin: *pin,
                    high: active,
                });
            }
        }
    }

    for part in parts {
        if part.kind != PartKind::AnalogJoystick {
            continue;
        }
        let high = uf.find(&format!("{}:VCC", part.id));
        let low = uf.find(&format!("{}:GND", part.id));
        if !(net_high(&high) && net_low(&low)) {
            continue;
        }
        for (pin, attr) in [("VERT", "y"), ("HORZ", "x")] {
            let millivolts =
                (SUPPLY_V * 1000.0 * fraction_of(&part.attrs, attr, 100.0)).round() as u16;
            let root = uf.find(&format!("{}:{}", part.id, pin));
            for channel in analog_channels.get(&root).into_iter().flatten() {
                eval.analogs.push(AnalogReading {
                    channel: *channel,
                    millivolts,
                });
            }
        }
        let sel = uf.find(&format!("{}:SEL", part.id));
        for (label, (port, pin)) in &rails.mcu_pins {
            if label.starts_with(&format!("{sel}/")) {
                eval.drives.push(Drive {
                    port: *port,
                    pin: *pin,
                    high: !pressed.get(&part.id).copied().unwrap_or(false),
                });
            }
        }
    }

    for part in parts {
        if part.kind != PartKind::Pushbutton {
            continue;
        }
        let roots = [
            uf.find(&format!("{}:1", part.id)),
            uf.find(&format!("{}:2", part.id)),
        ];
        let sides: &[usize] = if roots[0] == roots[1] { &[0] } else { &[0, 1] };
        for side in sides {
            let other = 1 - side;
            for (label, (port, pin)) in &rails.mcu_pins {
                if !label.starts_with(&format!("{}/", roots[*side])) {
                    continue;
                }
                let high = if pressed.get(&part.id).copied().unwrap_or(false) {
                    !rails.gnd.contains(&roots[other])
                } else {
                    resistance_to(&roots[*side], &rails.gnd, &edges).is_none()
                };
                eval.drives.push(Drive {
                    port: *port,
                    pin: *pin,
                    high,
                });
            }
        }
    }

    for part in parts {
        match part.kind {
            PartKind::Buzzer | PartKind::Relay => {
                let (a, b) = if part.kind == PartKind::Buzzer {
                    ("1", "2")
                } else {
                    ("COIL1", "COIL2")
                };
                let a = uf.find(&format!("{}:{a}", part.id));
                let b = uf.find(&format!("{}:{b}", part.id));
                let active = (high_nets.contains(&a) && low_nets.contains(&b))
                    || (high_nets.contains(&b) && low_nets.contains(&a));
                eval.outputs.insert(
                    part.id.clone(),
                    OutputState {
                        active,
                        values: Vec::new(),
                        frequency: if part.kind == PartKind::Buzzer {
                            Some(if active { 1000 } else { 0 })
                        } else {
                            None
                        },
                        ..Default::default()
                    },
                );
            }
            PartKind::SevenSegment => {
                let lit = ["COM.1", "COM.2", "COM"]
                    .iter()
                    .any(|common| low_nets.contains(&uf.find(&format!("{}:{common}", part.id))));
                let values = ["A", "B", "C", "D", "E", "F", "G", "DP"]
                    .iter()
                    .map(|pin| {
                        u8::from(lit && high_nets.contains(&uf.find(&format!("{}:{pin}", part.id))))
                    })
                    .collect();
                eval.outputs.insert(
                    part.id.clone(),
                    OutputState {
                        active: lit,
                        values,
                        digit: Some(0),
                        ..Default::default()
                    },
                );
            }
            PartKind::SevenSegment4 => {
                let mut digit = 0;
                let mut lit = false;
                for index in 0..4 {
                    if low_nets.contains(&uf.find(&format!("{}:DIG{}", part.id, index + 1))) {
                        digit = index;
                        lit = true;
                        break;
                    }
                }
                let values = ["A", "B", "C", "D", "E", "F", "G", "DP"]
                    .iter()
                    .map(|pin| {
                        u8::from(lit && high_nets.contains(&uf.find(&format!("{}:{pin}", part.id))))
                    })
                    .collect();
                eval.outputs.insert(
                    part.id.clone(),
                    OutputState {
                        active: lit,
                        values,
                        digit: Some(digit),
                        ..Default::default()
                    },
                );
            }
            PartKind::LedBarGraph => {
                let values = (1..=10)
                    .map(|index| {
                        let anode = uf.find(&format!("{}:A{index}", part.id));
                        let cathode = uf.find(&format!("{}:C{index}", part.id));
                        u8::from(high_nets.contains(&anode) && low_nets.contains(&cathode))
                    })
                    .collect::<Vec<_>>();
                eval.outputs.insert(
                    part.id.clone(),
                    OutputState {
                        active: values.iter().any(|value| *value == 1),
                        values,
                        ..Default::default()
                    },
                );
            }
            _ => {}
        }
    }

    eval
}

/// Parse every shipped preset with the Rust catalogue. Used as a corpus guard.
pub fn parse_all_presets() -> Vec<(String, crate::diagram::ParsedDiagram)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("croquis-core has a repository root")
        .join("presets");
    let mut paths: Vec<_> = std::fs::read_dir(root)
        .expect("presets/ exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap();
            let parsed = parse_diagram(&text, &CATALOGUE).unwrap();
            (path.to_string_lossy().into_owned(), parsed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::PartKind;

    fn uno() -> EvalPart {
        EvalPart::new("uno", PartKind::Uno)
    }

    fn led(color: &str) -> EvalPart {
        let mut part = EvalPart::new("led", PartKind::Led);
        part.attrs.insert("color".into(), color.into());
        part
    }

    fn resistor(value: &str) -> EvalPart {
        let mut part = EvalPart::new("r", PartKind::Resistor);
        part.attrs.insert("value".into(), value.into());
        part
    }

    fn at(instance_id: &str, pin: &str) -> PinRef {
        PinRef {
            instance_id: instance_id.into(),
            pin: pin.into(),
        }
    }

    fn wire(a: PinRef, b: PinRef) -> EvalWire {
        EvalWire { a, b }
    }

    fn run(parts: &[EvalPart], wires: &[EvalWire], avr: &[(&str, bool)]) -> CircuitEval {
        let mut all = vec![uno()];
        all.extend_from_slice(parts);
        let avr: HashMap<String, bool> = avr
            .iter()
            .map(|(key, value)| ((*key).into(), *value))
            .collect();
        evaluate_circuit(&all, wires, &HashMap::new(), &avr, &HashMap::new())
    }

    fn chain(value: Option<&str>, color: &str) -> CircuitEval {
        let mut parts = Vec::new();
        if let Some(value) = value {
            parts.push(resistor(value));
        }
        parts.push(led(color));
        let wires = if value.is_some() {
            vec![
                wire(at("uno", "13"), at("r", "1")),
                wire(at("r", "2"), at("led", "A")),
                wire(at("led", "C"), at("uno", "GND.2")),
            ]
        } else {
            vec![
                wire(at("uno", "13"), at("led", "A")),
                wire(at("led", "C"), at("uno", "GND.2")),
            ]
        };
        run(&parts, &wires, &[("B5", true)])
    }

    #[test]
    fn lights_an_led_through_a_resistor() {
        let out = chain(Some("220"), "red");
        assert_eq!(out.leds["led"], LedState::Lit);
        assert_eq!(out.led_brightness["led"], 1.0);
    }

    #[test]
    fn follows_pwm_duty() {
        let parts = [resistor("220"), led("red")];
        let wires = [
            wire(at("uno", "13"), at("r", "1")),
            wire(at("r", "2"), at("led", "A")),
            wire(at("led", "C"), at("uno", "GND.2")),
        ];
        let avr = HashMap::from([("B5".into(), true)]);
        let duty = HashMap::from([("B5".into(), 0.25)]);
        let out = evaluate_circuit(
            &[uno(), parts[0].clone(), parts[1].clone()],
            &wires,
            &HashMap::new(),
            &avr,
            &duty,
        );
        assert_eq!(out.leds["led"], LedState::Lit);
        assert!((out.led_brightness["led"] - 0.25).abs() < 1e-9);
    }

    #[test]
    fn warns_when_the_led_is_reversed() {
        let out = run(
            &[led("red")],
            &[
                wire(at("uno", "13"), at("led", "C")),
                wire(at("led", "A"), at("uno", "GND.2")),
            ],
            &[("B5", true)],
        );
        assert_eq!(out.leds["led"], LedState::Off);
        assert!(out.warnings["led"].contains("wrong way round"));
    }

    #[test]
    fn no_resistor_is_over_driven() {
        let out = chain(None, "red");
        assert_eq!(out.leds["led"], LedState::Burnt);
        assert!(out.warnings["led"].contains("Over-driven"));
    }

    #[test]
    fn potentiometer_sets_an_analog_channel() {
        let mut pot = EvalPart::new("pot", PartKind::Potentiometer);
        pot.attrs.insert("value".into(), "512".into());
        let wires = [
            wire(at("uno", "5V"), at("pot", "VCC")),
            wire(at("pot", "GND"), at("uno", "GND.2")),
            wire(at("pot", "SIG"), at("uno", "A0")),
        ];
        let out = run(&[pot], &wires, &[]);
        assert_eq!(out.analogs[0].channel, 0);
        assert!((i32::from(out.analogs[0].millivolts) - 2502).abs() <= 1);
    }

    #[test]
    fn a_pressed_button_pulls_its_pin_low() {
        let mut button = EvalPart::new("b", PartKind::Pushbutton);
        button.attrs = HashMap::new();
        let wires = [
            wire(at("b", "1"), at("uno", "2")),
            wire(at("b", "2"), at("uno", "GND.2")),
        ];
        let pressed = HashMap::from([("b".into(), true)]);
        let out = evaluate_circuit(
            &[uno(), button],
            &wires,
            &pressed,
            &HashMap::new(),
            &HashMap::new(),
        );
        assert!(out
            .drives
            .iter()
            .any(|drive| drive.port == 'D' && drive.pin == 2 && !drive.high));
    }

    #[test]
    fn all_presets_parse_and_keep_their_sketches() {
        let presets = parse_all_presets();
        assert!(!presets.is_empty());
        for (path, diagram) in presets {
            assert!(!diagram.parts.is_empty(), "{path}");
            assert!(
                diagram
                    .sketch
                    .as_deref()
                    .unwrap_or("")
                    .contains("void loop"),
                "{path}"
            );
        }
    }

    #[test]
    fn uno_pin_mapping_covers_digital_analog_and_rails() {
        assert_eq!(uno_pin("13"), Some(UnoPin::Mcu { port: 'B', pin: 5 }));
        assert_eq!(uno_pin("A0"), Some(UnoPin::Mcu { port: 'C', pin: 0 }));
        assert_eq!(uno_pin("GND.2"), Some(UnoPin::Rail(Rail::Gnd)));
        assert_eq!(uno_pin("5V"), Some(UnoPin::Rail(Rail::Vcc)));
        assert_eq!(uno_pin("D13"), None);
    }

    #[test]
    fn led_step_quantises_and_clamps() {
        assert_eq!(led_step(-1.0), 0.0);
        assert_eq!(led_step(2.0), 1.0);
        assert_eq!(led_step(0.25), 8.0 / 31.0);
    }

    #[test]
    fn rgb_channels_are_independent() {
        let rgb = EvalPart::new("rgb", PartKind::RgbLed);
        let wires = [
            wire(at("uno", "13"), at("r", "1")),
            wire(at("r", "2"), at("rgb", "R")),
            wire(at("rgb", "COM"), at("uno", "GND.2")),
        ];
        let out = run(&[resistor("220"), rgb], &wires, &[("B5", true)]);
        assert_eq!(out.rgb_leds["rgb"].r, 1.0);
        assert_eq!(out.rgb_leds["rgb"].g, 0.0);
        assert_eq!(out.rgb_leds["rgb"].b, 0.0);
    }

    #[test]
    fn a_powered_tilt_switch_drives_its_output() {
        let mut tilt = EvalPart::new("tilt", PartKind::TiltSwitch);
        tilt.attrs.insert("tilted".into(), "1".into());
        let wires = [
            wire(at("uno", "5V"), at("tilt", "VCC")),
            wire(at("tilt", "GND"), at("uno", "GND.2")),
            wire(at("tilt", "OUT"), at("uno", "2")),
        ];
        let out = run(&[tilt], &wires, &[]);
        assert!(out.sensors["tilt"].active);
        assert!(out
            .drives
            .iter()
            .any(|drive| drive.port == 'D' && drive.pin == 2 && drive.high));
    }

    #[test]
    fn a_flame_sensor_reports_both_comparator_and_analog_outputs() {
        let mut flame = EvalPart::new("flame", PartKind::FlameSensor);
        flame.attrs.insert("flame".into(), "80".into());
        let wires = [
            wire(at("uno", "5V"), at("flame", "VCC")),
            wire(at("flame", "GND"), at("uno", "GND.2")),
            wire(at("flame", "AOUT"), at("uno", "A0")),
            wire(at("flame", "DOUT"), at("uno", "3")),
        ];
        let out = run(&[flame], &wires, &[]);
        assert!(out.sensors["flame"].active);
        assert!(out
            .analogs
            .iter()
            .any(|analog| analog.channel == 0 && (i32::from(analog.millivolts) - 4000).abs() <= 1));
        assert!(out
            .drives
            .iter()
            .any(|drive| drive.port == 'D' && drive.pin == 3 && drive.high));
    }

    #[test]
    fn a_buzzer_is_a_load_between_high_and_low() {
        let buzzer = EvalPart::new("buzz", PartKind::Buzzer);
        let wires = [
            wire(at("uno", "8"), at("buzz", "1")),
            wire(at("buzz", "2"), at("uno", "GND.2")),
        ];
        let out = run(&[buzzer], &wires, &[("B0", true)]);
        assert!(out.outputs["buzz"].active);
        assert_eq!(out.outputs["buzz"].frequency, Some(1000));
    }

    #[test]
    fn a_seven_segment_maps_lit_segments() {
        let display = EvalPart::new("seg", PartKind::SevenSegment);
        let wires = [
            wire(at("uno", "2"), at("seg", "A")),
            wire(at("uno", "3"), at("seg", "B")),
            wire(at("seg", "COM.1"), at("uno", "GND.2")),
        ];
        let out = run(&[display], &wires, &[("D2", true), ("D3", true)]);
        assert!(out.outputs["seg"].active);
        assert_eq!(&out.outputs["seg"].values[..3], &[1, 1, 0]);
    }

    #[test]
    fn a_led_bar_maps_each_anode_cathode_pair() {
        let bar = EvalPart::new("bar", PartKind::LedBarGraph);
        let wires = [
            wire(at("uno", "2"), at("bar", "A1")),
            wire(at("bar", "C1"), at("uno", "GND.2")),
        ];
        let out = run(&[bar], &wires, &[("D2", true)]);
        assert_eq!(out.outputs["bar"].values[0], 1);
        assert_eq!(out.outputs["bar"].values[1], 0);
    }
}
