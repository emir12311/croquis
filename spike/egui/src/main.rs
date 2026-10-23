//! Spike A: pan/zoom, part hit-testing, pin hit-testing and wiring on egui.
//!
//! This is deliberately not the v0.3 application. It is a throwaway measurement
//! bed for the risks in the rewrite plan: can egui give us a crisp, responsive
//! circuit surface with stable world/screen coordinates, and can pin and part
//! hit-testing live in data rather than in a DOM?
//!
//! Run with:
//!
//!     cargo run --manifest-path spike/egui/Cargo.toml
//!
//! A display server is needed for the binary; `cargo test` exercises the pure
//! geometry without one.

use std::collections::HashMap;

use croquis_core::route::{expand_path, Point as CorePoint};
use eframe::egui::{self, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};

mod art;
mod scene;

use scene::{Part, PinRef, Wire};

const GRID: f32 = 24.0;
const PIN_HIT_RADIUS: f32 = 9.0;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("Croquis — egui spike A"),
        ..Default::default()
    };
    eframe::run_native(
        "croquis-egui-spike",
        options,
        Box::new(|_cc| Ok(Box::new(Spike::default()))),
    )
}

#[derive(Clone, Copy)]
struct Camera {
    zoom: f32,
    pan: Vec2,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: Vec2::ZERO,
        }
    }
}

impl Camera {
    fn world_to_screen(self, rect: Rect, world: Pos2) -> Pos2 {
        rect.center() + world.to_vec2() * self.zoom + self.pan
    }

    fn screen_to_world(self, rect: Rect, screen: Pos2) -> Pos2 {
        ((screen - rect.center() - self.pan) / self.zoom).to_pos2()
    }
}

struct Spike {
    camera: Camera,
    parts: Vec<Part>,
    wires: Vec<Wire>,
    wire_start: Option<PinRef>,
    selected: Option<usize>,
    dragging: Option<usize>,
    hovered_pin: Option<PinRef>,
    textures: HashMap<String, egui::TextureHandle>,
}

impl Default for Spike {
    fn default() -> Self {
        let (parts, wires) = scene::load_blink();
        Self {
            camera: Camera::default(),
            parts,
            wires,
            wire_start: None,
            selected: None,
            dragging: None,
            hovered_pin: None,
            textures: HashMap::new(),
        }
    }
}

fn hit_part(parts: &[Part], world: Pos2) -> Option<usize> {
    parts
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, part)| part.bounds().contains(world).then_some(index))
}

fn hit_pin(parts: &[Part], world: Pos2, radius: f32) -> Option<PinRef> {
    let radius_sq = radius * radius;
    for (part_index, part) in parts.iter().enumerate().rev() {
        for (pin_index, _) in part.pins.iter().enumerate().rev() {
            if part.pin_world(pin_index).distance_sq(world) <= radius_sq {
                return Some((part_index, pin_index));
            }
        }
    }
    None
}

impl eframe::App for Spike {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        for part in &self.parts {
            if self.textures.contains_key(&part.kind) {
                continue;
            }
            let width = (part.size.x * 3.0).max(1.0) as u32;
            let height = (part.size.y * 3.0).max(1.0) as u32;
            if let Some(image) = art::rasterize(&part.svg, width, height) {
                let texture = ctx.load_texture(
                    format!("part-{}", part.kind),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                self.textures.insert(part.kind.clone(), texture);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.wire_start = None;
        }

        egui::Panel::right("spike-info")
            .exact_size(250.0)
            .show(root, |ui| {
                ui.heading("egui spike A");
                ui.label("Canvas, pan/zoom, pin hit-testing, wiring.");
                ui.separator();
                ui.label(format!("Zoom: {:.0}%", self.camera.zoom * 100.0));
                ui.label(format!(
                    "Frame: {:.1} ms",
                    ctx.input(|i| i.stable_dt) * 1000.0
                ));
                ui.label(format!(
                    "Hovered pin: {}",
                    self.hovered_pin
                        .map(|(part, pin)| {
                            format!(
                                "{} · {}",
                                self.parts[part].name, self.parts[part].pins[pin].name
                            )
                        })
                        .unwrap_or_else(|| "—".into())
                ));
                ui.separator();
                ui.label("Drag a part with the left button.");
                ui.label("Drag the canvas with the middle or right button.");
                ui.label("Wheel zooms toward the pointer.");
                ui.label("Click a pin, then another, to wire them.");
                ui.label("Esc cancels a wire.");
                if ui.button("Reset view").clicked() {
                    self.camera = Camera::default();
                }
                if ui.button("Clear wires").clicked() {
                    self.wires.clear();
                    self.wire_start = None;
                }
                ui.separator();
                ui.label(format!("Wires: {}", self.wires.len()));
                if let Some(selected) = self.selected {
                    ui.label(format!("Selected: {}", self.parts[selected].name));
                }
            });

        egui::CentralPanel::default().show(root, |ui| {
            let (response, painter) =
                ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
            let rect = response.rect;

            painter.rect_filled(rect, 0.0, Color32::from_rgb(26, 26, 28));
            draw_grid(&painter, rect, self.camera);

            let pointer = response.hover_pos();
            let world_pointer = pointer.map(|p| self.camera.screen_to_world(rect, p));
            self.hovered_pin = world_pointer
                .and_then(|p| hit_pin(&self.parts, p, PIN_HIT_RADIUS / self.camera.zoom));

            if response.dragged_by(PointerButton::Middle)
                || response.dragged_by(PointerButton::Secondary)
            {
                self.camera.pan += response.drag_delta();
            }

            if let Some(pointer) = pointer {
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    let before = self.camera.screen_to_world(rect, pointer);
                    self.camera.zoom =
                        (self.camera.zoom * (1.0 + scroll * 0.0015)).clamp(0.25, 4.0);
                    let after = self.camera.world_to_screen(rect, before);
                    self.camera.pan += pointer - after;
                }
            }

            if response.drag_started_by(PointerButton::Primary) {
                if let Some(world) = world_pointer {
                    if hit_pin(&self.parts, world, PIN_HIT_RADIUS / self.camera.zoom).is_none() {
                        self.dragging = hit_part(&self.parts, world);
                        self.selected = self.dragging;
                    }
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                if let Some(index) = self.dragging {
                    self.parts[index].center += response.drag_delta() / self.camera.zoom;
                }
            }
            if response.drag_stopped() {
                self.dragging = None;
            }

            if response.clicked_by(PointerButton::Primary) {
                if let Some(pin) = self.hovered_pin {
                    match self.wire_start.take() {
                        Some(start) if start != pin => self.wires.push(Wire {
                            a: start,
                            b: pin,
                            waypoints: Vec::new(),
                        }),
                        Some(_) => {}
                        None => self.wire_start = Some(pin),
                    }
                } else if let Some(world) = world_pointer {
                    self.selected = hit_part(&self.parts, world);
                    self.wire_start = None;
                }
            }

            for (index, part) in self.parts.iter().enumerate() {
                draw_part(
                    &painter,
                    rect,
                    self.camera,
                    part,
                    self.selected == Some(index),
                    self.hovered_pin
                        .filter(|(part_index, _)| *part_index == index),
                    self.textures.get(&part.kind),
                );
            }

            for wire in &self.wires {
                let a = self
                    .camera
                    .world_to_screen(rect, self.parts[wire.a.0].pin_world(wire.a.1));
                let b = self
                    .camera
                    .world_to_screen(rect, self.parts[wire.b.0].pin_world(wire.b.1));
                let mut route = vec![CorePoint::new(a.x as f64, a.y as f64)];
                route.extend(
                    wire.waypoints
                        .iter()
                        .map(|point| CorePoint::new(point.x as f64, point.y as f64)),
                );
                route.push(CorePoint::new(b.x as f64, b.y as f64));
                let line = expand_path(&route);
                for pair in line.windows(2) {
                    painter.line_segment(
                        [
                            Pos2::new(pair[0].x as f32, pair[0].y as f32),
                            Pos2::new(pair[1].x as f32, pair[1].y as f32),
                        ],
                        Stroke::new(2.5, Color32::from_rgb(108, 203, 123)),
                    );
                }
            }

            if let (Some(start), Some(pointer)) = (self.wire_start, pointer) {
                let pa = self
                    .camera
                    .world_to_screen(rect, self.parts[start.0].pin_world(start.1));
                let preview = expand_path(&[
                    CorePoint::new(pa.x as f64, pa.y as f64),
                    CorePoint::new(pointer.x as f64, pointer.y as f64),
                ]);
                for pair in preview.windows(2) {
                    painter.line_segment(
                        [
                            Pos2::new(pair[0].x as f32, pair[0].y as f32),
                            Pos2::new(pair[1].x as f32, pair[1].y as f32),
                        ],
                        Stroke::new(2.0, Color32::from_rgb(217, 164, 65)),
                    );
                }
            }
        });
    }
}

fn draw_grid(painter: &egui::Painter, rect: Rect, camera: Camera) {
    let spacing = if camera.zoom < 0.5 { GRID * 4.0 } else { GRID };
    let step = spacing * camera.zoom;
    if step < 4.0 {
        return;
    }
    let origin = camera.world_to_screen(rect, Pos2::ZERO);
    let start_x = rect.left() + (origin.x - rect.left()).rem_euclid(step);
    let start_y = rect.top() + (origin.y - rect.top()).rem_euclid(step);
    let stroke = Stroke::new(1.0, Color32::from_rgb(42, 42, 46));
    let mut x = start_x;
    while x <= rect.right() {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            stroke,
        );
        x += step;
    }
    let mut y = start_y;
    while y <= rect.bottom() {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            stroke,
        );
        y += step;
    }
}

fn draw_part(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera,
    part: &Part,
    selected: bool,
    hovered_pin: Option<PinRef>,
    texture: Option<&egui::TextureHandle>,
) {
    let min = camera.world_to_screen(rect, part.bounds().min);
    let max = camera.world_to_screen(rect, part.bounds().max);
    let screen = Rect::from_min_max(min, max);
    if let Some(texture) = texture {
        painter.image(
            texture.id(),
            screen,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        painter.rect_filled(screen, 4.0, part.color);
    }
    let border = if selected {
        Color32::from_rgb(217, 164, 65)
    } else {
        Color32::from_rgb(90, 90, 96)
    };
    painter.rect_stroke(
        screen,
        4.0,
        Stroke::new(if selected { 2.5 } else { 1.0 }, border),
        egui::StrokeKind::Inside,
    );
    for (index, pin) in part.pins.iter().enumerate() {
        let hovered = hovered_pin.is_some_and(|(_, pin_index)| pin_index == index);
        // Match the app: no markers at rest, and only the hovered pin gets an
        // amber square. Size it in world space (10 px at 100%) so it stays
        // proportional to the part instead of appearing to grow when zoomed out.
        if !hovered {
            continue;
        }
        let p = camera.world_to_screen(rect, part.pin_world(index));
        let square = Rect::from_center_size(p, Vec2::splat(10.0 * camera.zoom));
        painter.rect_filled(square, 0.0, Color32::from_rgb(217, 164, 65));
        painter.rect_stroke(
            square,
            0.0,
            Stroke::new(1.0, Color32::from_rgb(26, 26, 28)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            p + Vec2::new(8.0, -8.0),
            Align2::LEFT_BOTTOM,
            &pin.name,
            FontId::monospace(11.0),
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::new(800.0, 600.0))
    }

    #[test]
    fn screen_and_world_round_trip() {
        let camera = Camera {
            zoom: 1.7,
            pan: Vec2::new(31.0, -12.0),
        };
        let world = Pos2::new(-42.0, 91.0);
        let screen = camera.world_to_screen(rect(), world);
        let back = camera.screen_to_world(rect(), screen);
        assert!((back.x - world.x).abs() < 0.001);
        assert!((back.y - world.y).abs() < 0.001);
    }

    #[test]
    fn pins_win_over_the_part_body() {
        let parts = scene::load_blink().0;
        assert!(!parts.is_empty(), "Blink preset should load");
        let pin = parts[0].pin_world(0);
        assert_eq!(hit_pin(&parts, pin, PIN_HIT_RADIUS), Some((0, 0)));
        assert_eq!(hit_part(&parts, pin), Some(0));
    }

    #[test]
    fn a_pin_outside_the_hit_radius_is_not_hit() {
        let parts = scene::load_blink().0;
        assert!(!parts.is_empty(), "Blink preset should load");
        let pin = parts[0].pin_world(0);
        assert_eq!(
            hit_pin(&parts, pin + Vec2::new(100.0, 100.0), PIN_HIT_RADIUS),
            None
        );
    }

    #[test]
    fn blink_assets_load_and_rasterise() {
        let parts = scene::load_blink().0;
        assert_eq!(parts.len(), 3, "Blink has Uno, resistor and LED");
        for part in &parts {
            assert!(!part.id.is_empty());
            assert!(!part.pins.is_empty(), "{} has pins", part.kind);
            let image = art::rasterize(&part.svg, part.size.x as u32, part.size.y as u32)
                .unwrap_or_else(|| panic!("{} rasterises", part.kind));
            let visible = image.pixels.iter().filter(|pixel| pixel.a() > 0).count();
            assert!(visible > 100, "{} has visible pixels", part.kind);
        }
    }
}
