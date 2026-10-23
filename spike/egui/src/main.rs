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

use eframe::egui::{self, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};

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

#[derive(Clone)]
struct Pin {
    name: &'static str,
    offset: Vec2,
}

#[derive(Clone)]
struct Part {
    name: &'static str,
    center: Pos2,
    size: Vec2,
    color: Color32,
    pins: Vec<Pin>,
}

impl Part {
    fn bounds(&self) -> Rect {
        Rect::from_center_size(self.center, self.size)
    }

    fn pin_world(&self, index: usize) -> Pos2 {
        self.center + self.pins[index].offset
    }
}

type PinRef = (usize, usize);
type Wire = (PinRef, PinRef);

struct Spike {
    camera: Camera,
    parts: Vec<Part>,
    wires: Vec<Wire>,
    wire_start: Option<PinRef>,
    selected: Option<usize>,
    dragging: Option<usize>,
    hovered_pin: Option<PinRef>,
}

impl Default for Spike {
    fn default() -> Self {
        Self {
            camera: Camera::default(),
            parts: sample_parts(),
            wires: Vec::new(),
            wire_start: None,
            selected: None,
            dragging: None,
            hovered_pin: None,
        }
    }
}

fn sample_parts() -> Vec<Part> {
    vec![
        Part {
            name: "Uno",
            center: Pos2::new(-320.0, -120.0),
            size: Vec2::new(300.0, 210.0),
            color: Color32::from_rgb(45, 116, 166),
            pins: vec![
                Pin {
                    name: "2",
                    offset: Vec2::new(70.0, -105.0),
                },
                Pin {
                    name: "3",
                    offset: Vec2::new(90.0, -105.0),
                },
                Pin {
                    name: "5V",
                    offset: Vec2::new(-90.0, -105.0),
                },
                Pin {
                    name: "GND",
                    offset: Vec2::new(-120.0, -105.0),
                },
            ],
        },
        Part {
            name: "Breadboard",
            center: Pos2::new(180.0, -60.0),
            size: Vec2::new(420.0, 170.0),
            color: Color32::from_rgb(220, 216, 196),
            pins: vec![
                Pin {
                    name: "a1",
                    offset: Vec2::new(-170.0, -35.0),
                },
                Pin {
                    name: "a2",
                    offset: Vec2::new(-140.0, -35.0),
                },
                Pin {
                    name: "e5",
                    offset: Vec2::new(-50.0, 35.0),
                },
            ],
        },
        Part {
            name: "LED",
            center: Pos2::new(30.0, 130.0),
            size: Vec2::new(44.0, 64.0),
            color: Color32::from_rgb(210, 54, 54),
            pins: vec![
                Pin {
                    name: "A",
                    offset: Vec2::new(-12.0, 32.0),
                },
                Pin {
                    name: "C",
                    offset: Vec2::new(12.0, 32.0),
                },
            ],
        },
        Part {
            name: "Resistor",
            center: Pos2::new(-120.0, 140.0),
            size: Vec2::new(120.0, 32.0),
            color: Color32::from_rgb(150, 110, 72),
            pins: vec![
                Pin {
                    name: "1",
                    offset: Vec2::new(-60.0, 0.0),
                },
                Pin {
                    name: "2",
                    offset: Vec2::new(60.0, 0.0),
                },
            ],
        },
    ]
}

fn hit_part(parts: &[Part], world: Pos2) -> Option<usize> {
    // Topmost last-drawn part wins.
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
                        Some(start) if start != pin => self.wires.push((start, pin)),
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
                );
            }

            for &(a, b) in &self.wires {
                let pa = self
                    .camera
                    .world_to_screen(rect, self.parts[a.0].pin_world(a.1));
                let pb = self
                    .camera
                    .world_to_screen(rect, self.parts[b.0].pin_world(b.1));
                painter.line_segment([pa, pb], Stroke::new(2.5, Color32::from_rgb(108, 203, 123)));
            }

            if let (Some(start), Some(pointer)) = (self.wire_start, pointer) {
                let pa = self
                    .camera
                    .world_to_screen(rect, self.parts[start.0].pin_world(start.1));
                painter.line_segment(
                    [pa, pointer],
                    Stroke::new(2.0, Color32::from_rgb(217, 164, 65)),
                );
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
) {
    let min = camera.world_to_screen(rect, part.bounds().min);
    let max = camera.world_to_screen(rect, part.bounds().max);
    let screen = Rect::from_min_max(min, max);
    painter.rect_filled(screen, 4.0, part.color);
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
    painter.text(
        screen.center(),
        Align2::CENTER_CENTER,
        part.name,
        FontId::proportional(13.0),
        Color32::WHITE,
    );

    for (index, pin) in part.pins.iter().enumerate() {
        let p = camera.world_to_screen(rect, part.pin_world(index));
        let hovered = hovered_pin.is_some_and(|(_, pin_index)| pin_index == index);
        let radius = if hovered { 7.0 } else { 4.5 };
        painter.circle_filled(
            p,
            radius,
            if hovered {
                Color32::from_rgb(217, 164, 65)
            } else {
                Color32::from_rgb(108, 203, 123)
            },
        );
        if hovered {
            painter.text(
                p + Vec2::new(8.0, -8.0),
                Align2::LEFT_BOTTOM,
                pin.name,
                FontId::monospace(11.0),
                Color32::WHITE,
            );
        }
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
        let parts = sample_parts();
        let pin = parts[0].pin_world(0);
        assert_eq!(hit_pin(&parts, pin, PIN_HIT_RADIUS), Some((0, 0)));
        assert_eq!(hit_part(&parts, pin), Some(0));
    }

    #[test]
    fn a_pin_outside_the_hit_radius_is_not_hit() {
        let parts = sample_parts();
        let pin = parts[0].pin_world(0);
        assert_eq!(
            hit_pin(
                &parts,
                pin + Vec2::new(PIN_HIT_RADIUS + 1.0, 0.0),
                PIN_HIT_RADIUS
            ),
            None
        );
    }
}
