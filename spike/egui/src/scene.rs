//! Load a real preset and the extracted Wokwi manifest into the canvas spike.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use eframe::egui::{Color32, Pos2, Vec2};
use serde::Deserialize;

use croquis_core::catalogue::{parse_kind, part_by_kind, PartKind, CATALOGUE};
use croquis_core::diagram::parse_diagram;
use croquis_core::document::PinRef as CorePinRef;
use croquis_core::netlist::{evaluate_circuit, EvalPart, EvalWire, LedState};

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
    pub size: Vec2,
    pub color: Color32,
    pub pins: Vec<Pin>,
    pub attrs: HashMap<String, String>,
    pub svg: String,
}

impl Part {
    pub fn bounds(&self) -> eframe::egui::Rect {
        eframe::egui::Rect::from_center_size(self.center, self.size)
    }

    pub fn pin_world(&self, index: usize) -> Pos2 {
        self.center + self.pins[index].offset
    }
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

pub fn load_blink() -> (Vec<Part>, Vec<Wire>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest_text = fs::read_to_string(root.join("assets/wokwi/manifest.json")).ok();
    let preset_text = fs::read_to_string(root.join("presets/blink.json")).ok();
    let (Some(manifest_text), Some(preset_text)) = (manifest_text, preset_text) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(manifest) = serde_json::from_str::<Manifest>(&manifest_text) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(parsed) = parse_diagram(&preset_text, &CATALOGUE) else {
        return (Vec::new(), Vec::new());
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
        let Some(def) = defs.get(source.kind.as_str()) else {
            continue;
        };
        let Some(svg) = fs::read_to_string(root.join("assets/wokwi").join(&def.svg)).ok() else {
            continue;
        };
        let index = parts.len();
        indices.insert(source.id.clone(), index);
        parts.push(Part {
            id: source.id.clone(),
            kind: source.kind.clone(),
            name: part_by_kind(kind).label.to_string(),
            center: Pos2::new(
                source.x as f32 + def.width / 2.0,
                source.y as f32 + def.height / 2.0,
            ),
            size: Vec2::new(def.width, def.height),
            color: Color32::from_rgb(70, 100, 130),
            pins: def
                .pins
                .iter()
                .map(|pin| Pin {
                    name: pin.name.clone(),
                    offset: Vec2::new(pin.x - def.width / 2.0, pin.y - def.height / 2.0),
                })
                .collect(),
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
    (parts, wires)
}

/// Evaluate the Blink preset with the AVR's pin 13 high.
pub fn evaluate_blink() -> HashMap<String, LedState> {
    let avr = HashMap::from([("B5".into(), true)]);
    evaluate_blink_with(&avr)
}

/// Evaluate Blink from the simulator's current AVR pin levels.
pub fn evaluate_blink_with(avr: &HashMap<String, bool>) -> HashMap<String, LedState> {
    let (parts, wires) = load_blink();
    let eval_parts: Vec<EvalPart> = parts
        .iter()
        .filter_map(|part| {
            let kind = parse_kind(&part.kind)?;
            Some(EvalPart {
                id: part.id.clone(),
                kind,
                pins: Vec::new(),
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
    evaluate_circuit(
        &eval_parts,
        &eval_wires,
        &HashMap::new(),
        avr,
        &HashMap::new(),
    )
    .leds
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
pub fn part_from_kind(kind: PartKind, center: Pos2, id: String) -> Option<Part> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let text = fs::read_to_string(root.join("assets/wokwi/manifest.json")).ok()?;
    let manifest = serde_json::from_str::<Manifest>(&text).ok()?;
    let def = manifest
        .parts
        .iter()
        .find(|part| part.kind == kind.as_str())?;
    let svg = fs::read_to_string(root.join("assets/wokwi").join(&def.svg)).ok()?;
    let meta = part_by_kind(kind);
    Some(Part {
        id,
        kind: kind.as_str().to_string(),
        name: meta.label.to_string(),
        center,
        size: Vec2::new(def.width, def.height),
        color: Color32::from_rgb(70, 100, 130),
        pins: def
            .pins
            .iter()
            .map(|pin| Pin {
                name: pin.name.clone(),
                offset: Vec2::new(pin.x - def.width / 2.0, pin.y - def.height / 2.0),
            })
            .collect(),
        attrs: meta.attrs.clone(),
        svg,
    })
}
