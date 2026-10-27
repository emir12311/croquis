//! The `diagram.json` format: Wokwi's parts/connections envelope, kept separate
//! from the catalogue and the app state.
//!
//! Tag resolution is injected through [`TagMap`], so this crate can be tested
//! before the full catalogue is ported. Unknown parts and connections are skipped,
//! matching the app's tolerant loader.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::route::{encode_route, Point};

pub use crate::document::PinRef;

pub const WIRE_COLOR: &str = "green";

pub trait TagMap {
    fn tag_for_kind(&self, kind: &str) -> Option<&str>;
    fn kind_for_tag(&self, tag: &str) -> Option<&str>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagramPart {
    pub id: String,
    #[serde(rename = "type")]
    pub diagram_type: String,
    pub left: f64,
    pub top: f64,
    pub attrs: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rotate: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagramFile {
    pub version: u32,
    pub author: String,
    pub editor: String,
    pub parts: Vec<DiagramPart>,
    /// Wokwi's connection shape: `[from, to, colour, route]`.
    pub connections: Vec<Vec<Value>>,
    pub sketch: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlacedPart {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub rotate: Option<f64>,
    pub attrs: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoutedWire {
    pub a: PinRef,
    pub b: PinRef,
    pub colour: Option<String>,
    pub waypoints: Vec<Point>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedPart {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    pub rotate: f64,
    pub attrs: HashMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedWire {
    pub a: PinRef,
    pub b: PinRef,
    /// The route as written; decoded once part geometry is known.
    pub route: Vec<String>,
    pub colour: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedDiagram {
    pub parts: Vec<ParsedPart>,
    pub wires: Vec<ParsedWire>,
    pub sketch: Option<String>,
}

pub fn parse_ref(reference: &str) -> Option<PinRef> {
    let (instance_id, pin) = reference.split_once(':')?;
    if instance_id.is_empty() {
        return None;
    }
    Some(PinRef {
        instance_id: instance_id.to_string(),
        pin: pin.to_string(),
    })
}

fn round(value: f64) -> f64 {
    value.round()
}

/// Turn the app's circuit into the file format.
pub fn build_diagram(
    parts: &[PlacedPart],
    wires: &[RoutedWire],
    sketch: &str,
    tags: &dyn TagMap,
    pin_point: &dyn Fn(&PinRef) -> Option<Point>,
) -> DiagramFile {
    let parts = parts
        .iter()
        .filter_map(|part| {
            let diagram_type = tags.tag_for_kind(&part.kind)?;
            Some(DiagramPart {
                id: part.id.clone(),
                diagram_type: diagram_type.to_string(),
                left: round(part.x),
                top: round(part.y),
                attrs: part.attrs.clone(),
                rotate: part.rotate.filter(|rotate| *rotate != 0.0),
            })
        })
        .collect();

    let mut connections = Vec::new();
    for wire in wires {
        let (Some(a), Some(b)) = (pin_point(&wire.a), pin_point(&wire.b)) else {
            continue;
        };
        // `[from, to, colour, route]`, with the route as one nested array of
        // relative moves — what Wokwi writes, and what `parse_diagram` reads back.
        // Spreading the moves flat put a string where the reader wanted an array,
        // so every bent wire silently lost its bends when the file was reopened.
        connections.push(vec![
            Value::String(format!("{}:{}", wire.a.instance_id, wire.a.pin)),
            Value::String(format!("{}:{}", wire.b.instance_id, wire.b.pin)),
            Value::String(
                wire.colour
                    .clone()
                    .unwrap_or_else(|| WIRE_COLOR.to_string()),
            ),
            Value::Array(
                encode_route(a, b, &wire.waypoints)
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        ]);
    }

    DiagramFile {
        version: 1,
        author: "croquis".into(),
        editor: "croquis".into(),
        parts,
        connections,
        sketch: sketch.to_string(),
    }
}

/// Read a `diagram.json` into the app's shapes.
pub fn parse_diagram(text: &str, tags: &dyn TagMap) -> Result<ParsedDiagram, serde_json::Error> {
    let doc: Value = serde_json::from_str(text)?;
    let mut parts = Vec::new();
    let mut known = HashSet::new();

    if let Some(raw_parts) = doc.get("parts").and_then(Value::as_array) {
        for raw in raw_parts {
            let Some(id) = raw.get("id").and_then(Value::as_str) else {
                continue;
            };
            let Some(diagram_type) = raw.get("type").and_then(Value::as_str) else {
                continue;
            };
            let Some(kind) = tags.kind_for_tag(diagram_type) else {
                continue;
            };

            let rotate = raw
                .get("rotate")
                .and_then(Value::as_f64)
                .map(|value| ((value % 360.0) + 360.0) % 360.0)
                .unwrap_or(0.0);

            let mut attrs = HashMap::new();
            if let Some(raw_attrs) = raw.get("attrs").and_then(Value::as_object) {
                for (key, value) in raw_attrs {
                    if let Some(value) = value.as_str() {
                        attrs.insert(key.clone(), value.to_string());
                    }
                }
            }

            parts.push(ParsedPart {
                id: id.to_string(),
                kind: kind.to_string(),
                x: raw.get("left").and_then(Value::as_f64).unwrap_or(0.0),
                y: raw.get("top").and_then(Value::as_f64).unwrap_or(0.0),
                rotate,
                attrs,
            });
            known.insert(id.to_string());
        }
    }

    let mut wires = Vec::new();
    if let Some(raw_connections) = doc.get("connections").and_then(Value::as_array) {
        for raw in raw_connections {
            let Some(items) = raw.as_array() else {
                continue;
            };
            if items.len() < 2 {
                continue;
            }
            let Some(a) = items[0].as_str().and_then(parse_ref) else {
                continue;
            };
            let Some(b) = items[1].as_str().and_then(parse_ref) else {
                continue;
            };
            if !known.contains(&a.instance_id) || !known.contains(&b.instance_id) {
                continue;
            }
            let route = items
                .get(3)
                .and_then(Value::as_array)
                .map(|route| {
                    route
                        .iter()
                        .map(|value| {
                            value
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| value.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let colour = items
                .get(2)
                .and_then(Value::as_str)
                .filter(|colour| !colour.is_empty() && *colour != WIRE_COLOR)
                .map(str::to_string);
            wires.push(ParsedWire {
                a,
                b,
                route,
                colour,
            });
        }
    }

    Ok(ParsedDiagram {
        parts,
        wires,
        sketch: doc
            .get("sketch")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestTags {
        to_tag: HashMap<&'static str, &'static str>,
        to_kind: HashMap<&'static str, &'static str>,
    }

    impl TestTags {
        fn new() -> Self {
            let pairs = [
                ("uno", "wokwi-arduino-uno"),
                ("resistor", "wokwi-resistor"),
                ("led", "wokwi-led"),
            ];
            Self {
                to_tag: pairs.into_iter().collect(),
                to_kind: pairs.into_iter().map(|(kind, tag)| (tag, kind)).collect(),
            }
        }
    }

    impl TagMap for TestTags {
        fn tag_for_kind(&self, kind: &str) -> Option<&str> {
            self.to_tag.get(kind).copied()
        }

        fn kind_for_tag(&self, tag: &str) -> Option<&str> {
            self.to_kind.get(tag).copied()
        }
    }

    fn point(reference: &PinRef) -> Option<Point> {
        Some(Point::new(
            100.0 + reference.pin.len() as f64 * 7.0,
            40.0 + reference.instance_id.len() as f64 * 13.0,
        ))
    }

    fn circuit() -> (Vec<PlacedPart>, Vec<RoutedWire>, String) {
        let parts = vec![
            PlacedPart {
                id: "uno".into(),
                kind: "uno".into(),
                x: 60.0,
                y: 80.0,
                rotate: None,
                attrs: HashMap::new(),
            },
            PlacedPart {
                id: "r1".into(),
                kind: "resistor".into(),
                x: 430.0,
                y: 200.0,
                rotate: Some(90.0),
                attrs: HashMap::from([("value".into(), "4700".into())]),
            },
            PlacedPart {
                id: "led1".into(),
                kind: "led".into(),
                x: 580.0,
                y: 170.0,
                rotate: None,
                attrs: HashMap::from([("color".into(), "blue".into())]),
            },
        ];
        let wires = vec![RoutedWire {
            a: PinRef {
                instance_id: "r1".into(),
                pin: "2".into(),
            },
            b: PinRef {
                instance_id: "led1".into(),
                pin: "A".into(),
            },
            colour: None,
            waypoints: Vec::new(),
        }];
        (parts, wires, "void setup() {}\nvoid loop() {}\n".into())
    }

    #[test]
    fn writes_the_wokwi_shape_with_the_sketch_riding_along() {
        let (parts, wires, sketch) = circuit();
        let doc = build_diagram(&parts, &wires, &sketch, &TestTags::new(), &point);
        assert_eq!(doc.version, 1);
        assert_eq!(
            doc.parts
                .iter()
                .map(|part| part.diagram_type.as_str())
                .collect::<Vec<_>>(),
            vec!["wokwi-arduino-uno", "wokwi-resistor", "wokwi-led"]
        );
        assert_eq!(&doc.connections[0][..3], &["r1:2", "led1:A", WIRE_COLOR]);
        assert!(doc.sketch.contains("void setup"));
    }

    #[test]
    fn keeps_position_rotation_and_attributes_through_a_save_and_load() {
        let (parts, wires, sketch) = circuit();
        let tags = TestTags::new();
        let doc = build_diagram(&parts, &wires, &sketch, &tags, &point);
        let back = parse_diagram(&serde_json::to_string(&doc).unwrap(), &tags).unwrap();
        assert_eq!(
            back.parts
                .iter()
                .map(|part| (
                    part.id.as_str(),
                    part.kind.as_str(),
                    part.x,
                    part.y,
                    part.rotate
                ))
                .collect::<Vec<_>>(),
            vec![
                ("uno", "uno", 60.0, 80.0, 0.0),
                ("r1", "resistor", 430.0, 200.0, 90.0),
                ("led1", "led", 580.0, 170.0, 0.0),
            ]
        );
        assert_eq!(back.parts[1].attrs["value"], "4700");
        assert_eq!(back.parts[2].attrs["color"], "blue");
        assert_eq!(back.sketch.as_deref(), Some(sketch.as_str()));
    }

    #[test]
    fn round_trips_a_circuit_through_the_format() {
        let (parts, wires, sketch) = circuit();
        let tags = TestTags::new();
        let first = parse_diagram(
            &serde_json::to_string(&build_diagram(&parts, &wires, &sketch, &tags, &point)).unwrap(),
            &tags,
        )
        .unwrap();
        let rebuilt_parts: Vec<PlacedPart> = first
            .parts
            .iter()
            .map(|part| PlacedPart {
                id: part.id.clone(),
                kind: part.kind.clone(),
                x: part.x,
                y: part.y,
                rotate: Some(part.rotate),
                attrs: part.attrs.clone(),
            })
            .collect();
        let rebuilt_wires: Vec<RoutedWire> = first
            .wires
            .iter()
            .map(|wire| RoutedWire {
                a: wire.a.clone(),
                b: wire.b.clone(),
                colour: wire.colour.clone(),
                waypoints: crate::route::decode_route(point(&wire.a).unwrap(), &wire.route),
            })
            .collect();
        let second = parse_diagram(
            &serde_json::to_string(&build_diagram(
                &rebuilt_parts,
                &rebuilt_wires,
                first.sketch.as_deref().unwrap_or(""),
                &tags,
                &point,
            ))
            .unwrap(),
            &tags,
        )
        .unwrap();
        assert_eq!(second, first);
    }

    #[test]
    fn a_bend_survives_a_save_and_a_load() {
        // The route rides in one nested array of relative moves. Written flat, the
        // reader found a string where it wanted an array and dropped the whole
        // route, so every bent wire straightened itself on the way back in.
        let (parts, _, sketch) = circuit();
        let tags = TestTags::new();
        let wires = vec![RoutedWire {
            a: PinRef {
                instance_id: "r1".into(),
                pin: "2".into(),
            },
            b: PinRef {
                instance_id: "led1".into(),
                pin: "A".into(),
            },
            colour: None,
            waypoints: vec![crate::route::Point::new(500.0, 150.0)],
        }];
        let text =
            serde_json::to_string(&build_diagram(&parts, &wires, &sketch, &tags, &point)).unwrap();
        let parsed = parse_diagram(&text, &tags).unwrap();
        let wire = &parsed.wires[0];
        let back = crate::route::decode_route(point(&wire.a).unwrap(), &wire.route);
        assert!(
            !back.is_empty(),
            "a bent wire keeps its route through the file: {text}"
        );
    }

    #[test]
    fn skips_unknown_parts_and_connections_rather_than_guessing() {
        let text = serde_json::json!({
            "parts": [
                { "id": "uno", "type": "wokwi-arduino-uno", "left": 10, "top": 20, "attrs": {} },
                { "id": "x", "type": "wokwi-something-new", "left": 0, "top": 0 },
                { "type": "wokwi-led", "left": 0, "top": 0 }
            ],
            "connections": [
                ["uno:13", "ghost:A"],
                ["uno:13", "uno:GND.2", "red", ["v10"]],
                "nonsense",
                ["only-one"]
            ],
            "sketch": "void setup() {}\nvoid loop() {}\n"
        })
        .to_string();
        let doc = parse_diagram(&text, &TestTags::new()).unwrap();
        assert_eq!(doc.parts.len(), 1);
        assert_eq!(doc.parts[0].id, "uno");
        assert_eq!(doc.wires.len(), 1);
        assert_eq!(doc.wires[0].colour.as_deref(), Some("red"));
        assert_eq!(doc.wires[0].route, vec!["v10"]);
    }

    #[test]
    fn treats_the_default_wire_colour_as_no_colour() {
        let text = serde_json::json!({
            "parts": [{ "id": "uno", "type": "wokwi-arduino-uno", "left": 0, "top": 0 }],
            "connections": [["uno:13", "uno:GND.2", WIRE_COLOR, []]]
        })
        .to_string();
        let doc = parse_diagram(&text, &TestTags::new()).unwrap();
        assert_eq!(doc.wires[0].colour, None);
        assert_eq!(doc.sketch, None);
    }

    #[test]
    fn reads_a_pin_reference_and_refuses_a_malformed_one() {
        assert_eq!(
            parse_ref("uno:13"),
            Some(PinRef {
                instance_id: "uno".into(),
                pin: "13".into()
            })
        );
        assert_eq!(
            parse_ref("led1:C"),
            Some(PinRef {
                instance_id: "led1".into(),
                pin: "C".into()
            })
        );
        assert_eq!(parse_ref("nocolon"), None);
        assert_eq!(parse_ref(":pin-only"), None);
    }

    #[test]
    fn skips_a_part_whose_kind_is_not_in_the_catalogue() {
        let parts = vec![PlacedPart {
            id: "mystery".into(),
            kind: "not-a-part".into(),
            x: 0.0,
            y: 0.0,
            rotate: None,
            attrs: HashMap::new(),
        }];
        let doc = build_diagram(&parts, &[], "", &TestTags::new(), &point);
        assert!(doc.parts.is_empty());
    }
}
