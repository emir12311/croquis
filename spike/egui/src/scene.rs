//! Load a real preset and the extracted Wokwi manifest into the canvas spike.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{Color32, Pos2, Vec2};
use serde::Deserialize;

use croquis_core::catalogue::{parse_kind, part_by_kind, PartKind, CATALOGUE};
use croquis_core::diagram::{build_diagram, parse_diagram, PlacedPart, RoutedWire};
use croquis_core::document::PinRef as CorePinRef;
use croquis_core::netlist::{evaluate_circuit, CircuitEval, EvalPart, EvalWire, LedState};

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
    (parts, wires, parsed.sketch.unwrap_or_default())
}

/// Read the shipped preset titles and paths for the File menu.
pub fn presets() -> Vec<(String, PathBuf)> {
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
        let title = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| {
                value
                    .get("title")
                    .and_then(|title| title.as_str())
                    .map(str::to_string)
            })
            .or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "Untitled".into());
        entries.push((title, path));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
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
            rotate: None,
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
    avr: &HashMap<String, bool>,
) -> CircuitEval {
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
}

/// The LED states from a loaded circuit.
pub fn evaluate_with(
    parts: &[Part],
    wires: &[Wire],
    avr: &HashMap<String, bool>,
) -> HashMap<String, LedState> {
    evaluate_scene(parts, wires, avr).leds
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
            let fill_color = if lit { bar_colour(color, index) } else { "#444".into() };
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
        "GYR" => ["#9eff3c", "#9eff3c", "#9eff3c", "#9eff3c", "#9eff3c", "#f1d73c", "#f1d73c", "#f1d73c", "#dc012d", "#dc012d"][index.min(9)].into(),
        "BCYR" => ["#2c95fa", "#6cf9dc", "#6cf9dc", "#6cf9dc", "#6cf9dc", "#f1d73c", "#f1d73c", "#f1d73c", "#dc012d", "#dc012d"][index.min(9)].into(),
        "green" => "#9eff3c".into(),
        "blue" => "#2c95fa".into(),
        "cyan" => "#6cf9dc".into(),
        "yellow" => "#f1d73c".into(),
        "red" => "#dc012d".into(),
        other => other.into(),
    }
}
