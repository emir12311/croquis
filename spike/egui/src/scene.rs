//! Load a real preset and the extracted Wokwi manifest into the canvas spike.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use eframe::egui::{Color32, Pos2, Vec2};
use serde::Deserialize;

use croquis_core::catalogue::{parse_kind, part_by_kind, CATALOGUE};
use croquis_core::diagram::parse_diagram;

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
        let waypoints = decode_route(
            CorePoint::new(from.x as f64, from.y as f64),
            &wire.route,
        )
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
