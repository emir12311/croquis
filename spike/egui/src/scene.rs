//! Load a real preset and the extracted Wokwi manifest into the canvas spike.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{Color32, Pos2, Vec2};
use serde::Deserialize;

use croquis_core::catalogue::{parse_kind, part_by_kind, PartKind, CATALOGUE};
use croquis_core::diagram::{build_diagram, parse_diagram, PlacedPart, RoutedWire};
use croquis_core::document::PinRef as CorePinRef;
use croquis_core::netlist::{
    evaluate_circuit_with_state, CircuitEval, EvalPart, EvalWire, KeyPosition, LedState, PinPoint,
};
use croquis_core::shift_register::ShiftState;

#[derive(Clone)]
pub struct Pin {
    pub name: String,
    pub offset: Vec2,
}

#[derive(Clone)]
pub struct Part {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub center: Pos2,
    /// The part's unrotated size. `rotated_size` is what it occupies on screen.
    pub size: Vec2,
    /// Clockwise degrees, a quarter turn at a time: 0, 90, 180 or 270.
    pub rotation: f32,
    pub color: Color32,
    /// Pin offsets from the centre, in the part's own unrotated frame.
    pub pins: Vec<Pin>,
    pub attrs: HashMap<String, String>,
    pub svg: String,
}

impl Part {
    /// The box the part takes up once turned: a quarter turn swaps its sides.
    pub fn rotated_size(&self) -> Vec2 {
        if (self.rotation.round() as i32).rem_euclid(180) == 90 {
            Vec2::new(self.size.y, self.size.x)
        } else {
            self.size
        }
    }

    pub fn bounds(&self) -> eframe::egui::Rect {
        eframe::egui::Rect::from_center_size(self.center, self.rotated_size())
    }

    /// The unrotated box: the frame a part's drawings are measured in, before
    /// its quarter turn is applied.
    pub fn rect(&self) -> eframe::egui::Rect {
        eframe::egui::Rect::from_center_size(self.center, self.size)
    }

    /// A pin's position, with the part's own rotation applied — which is what
    /// the netlist, the wires and the hit-test all have to agree on.
    pub fn pin_world(&self, index: usize) -> Pos2 {
        self.center + rotate_offset(self.pins[index].offset, self.rotation)
    }
}

/// Rotate an offset clockwise by a multiple of 90 degrees (screen space, y down).
pub fn rotate_offset(offset: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Vec2::new(offset.x * cos - offset.y * sin, offset.x * sin + offset.y * cos)
}

#[derive(Deserialize)]
struct Manifest {
    parts: Vec<ManifestPart>,
}

#[derive(Deserialize)]
struct ManifestPart {
    kind: String,
    width: f32,
    height: f32,
    svg: String,
    pins: Vec<ManifestPin>,
}

#[derive(Deserialize)]
struct ManifestPin {
    name: String,
    x: f32,
    y: f32,
}

use croquis_core::route::{decode_route, Point as CorePoint};

pub type PinRef = (usize, usize);

#[derive(Clone)]
pub struct Wire {
    pub a: PinRef,
    pub b: PinRef,
    pub waypoints: Vec<Pos2>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn load_blink() -> (Vec<Part>, Vec<Wire>) {
    let path = root().join("presets/blink.json");
    let (parts, wires, _) = load_diagram_file(&path);
    (parts, wires)
}

/// Load a diagram file, returning the parts, wires and sketch.
pub fn load_diagram_file(path: &Path) -> (Vec<Part>, Vec<Wire>, String) {
    let root = root();
    let manifest_text = fs::read_to_string(root.join("assets/wokwi/manifest.json")).ok();
    let preset_text = fs::read_to_string(path).ok();
    let (Some(manifest_text), Some(preset_text)) = (manifest_text, preset_text) else {
        return (Vec::new(), Vec::new(), String::new());
    };
    let Ok(manifest) = serde_json::from_str::<Manifest>(&manifest_text) else {
        return (Vec::new(), Vec::new(), String::new());
    };
    let Ok(parsed) = parse_diagram(&preset_text, &CATALOGUE) else {
        return (Vec::new(), Vec::new(), String::new());
    };
    let defs: HashMap<&str, &ManifestPart> = manifest
        .parts
        .iter()
        .map(|part| (part.kind.as_str(), part))
        .collect();

    let mut parts = Vec::new();
    let mut indices = HashMap::new();
    for source in &parsed.parts {
        let Some(kind) = parse_kind(&source.kind) else {
            continue;
        };
        let Some((width, height, pins, svg)) = part_art(&root, kind, &defs) else {
            continue;
        };
        let index = parts.len();
        indices.insert(source.id.clone(), index);
        parts.push(Part {
            id: source.id.clone(),
            kind: source.kind.clone(),
            name: part_by_kind(kind).label.to_string(),
            center: Pos2::new(source.x as f32 + width / 2.0, source.y as f32 + height / 2.0),
            size: Vec2::new(width, height),
            rotation: source.rotate as f32,
            color: Color32::from_rgb(70, 100, 130),
            pins,
            attrs: source.attrs.clone(),
            svg,
        });
    }

    let mut wires = Vec::new();
    for wire in &parsed.wires {
        let (Some(&a), Some(&b)) = (
            indices.get(&wire.a.instance_id),
            indices.get(&wire.b.instance_id),
        ) else {
            continue;
        };
        let Some(pin_a) = parts[a].pins.iter().position(|pin| pin.name == wire.a.pin) else {
            continue;
        };
        let Some(pin_b) = parts[b].pins.iter().position(|pin| pin.name == wire.b.pin) else {
            continue;
        };
        let from = parts[a].pin_world(pin_a);
        let waypoints = decode_route(CorePoint::new(from.x as f64, from.y as f64), &wire.route)
            .into_iter()
            .map(|point| Pos2::new(point.x as f32, point.y as f32))
            .collect();
        wires.push(Wire {
            a: (a, pin_a),
            b: (b, pin_b),
            waypoints,
        });
    }
    (parts, wires, parsed.sketch.unwrap_or_default())
}

/// Which tab a preset sits under in File → Presets. v0.2's grouping: the parts
/// every lesson uses, then inputs, then outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetGroup {
    Basics,
    Sensors,
    Outputs,
}

impl PresetGroup {
    /// In the order the tabs read, which is also the teaching order.
    pub const ALL: [PresetGroup; 3] = [Self::Basics, Self::Sensors, Self::Outputs];

    pub fn label(self) -> &'static str {
        match self {
            Self::Basics => "Basics",
            Self::Sensors => "Sensors",
            Self::Outputs => "Outputs",
        }
    }

    /// The group a preset id belongs to. An id nobody listed is a basic: it is
    /// still reachable, which is the point of a fallback rather than a filter.
    fn of(id: &str) -> Self {
        match id {
            "photoresistor" | "thermometer" | "gas-sensor" | "joystick" | "digital-sensors"
            | "dht22" | "ir-remote" | "keypad" | "ultrasonic" => Self::Sensors,
            "rgb" | "seven-segment" | "four-digit" | "led-bar" | "buzzer-relay" | "lcd" | "servo"
            | "stepper" | "shift-register" => Self::Outputs,
            _ => Self::Basics,
        }
    }
}

/// One shipped circuit: its file, its title, and the tab it opens from.
pub struct Preset {
    pub id: String,
    pub title: String,
    pub path: PathBuf,
    pub group: PresetGroup,
}

/// The shipped presets, in the id order v0.2 lists them.
pub fn presets() -> Vec<Preset> {
    let mut entries = Vec::new();
    let Ok(dir) = fs::read_dir(root().join("presets")) else {
        return entries;
    };
    for entry in dir.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("untitled")
            .to_string();
        let title = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("title")
                    .and_then(|title| title.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| id.clone());
        entries.push(Preset {
            group: PresetGroup::of(&id),
            id,
            title,
            path,
        });
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries
}

/// Write the current circuit in the old app's diagram format.
pub fn save_diagram_file(
    path: &Path,
    parts: &[Part],
    wires: &[Wire],
    sketch: &str,
) -> Result<(), String> {
    let placed: Vec<PlacedPart> = parts
        .iter()
        .map(|part| PlacedPart {
            id: part.id.clone(),
            kind: part.kind.clone(),
            x: (part.center.x - part.size.x / 2.0) as f64,
            y: (part.center.y - part.size.y / 2.0) as f64,
            // Left/top stay the unrotated box, as Wokwi's own files have them,
            // and the turn is carried beside them.
            rotate: (part.rotation != 0.0).then_some(part.rotation as f64),
            attrs: part.attrs.clone(),
        })
        .collect();
    let routed: Vec<RoutedWire> = wires
        .iter()
        .filter_map(|wire| {
            let a = &parts[wire.a.0];
            let b = &parts[wire.b.0];
            Some(RoutedWire {
                a: CorePinRef {
                    instance_id: a.id.clone(),
                    pin: a.pins[wire.a.1].name.clone(),
                },
                b: CorePinRef {
                    instance_id: b.id.clone(),
                    pin: b.pins[wire.b.1].name.clone(),
                },
                colour: None,
                waypoints: wire
                    .waypoints
                    .iter()
                    .map(|point| CorePoint::new(point.x as f64, point.y as f64))
                    .collect(),
            })
        })
        .collect();
    let pin_point = |reference: &CorePinRef| {
        let part = parts.iter().find(|part| part.id == reference.instance_id)?;
        let index = part.pins.iter().position(|pin| pin.name == reference.pin)?;
        let point = part.pin_world(index);
        Some(CorePoint::new(point.x as f64, point.y as f64))
    };
    let diagram = build_diagram(&placed, &routed, sketch, &CATALOGUE, &pin_point);
    let text = serde_json::to_string_pretty(&diagram).map_err(|error| error.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(|error| error.to_string())
}

/// Evaluate the Blink preset with the AVR's pin 13 high.
pub fn evaluate_blink() -> HashMap<String, LedState> {
    let avr = HashMap::from([("B5".into(), true)]);
    evaluate_blink_with(&avr)
}

/// Evaluate any loaded circuit and return the full state.
pub fn evaluate_scene(
    parts: &[Part],
    wires: &[Wire],
    pressed: &HashMap<String, bool>,
    avr: &HashMap<String, bool>,
    pin_duty: &HashMap<String, f64>,
    pin_pulse: &HashMap<String, f64>,
    keypad_keys: &HashMap<String, Option<KeyPosition>>,
    previous_shift: &HashMap<String, ShiftState>,
    pin_frequency: &HashMap<String, f64>,
) -> CircuitEval {
    let eval_parts: Vec<EvalPart> = parts
        .iter()
        .filter_map(|part| {
            let kind = parse_kind(&part.kind)?;
            Some(EvalPart {
                id: part.id.clone(),
                kind,
                // A pin sitting in a breadboard hole is how the board's strips
                // connect: the netlist joins holes by proximity, so it needs the
                // legs' real positions, not just their names.
                pins: part
                    .pins
                    .iter()
                    .enumerate()
                    .map(|(index, pin)| {
                        let at = part.pin_world(index);
                        PinPoint {
                            name: pin.name.clone(),
                            x: f64::from(at.x),
                            y: f64::from(at.y),
                        }
                    })
                    .collect(),
                attrs: part.attrs.clone(),
            })
        })
        .collect();
    let eval_wires: Vec<EvalWire> = wires
        .iter()
        .filter_map(|wire| {
            Some(EvalWire {
                a: CorePinRef {
                    instance_id: parts[wire.a.0].id.clone(),
                    pin: parts[wire.a.0].pins[wire.a.1].name.clone(),
                },
                b: CorePinRef {
                    instance_id: parts[wire.b.0].id.clone(),
                    pin: parts[wire.b.0].pins[wire.b.1].name.clone(),
                },
            })
        })
        .collect();
    evaluate_circuit_with_state(
        &eval_parts,
        &eval_wires,
        pressed,
        avr,
        pin_duty,
        pin_pulse,
        keypad_keys,
        previous_shift,
        pin_frequency,
    )
}

/// The LED states from a loaded circuit.
pub fn evaluate_with(
    parts: &[Part],
    wires: &[Wire],
    avr: &HashMap<String, bool>,
) -> HashMap<String, LedState> {
    evaluate_scene(
        parts,
        wires,
        &HashMap::new(),
        avr,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
    .leds
}

/// Evaluate Blink from the simulator's current AVR pin levels.
pub fn evaluate_blink_with(avr: &HashMap<String, bool>) -> HashMap<String, LedState> {
    let (parts, wires) = load_blink();
    evaluate_with(&parts, &wires, avr)
}

/// Apply the LED element's own state rules to its extracted SVG.
///
/// The element's glow group is hidden by default. Its first ellipse carries the
/// light colour and the first/third ellipses carry `opacity = 0.3 + brightness*0.7`.
pub fn led_state_svg(svg: &str, color: &str, brightness: f64, on: bool) -> String {
    let light_color = match color.to_ascii_lowercase().as_str() {
        "green" => "#80ff80",
        "blue" => "#8080ff",
        "yellow" => "#ffff80",
        "orange" => "#ffcf80",
        "white" => "#ffffff",
        "purple" => "#ff80ff",
        _ => "#ff8080",
    };
    let brightness = brightness.clamp(0.0, 1.0);
    let opacity = if brightness > f64::EPSILON {
        ((0.3 + brightness * 0.7) * 1000.0).round() / 1000.0
    } else {
        0.0
    };
    let Some(start) = svg.find("<g class=\"light\"") else {
        return svg.to_string();
    };
    let Some(relative_end) = svg[start..].find("</g>") else {
        return svg.to_string();
    };
    let end = start + relative_end + "</g>".len();
    let group = &svg[start..end];
    let display = if on {
        "display: block"
    } else {
        "display: none"
    };
    let patched = group
        .replacen(
            "style=\"display: none\"",
            &format!("style=\"{display}\""),
            1,
        )
        .replacen("fill=\"#ff8080\"", &format!("fill=\"{light_color}\""), 1)
        .replacen(
            "style=\"opacity: 1\"",
            &format!("style=\"opacity: {opacity}\""),
            1,
        )
        .replacen(
            "style=\"opacity: 1\"",
            &format!("style=\"opacity: {opacity}\""),
            1,
        );
    format!("{}{}{}", &svg[..start], patched, &svg[end..])
}

/// The joystick's base art, ready to have its knob drawn on top.
///
/// The element's own CSS says `.controls .region { fill: none }`, which the
/// extraction lost, so the four invisible hit-areas rendered as grey blocks.
/// The knob itself is hidden: it moves with the axes, and a texture that moved
/// would have to be re-rasterised for every step of a drag.
pub fn joystick_svg(svg: &str) -> String {
    let mut out = svg.to_string();
    // The hit-areas keep their outline paths; only the fills go.
    out = out.replace("<rect class=\"region\"", "<rect class=\"region\" fill=\"none\"");
    // The knob is drawn live, at whatever the axes say.
    out = out.replace(
        "<circle id=\"knob\" cx=\"13.6\" cy=\"13.6\" r=\"10.6\" fill=\"url(#g-knob)\" filter=\"url(#noise)\" tabindex=\"0\" transform=\"translate(0, 0)\"></circle>",
        "",
    );
    out
}

/// The sketch text from the Blink preset.
pub fn blink_sketch() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let Ok(text) = fs::read_to_string(root.join("presets/blink.json")) else {
        return String::new();
    };
    parse_diagram(&text, &CATALOGUE)
        .ok()
        .and_then(|diagram| diagram.sketch)
        .unwrap_or_default()
}

/// Create a new instance of a catalogue part for the palette.
/// A part's art and pins: from the Wokwi assets when they have it, or generated
/// when the kind is one Croquis draws itself (the breadboards, the 74HC595).
fn part_art(
    root: &Path,
    kind: PartKind,
    defs: &HashMap<&str, &ManifestPart>,
) -> Option<(f32, f32, Vec<Pin>, String)> {
    if let Some(def) = defs.get(kind.as_str()) {
        let svg = fs::read_to_string(root.join("assets/wokwi").join(&def.svg)).ok()?;
        let pins = def
            .pins
            .iter()
            .map(|pin| Pin {
                name: pin.name.clone(),
                offset: Vec2::new(pin.x - def.width / 2.0, pin.y - def.height / 2.0),
            })
            .collect();
        return Some((def.width, def.height, pins, svg));
    }
    let built = crate::generated::for_kind(kind)?;
    let pins = built
        .pins
        .iter()
        .map(|(name, x, y)| Pin {
            name: name.clone(),
            offset: Vec2::new(x - built.width / 2.0, y - built.height / 2.0),
        })
        .collect();
    Some((built.width, built.height, pins, built.svg))
}

pub fn part_from_kind(kind: PartKind, center: Pos2, id: String) -> Option<Part> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(root.join("assets/wokwi/manifest.json")).ok()?;
    let manifest = serde_json::from_str::<Manifest>(&text).ok()?;
    let defs: HashMap<&str, &ManifestPart> = manifest
        .parts
        .iter()
        .map(|part| (part.kind.as_str(), part))
        .collect();
    let (width, height, pins, svg) = part_art(&root, kind, &defs)?;
    let meta = part_by_kind(kind);
    Some(Part {
        id,
        kind: kind.as_str().to_string(),
        name: meta.label.to_string(),
        center,
        size: Vec2::new(width, height),
        rotation: 0.0,
        color: Color32::from_rgb(70, 100, 130),
        pins,
        attrs: meta.attrs.clone(),
        svg,
    })
}

/// Apply the LED bar element's segment fills.
pub fn led_bar_svg(svg: &str, values: &[u8], color: &str) -> String {
    let needle = "x=\"2.5\" width=\"5\" height=\"1.74\"";
    let mut out = String::new();
    let mut rest = svg;
    let mut index = 0;
    while let Some(position) = rest.find(needle) {
        let after = &rest[position + needle.len()..];
        out.push_str(&rest[..position + needle.len()]);
        if let Some(fill) = after.find("fill=\"#444\"") {
            out.push_str(&after[..fill]);
            let lit = values.get(index).copied().unwrap_or(0) != 0;
            let fill_color = if lit {
                bar_colour(color, index)
            } else {
                "#444".into()
            };
            out.push_str(&format!("fill=\"{fill_color}\""));
            rest = &after[fill + "fill=\"#444\"".len()..];
        } else {
            out.push_str(after);
            rest = "";
        }
        index += 1;
    }
    out.push_str(rest);
    out
}

fn bar_colour(color: &str, index: usize) -> String {
    match color {
        "GYR" => [
            "#9eff3c", "#9eff3c", "#9eff3c", "#9eff3c", "#9eff3c", "#f1d73c", "#f1d73c", "#f1d73c",
            "#dc012d", "#dc012d",
        ][index.min(9)]
        .into(),
        "BCYR" => [
            "#2c95fa", "#6cf9dc", "#6cf9dc", "#6cf9dc", "#6cf9dc", "#f1d73c", "#f1d73c", "#f1d73c",
            "#dc012d", "#dc012d",
        ][index.min(9)]
        .into(),
        "green" => "#9eff3c".into(),
        "blue" => "#2c95fa".into(),
        "cyan" => "#6cf9dc".into(),
        "yellow" => "#f1d73c".into(),
        "red" => "#dc012d".into(),
        other => other.into(),
    }
}

/// Rotate the servo horn from the netlist angle.
pub fn servo_svg(svg: &str, angle: f64) -> String {
    svg.replacen("rotate(0)", &format!("rotate({angle})"), 1)
}

/// The potentiometer's base art: everything except the tick that moves.
///
/// The tick is `<rect id="rotating">`, which the element turns itself. Baking it
/// into a texture per angle cost about 40 ms a state — measured — which is far
/// too much to pay on every step of a drag, so the art keeps still and the tick
/// is drawn live (`draw_pot_tick`).
pub fn potentiometer_svg(svg: &str) -> String {
    let Some(start) = svg.find("<rect id=\"rotating\"") else {
        return svg.to_string();
    };
    let Some(length) = svg[start..].find("></rect>") else {
        return svg.to_string();
    };
    format!("{}{}", &svg[..start], &svg[start + length + "></rect>".len()..])
}

/// The slide pot's base art: everything except the tip that slides.
pub fn slide_pot_svg(svg: &str) -> String {
    let Some(start) = svg.find("<g id=\"tip\"") else {
        return svg.to_string();
    };
    let Some(length) = svg[start..].find("</g>") else {
        return svg.to_string();
    };
    format!("{}{}", &svg[..start], &svg[start + length + "</g>".len()..])
}

/// The art a part rests with when nothing is driving it: the palette's picture,
/// and the canvas's starting point before a state is layered on. These patches
/// are the ones that are not state — the joystick's invisible hit-areas, the
/// knobs the elements draw themselves.
pub fn resting_svg(part: &Part) -> String {
    match part.kind.as_str() {
        "potentiometer" => potentiometer_svg(&part.svg),
        "slide-potentiometer" => slide_pot_svg(&part.svg),
        "analog-joystick" => joystick_svg(&part.svg),
        // A resting button, not the element's stale pressed cap.
        "pushbutton" => pushbutton_svg(&part.svg, false),
        _ => part.svg.clone(),
    }
}

/// The pushbutton with its cap held down.
///
/// The element keeps two concentric circles and a stylesheet rule that hides the
/// `button-active-circle` until the button is held. The extraction lost the
/// rule, so the down circle sat on top of the up one and both states looked
/// pressed — a press was invisible except for the seam. Restore the rule rather
/// than swapping a gradient on a circle nothing can see.
pub fn pushbutton_svg(svg: &str, pressed: bool) -> String {
    if pressed {
        return svg.to_string();
    }
    svg.replacen(
        "<circle class=\"button-active-circle\"",
        "<circle class=\"button-active-circle\" style=\"display: none\"",
        1,
    )
}

/// The board's own "L" LED, lit when pin 13 is high.
///
/// The element adds a red dot inside the LED's body group and changes nothing
/// else. Pin 13 is port B, bit 5.
pub fn uno_svg(svg: &str, led13: bool) -> String {
    if !led13 {
        return svg.to_string();
    }
    const GROUP: &str = "<g transform=\"translate(26.87,11.69)\">";
    let Some(start) = svg.find(GROUP) else {
        return svg.to_string();
    };
    let Some(end) = svg[start..].find("</g>") else {
        return svg.to_string();
    };
    let lit = format!(
        "{GROUP}\n          <use xlink:href=\"#led-body\"></use>\n          <circle cx=\"1.3\" cy=\"0.55\" r=\"1.3\" fill=\"#ff8080\" filter=\"url(#ledFilter)\"></circle>\n        "
    );
    format!("{}{}{}", &svg[..start], lit, &svg[start + end..])
}

/// Rotate the stepper's shaft and rotor arrow to an angle.
///
/// The element draws both, in its own two forms: the arrow is
/// `rotate(0, 28.65,28.65)` about the centre, the shaft is
/// `translate(28.65, 28.65) rotate(0)`. `servo_svg` cannot be used here — its
/// `rotate(0)` replacement lands inside the first of those and leaves
/// `rotate(<angle>), 28.65,28.65)`, which is not a transform at all.
pub fn stepper_svg(svg: &str, angle: f64) -> String {
    svg.replacen(
        "rotate(0, 28.65,28.65)",
        &format!("rotate({}, 28.65,28.65)", tidy(angle)),
        1,
    )
    .replacen(
        "rotate(0)",
        &format!("rotate({})", tidy(angle)),
        1,
    )
}

/// An angle as SVG wants it: short, and never `-0`.
fn tidy(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded == 0.0 {
        "0".into()
    } else {
        format!("{rounded}")
    }
}

/// Apply the 7-segment element's segment/DP fills.
pub fn seven_segment_svg(svg: &str, values: &[u8], color: &str) -> String {
    let lit = match color {
        "green" => "#9eff3c",
        "blue" => "#2c95fa",
        "cyan" => "#6cf9dc",
        "yellow" => "#f1d73c",
        "white" => "#ffffff",
        _ => "#dc012d",
    };
    let mut out = String::new();
    let mut rest = svg;
    let mut index = 0;
    let needle = "fill=\"#444\"";
    while let Some(position) = rest.find(needle) {
        out.push_str(&rest[..position]);
        let on = values.get(index).copied().unwrap_or(0) != 0;
        out.push_str(&format!("fill=\"{}\"", if on { lit } else { "#444" }));
        rest = &rest[position + needle.len()..];
        index += 1;
    }
    out.push_str(rest);
    out
}
