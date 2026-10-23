//! Orthogonal wire routing: the only place that decides where bends go.
//!
//! Every route is a polyline of horizontal and vertical segments. The saved form is
//! a list of relative `v<px>`/`h<px>` moves, so a wire stays sensible when its part
//! moves.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

fn elbow_for(index: usize, a: Point, b: Point) -> Point {
    if index % 2 == 0 {
        Point::new(a.x, b.y)
    } else {
        Point::new(b.x, a.y)
    }
}

/// The one bend of the current live leg, or `None` when the points already share an axis.
pub fn leg_bend(index: usize, last: Point, end: Point) -> Option<Point> {
    if last.x == end.x || last.y == end.y {
        None
    } else {
        Some(elbow_for(index, last, end))
    }
}

/// The wire so far, plus the live leg ending at `end`.
pub fn live_points(from: Point, waypoints: &[Point], end: Point) -> Vec<Point> {
    let last = waypoints.last().copied().unwrap_or(from);
    let bend = leg_bend(waypoints.len(), last, end);
    let mut out = Vec::with_capacity(waypoints.len() + 3);
    out.push(from);
    out.extend_from_slice(waypoints);
    if let Some(bend) = bend {
        out.push(bend);
    }
    out.push(end);
    out
}

/// The full orthogonal polyline through `pts`, inserting one bend between any two
/// points that do not already share a row or column.
pub fn expand_path(pts: &[Point]) -> Vec<Point> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let mut out = vec![pts[0]];
    for (index, pair) in pts.windows(2).enumerate() {
        let a = pair[0];
        let b = pair[1];
        if a.x == b.x || a.y == b.y {
            out.push(b);
        } else {
            out.push(elbow_for(index, a, b));
            out.push(b);
        }
    }
    out
}

/// Route through a list of points, always at right angles.
pub fn route_path(pts: &[Point]) -> String {
    let line = expand_path(pts);
    if line.len() < 2 {
        return String::new();
    }
    let mut out = format!("M {} {}", line[0].x, line[0].y);
    for point in &line[1..] {
        out.push_str(&format!(" L {} {}", point.x, point.y));
    }
    out
}

fn round_tenth(n: f64) -> f64 {
    (n * 10.0).round() / 10.0
}

/// Wokwi's route mini-language: relative moves from a pin.
pub fn encode_route(a: Point, b: Point, waypoints: &[Point]) -> Vec<String> {
    let mut all = Vec::with_capacity(waypoints.len() + 2);
    all.push(a);
    all.extend_from_slice(waypoints);
    all.push(b);
    let line = expand_path(&all);
    let mut out = Vec::new();
    for pair in line.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let dy = pair[1].y - pair[0].y;
        if dx == 0.0 && dy == 0.0 {
            continue;
        }
        if dx == 0.0 {
            out.push(format!("v{}", round_tenth(dy)));
        } else if dy == 0.0 {
            out.push(format!("h{}", round_tenth(dx)));
        }
    }
    out
}

/// Decode a saved route into the points strictly between the two pins.
pub fn decode_route(from: Point, route: &[String]) -> Vec<Point> {
    let mut points = Vec::new();
    let mut x = from.x;
    let mut y = from.y;
    for instruction in route {
        let Some((direction, amount)) = instruction.split_at_checked(1) else {
            continue;
        };
        if direction != "v" && direction != "h" {
            continue;
        }
        let Ok(value) = amount.parse::<f64>() else {
            continue;
        };
        if direction == "v" {
            y += value;
        } else {
            x += value;
        }
        points.push(Point::new(x, y));
    }
    points.pop();
    points
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn segments(points: &[Point]) -> Vec<(f64, f64)> {
        points
            .windows(2)
            .map(|pair| (pair[1].x - pair[0].x, pair[1].y - pair[0].y))
            .collect()
    }

    #[test]
    fn never_produces_a_diagonal() {
        let cases = [
            vec![p(0.0, 0.0), p(50.0, 50.0)],
            vec![p(0.0, 0.0), p(10.0, 90.0), p(90.0, 10.0)],
            vec![p(0.0, 0.0), p(40.0, 40.0), p(80.0, 0.0), p(120.0, 40.0)],
            vec![p(185.0, 89.0), p(185.0, 205.65), p(430.0, 205.65)],
            vec![p(0.0, 0.0), p(0.0, 40.0), p(60.0, 40.0), p(60.0, 0.0)],
        ];
        for points in cases {
            for (dx, dy) in segments(&expand_path(&points)) {
                assert!(dx == 0.0 || dy == 0.0, "diagonal in {points:?}");
            }
        }
    }

    #[test]
    fn leaves_an_axis_aligned_route_alone() {
        let straight = [p(0.0, 0.0), p(0.0, 50.0), p(90.0, 50.0)];
        assert_eq!(expand_path(&straight), straight);
    }

    #[test]
    fn adds_one_bend_and_alternates_the_corner() {
        assert_eq!(
            expand_path(&[p(0.0, 0.0), p(50.0, 50.0)]),
            vec![p(0.0, 0.0), p(0.0, 50.0), p(50.0, 50.0)]
        );
        assert_eq!(
            expand_path(&[p(0.0, 0.0), p(50.0, 50.0), p(90.0, 10.0)]),
            vec![
                p(0.0, 0.0),
                p(0.0, 50.0),
                p(50.0, 50.0),
                p(90.0, 50.0),
                p(90.0, 10.0),
            ]
        );
    }

    #[test]
    fn writes_a_polyline_without_diagonals() {
        assert_eq!(
            route_path(&[p(0.0, 0.0), p(50.0, 50.0)]),
            "M 0 0 L 0 50 L 50 50"
        );
        assert_eq!(route_path(&[p(5.0, 5.0)]), "");
    }

    #[test]
    fn survives_the_saved_mini_language() {
        let from = p(185.0, 89.0);
        let to = p(605.0, 212.0);
        let waypoints = [p(185.0, 205.6), p(488.8, 205.6)];
        let encoded = encode_route(from, to, &waypoints);
        assert!(!encoded.is_empty());
        for movement in &encoded {
            assert!(
                movement
                    .chars()
                    .next()
                    .is_some_and(|c| c == 'v' || c == 'h'),
                "{movement}"
            );
        }
        let expected = expand_path(&[from, waypoints[0], waypoints[1], to]);
        let expected = &expected[1..expected.len() - 1];
        let decoded = decode_route(from, &encoded);
        assert_eq!(decoded.len(), expected.len());
        for (actual, wanted) in decoded.iter().zip(expected) {
            assert!((actual.x - wanted.x).abs() < 1.0);
            assert!((actual.y - wanted.y).abs() < 1.0);
        }
    }

    #[test]
    fn decodes_the_bend_between_the_pins() {
        let from = p(10.0, 10.0);
        let to = p(90.0, 60.0);
        assert_eq!(
            decode_route(from, &encode_route(from, to, &[])),
            vec![p(10.0, 60.0)]
        );
    }

    #[test]
    fn live_points_adds_only_a_bend_that_is_needed() {
        assert_eq!(
            live_points(p(0.0, 0.0), &[], p(10.0, 20.0)),
            vec![p(0.0, 0.0), p(0.0, 20.0), p(10.0, 20.0)]
        );
        assert_eq!(
            live_points(p(0.0, 0.0), &[], p(0.0, 20.0)),
            vec![p(0.0, 0.0), p(0.0, 20.0)]
        );
    }
}
