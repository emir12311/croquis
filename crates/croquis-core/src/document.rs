//! The circuit document: parts, wires, undo snapshots and clipboard operations.
//!
//! `kind` is a string for now. The catalogue will provide typed definitions and
//! attribute defaults; this layer deliberately does not depend on it, so the pure
//! operations can be tested before the catalogue is ported.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::route::Point;

pub const WIRE_GRID: f64 = 12.0;
pub const HISTORY_LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PinRef {
    pub instance_id: String,
    pub pin: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub kind: String,
    pub x: f64,
    pub y: f64,
    /// Rotation in degrees (0/90/180/270), applied about the part centre.
    pub rotate: Option<f64>,
    /// This part's own attributes, overriding catalogue defaults.
    pub attrs: Option<HashMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Wire {
    pub id: String,
    pub a: PinRef,
    pub b: PinRef,
    pub waypoints: Vec<Point>,
    pub colour: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub instances: Vec<Instance>,
    pub wires: Vec<Wire>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipboardPart {
    pub kind: String,
    pub dx: f64,
    pub dy: f64,
    pub attrs: Option<HashMap<String, String>>,
    pub rotate: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clipboard {
    pub x: f64,
    pub y: f64,
    pub parts: Vec<ClipboardPart>,
}

pub fn snap_to_grid(value: f64) -> f64 {
    (value / WIRE_GRID).round() * WIRE_GRID
}

/// A copy deep enough to undo to: parts, wires and waypoints are all copied.
pub fn snapshot_of(instances: &[Instance], wires: &[Wire]) -> Snapshot {
    Snapshot {
        instances: instances.to_vec(),
        wires: wires.to_vec(),
    }
}

/// Append a snapshot, dropping the oldest once the limit is reached.
pub fn history_push(history: &[Snapshot], snapshot: Snapshot) -> Vec<Snapshot> {
    let mut out = history.to_vec();
    out.push(snapshot);
    if out.len() > HISTORY_LIMIT {
        out.drain(0..out.len() - HISTORY_LIMIT);
    }
    out
}

pub fn same_pin(a: &PinRef, b: &PinRef) -> bool {
    a.instance_id == b.instance_id && a.pin == b.pin
}

/// Two wires joining the same two pins, in either direction.
pub fn same_wire(a: &Wire, b: &Wire) -> bool {
    (same_pin(&a.a, &b.a) && same_pin(&a.b, &b.b)) || (same_pin(&a.a, &b.b) && same_pin(&a.b, &b.a))
}

/// Drop duplicate wires, keeping the first of each pin pair.
pub fn dedupe_wires(wires: &[Wire]) -> Vec<Wire> {
    let mut seen = HashSet::new();
    wires
        .iter()
        .filter(|wire| {
            let ka = format!("{}:{}", wire.a.instance_id, wire.a.pin);
            let kb = format!("{}:{}", wire.b.instance_id, wire.b.pin);
            let key = if ka < kb {
                format!("{ka}|{kb}")
            } else {
                format!("{kb}|{ka}")
            };
            seen.insert(key)
        })
        .cloned()
        .collect()
}

/// Breadboards draw behind everything else, so a late board cannot swallow parts
/// placed before it. Within each group placement order is preserved.
pub fn render_order_of(
    instances: &[Instance],
    is_background: impl Fn(&Instance) -> bool,
) -> Vec<Instance> {
    let mut out = instances.to_vec();
    out.sort_by_key(|instance| if is_background(instance) { 0 } else { 1 });
    out
}

/// What a copy of the selection should hold, positioned relative to its top-left.
/// `attrs_of` is injected so the catalogue can supply defaults.
pub fn clipboard_of(
    instances: &[Instance],
    selected_ids: &[String],
    attrs_of: impl Fn(&Instance) -> HashMap<String, String>,
) -> Option<Clipboard> {
    let parts: Vec<&Instance> = instances
        .iter()
        .filter(|instance| selected_ids.iter().any(|id| id == &instance.id))
        .collect();
    if parts.is_empty() {
        return None;
    }
    let x = parts
        .iter()
        .map(|part| part.x)
        .fold(f64::INFINITY, f64::min);
    let y = parts
        .iter()
        .map(|part| part.y)
        .fold(f64::INFINITY, f64::min);
    Some(Clipboard {
        x,
        y,
        parts: parts
            .iter()
            .map(|part| ClipboardPart {
                kind: part.kind.clone(),
                dx: part.x - x,
                dy: part.y - y,
                attrs: Some(attrs_of(part)),
                rotate: Some(part.rotate.unwrap_or(0.0)),
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(overrides: Option<&str>) -> Instance {
        Instance {
            id: overrides.unwrap_or("p1").into(),
            kind: "led".into(),
            x: 10.0,
            y: 20.0,
            rotate: None,
            attrs: None,
        }
    }

    fn wire(id: &str, a: &str, b: &str) -> Wire {
        Wire {
            id: id.into(),
            a: PinRef {
                instance_id: a.into(),
                pin: "1".into(),
            },
            b: PinRef {
                instance_id: b.into(),
                pin: "2".into(),
            },
            waypoints: Vec::new(),
            colour: None,
        }
    }

    #[test]
    fn snap_uses_the_fine_grid() {
        assert_eq!(snap_to_grid(0.0), 0.0);
        assert_eq!(snap_to_grid(5.9), 0.0);
        assert_eq!(snap_to_grid(6.1), 12.0);
        assert_eq!(snap_to_grid(-5.9), -0.0);
        assert_eq!(snap_to_grid(-7.0), -12.0);
    }

    #[test]
    fn snapshots_do_not_alias_later_edits() {
        let mut instance = part(None);
        instance.attrs = Some(HashMap::from([("color".into(), "blue".into())]));
        let mut routed = wire("w1", "p1", "p2");
        routed.waypoints.push(Point::new(1.0, 2.0));
        let snapshot = snapshot_of(&[instance.clone()], &[routed.clone()]);

        instance.x = 999.0;
        instance
            .attrs
            .as_mut()
            .unwrap()
            .insert("color".into(), "green".into());
        routed.waypoints[0].x = 50.0;

        assert_eq!(snapshot.instances[0].x, 10.0);
        assert_eq!(
            snapshot.instances[0].attrs.as_ref().unwrap()["color"],
            "blue"
        );
        assert_eq!(snapshot.wires[0].waypoints[0], Point::new(1.0, 2.0));
    }

    #[test]
    fn history_keeps_the_newest_snapshots() {
        let mut history = Vec::new();
        for _ in 0..205 {
            history = history_push(
                &history,
                Snapshot {
                    instances: Vec::new(),
                    wires: Vec::new(),
                },
            );
        }
        assert_eq!(history.len(), HISTORY_LIMIT);
    }

    #[test]
    fn dedupe_keeps_the_first_of_each_pair_in_either_direction() {
        let reversed = Wire {
            id: "w2".into(),
            a: PinRef {
                instance_id: "b".into(),
                pin: "2".into(),
            },
            b: PinRef {
                instance_id: "a".into(),
                pin: "1".into(),
            },
            waypoints: Vec::new(),
            colour: None,
        };
        let kept = dedupe_wires(&[wire("w1", "a", "b"), reversed, wire("w3", "a", "c")]);
        assert_eq!(
            kept.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            vec!["w1", "w3"]
        );
    }

    #[test]
    fn render_order_keeps_background_boards_first() {
        let led = part(Some("led"));
        let mut board = part(Some("board"));
        board.kind = "breadboard".into();
        let mut resistor = part(Some("res"));
        resistor.kind = "resistor".into();
        let mut full = part(Some("full"));
        full.kind = "breadboard-full".into();
        let ordered = render_order_of(&[led, board, resistor, full], |instance| {
            instance.kind == "breadboard" || instance.kind == "breadboard-full"
        });
        assert_eq!(
            ordered.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["board", "full", "led", "res"]
        );
    }

    #[test]
    fn clipboard_is_relative_to_its_top_left() {
        let mut a = part(Some("a"));
        a.x = 100.0;
        a.y = 50.0;
        a.rotate = Some(90.0);
        a.attrs = Some(HashMap::from([("color".into(), "blue".into())]));
        let mut b = part(Some("b"));
        b.kind = "resistor".into();
        b.x = 140.0;
        b.y = 90.0;
        b.attrs = Some(HashMap::from([("value".into(), "1000".into())]));
        let clip = clipboard_of(&[a, b], &["a".into(), "b".into()], |instance| {
            instance.attrs.clone().unwrap_or_default()
        })
        .unwrap();
        assert_eq!((clip.x, clip.y), (100.0, 50.0));
        assert_eq!(clip.parts.len(), 2);
        assert_eq!(clip.parts[1].dx, 40.0);
        assert_eq!(clip.parts[1].dy, 40.0);
        assert_eq!(clip.parts[0].attrs.as_ref().unwrap()["color"], "blue");
    }

    #[test]
    fn clipboard_is_empty_without_a_selection() {
        assert!(clipboard_of(&[part(None)], &[], |_| HashMap::new()).is_none());
    }
}
