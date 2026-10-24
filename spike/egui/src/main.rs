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
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

use croquis_core::catalogue::parts;
use croquis_core::netlist::{LedState, OutputState, RgbState};
use croquis_core::route::{expand_path, Point as CorePoint};
use croquis_sim::engine::{self, SimEvent};
use eframe::egui::{self, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};

mod art;
mod editor;
mod scene;

use scene::{Part, PinRef, Wire};

const GRID: f32 = 24.0;
const PIN_HIT_RADIUS: f32 = 9.0;

fn configure_style(ctx: &egui::Context) {
    use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

    let bg = Color32::from_rgb(0x1a, 0x1a, 0x1c);
    let surface = Color32::from_rgb(0x21, 0x21, 0x24);
    let raised = Color32::from_rgb(0x2a, 0x2a, 0x2e);
    let border = Color32::from_rgb(0x35, 0x35, 0x3a);
    let text = Color32::from_rgb(0xe6, 0xe4, 0xe0);
    let muted = Color32::from_rgb(0xa6, 0xa4, 0xa0);
    let accent = Color32::from_rgb(0xd9, 0xa4, 0x41);

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = bg;
    visuals.window_fill = surface;
    visuals.extreme_bg_color = Color32::from_rgb(0x16, 0x16, 0x18);
    visuals.faint_bg_color = surface;
    visuals.window_stroke = Stroke::new(1.0, border);
    visuals.window_corner_radius = CornerRadius::same(4);
    visuals.menu_corner_radius = CornerRadius::same(4);
    visuals.override_text_color = Some(text);
    visuals.selection.bg_fill = accent;
    visuals.selection.stroke = Stroke::new(1.0, accent);
    visuals.hyperlink_color = accent;

    visuals.widgets.noninteractive.bg_fill = surface;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, muted);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(2);
    visuals.widgets.inactive.bg_fill = raised;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(2);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x35, 0x33, 0x2e);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, text);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(2);
    visuals.widgets.active.bg_fill = accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, bg);
    visuals.widgets.active.corner_radius = CornerRadius::same(2);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(10.0, 4.0);
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Button,
        FontId::new(13.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(11.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(15.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(12.5, FontFamily::Monospace),
    );
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title("Croquis"),
        ..Default::default()
    };
    eframe::run_native(
        "croquis-egui-spike",
        options,
        Box::new(|cc| {
            configure_style(&cc.egui_ctx);
            Ok(Box::new(Spike::default()))
        }),
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
    led_states: HashMap<String, LedState>,
    outputs: HashMap<String, OutputState>,
    rgb_leds: HashMap<String, RgbState>,
    sketch: String,
    next_id: usize,
    sim: Option<engine::Simulator>,
    events: Option<mpsc::Receiver<SimEvent>>,
    running: bool,
    avr_pins: HashMap<String, bool>,
    serial: String,
    run_error: Option<String>,
    path: Option<PathBuf>,
    file_error: Option<String>,
    presets: Vec<(String, PathBuf)>,
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
            led_states: HashMap::new(),
            outputs: HashMap::new(),
            rgb_leds: HashMap::new(),
            sketch: scene::blink_sketch(),
            next_id: 0,
            sim: None,
            events: None,
            running: false,
            avr_pins: HashMap::new(),
            serial: String::new(),
            run_error: None,
            path: None,
            file_error: None,
            presets: scene::presets(),
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


fn texture_key(
    part: &Part,
    lit: bool,
    outputs: &HashMap<String, OutputState>,
    rgb_leds: &HashMap<String, RgbState>,
) -> String {
    if part.kind == "led" {
        return format!("led:{lit}");
    }
    if part.kind == "led-bar-graph" {
        let values = outputs.get(&part.id).map(|out| out.values.as_slice()).unwrap_or(&[]);
        let bits: String = values
            .iter()
            .map(|value| if *value != 0 { '1' } else { '0' })
            .collect();
        let color = part.attrs.get("color").map(String::as_str).unwrap_or("red");
        return format!("bar:{color}:{bits}");
    }
    if part.kind == "rgb-led" {
        let value = rgb_leds.get(&part.id);
        return format!(
            "rgb:{:.2}:{:.2}:{:.2}",
            value.map(|v| v.r).unwrap_or(0.0),
            value.map(|v| v.g).unwrap_or(0.0),
            value.map(|v| v.b).unwrap_or(0.0)
        );
    }
    if part.kind == "servo" || part.kind == "stepper-motor" {
        let angle = outputs
            .get(&part.id)
            .and_then(|out| out.angle)
            .unwrap_or(0.0);
        return format!("{}-{:.1}", part.kind, angle);
    }
    if part.kind == "seven-segment" || part.kind == "seven-segment-4" {
        let values = outputs.get(&part.id).map(|out| out.values.as_slice()).unwrap_or(&[]);
        let bits: String = values
            .iter()
            .map(|value| if *value != 0 { '1' } else { '0' })
            .collect();
        return format!("seg:{bits}");
    }
    part.kind.clone()
}

fn ensure_textures(
    ctx: &egui::Context,
    textures: &mut HashMap<String, egui::TextureHandle>,
    parts: &[Part],
    outputs: &HashMap<String, OutputState>,
    rgb_leds: &HashMap<String, RgbState>,
) {
    for part in parts {
        let variants: Vec<(String, bool)> = if part.kind == "led" {
            vec![("led:false".into(), false), ("led:true".into(), true)]
        } else {
            vec![(texture_key(part, false, outputs, rgb_leds), false)]
        };
        for (key, lit) in variants {
            if textures.contains_key(&key) {
                continue;
            }
            let svg = if part.kind == "led" {
                scene::led_state_svg(
                    &part.svg,
                    part.attrs.get("color").map(String::as_str).unwrap_or("red"),
                    if lit { 1.0 } else { 0.0 },
                    lit,
                )
            } else if part.kind == "led-bar-graph" {
                let values = outputs
                    .get(&part.id)
                    .map(|out| out.values.as_slice())
                    .unwrap_or(&[]);
                scene::led_bar_svg(
                    &part.svg,
                    values,
                    part.attrs.get("color").map(String::as_str).unwrap_or("red"),
                )
            } else if part.kind == "rgb-led" {
                let value = rgb_leds.get(&part.id);
                scene::rgb_svg(
                    &part.svg,
                    value.map(|v| v.r).unwrap_or(0.0),
                    value.map(|v| v.g).unwrap_or(0.0),
                    value.map(|v| v.b).unwrap_or(0.0),
                )
            } else if part.kind == "servo" || part.kind == "stepper-motor" {
                let angle = outputs
                    .get(&part.id)
                    .and_then(|out| out.angle)
                    .unwrap_or(0.0);
                scene::servo_svg(&part.svg, angle)
            } else if part.kind == "seven-segment" || part.kind == "seven-segment-4" {
                let values = outputs
                    .get(&part.id)
                    .map(|out| out.values.as_slice())
                    .unwrap_or(&[]);
                scene::seven_segment_svg(
                    &part.svg,
                    values,
                    part.attrs.get("color").map(String::as_str).unwrap_or("red"),
                )
            } else {
                part.svg.clone()
            };
            let width = (part.size.x * 3.0).max(1.0) as u32;
            let height = (part.size.y * 3.0).max(1.0) as u32;
            if let Some(image) = art::rasterize(&svg, width, height) {
                let texture = ctx.load_texture(
                    format!("part-{key}"),
                    image,
                    egui::TextureOptions::LINEAR,
                );
                textures.insert(key, texture);
            }
        }
    }
}

impl Spike {
    fn new_document(&mut self) {
        self.stop_sim();
        self.parts.clear();
        self.wires.clear();
        self.sketch.clear();
        self.selected = None;
        self.wire_start = None;
        self.led_states.clear();
        self.outputs.clear();
        self.rgb_leds.clear();
        self.serial.clear();
        self.run_error = None;
        self.file_error = None;
        self.path = None;
        self.textures.clear();
        self.camera = Camera::default();
    }

    fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.stop_sim();
        let (parts, wires, sketch) = scene::load_diagram_file(&path);
        if parts.is_empty() {
            self.file_error = Some(format!("Could not load {}", path.display()));
            return;
        }
        // Rasterise the new scene before swapping it in. Otherwise the canvas
        // paints the blue fallback for a frame while textures load.
        ensure_textures(ctx, &mut self.textures, &parts, &HashMap::new(), &HashMap::new());
        self.parts = parts;
        self.wires = wires;
        self.sketch = sketch;
        self.path = Some(path);
        self.selected = None;
        self.wire_start = None;
        self.serial.clear();
        self.run_error = None;
        self.file_error = None;
        self.camera = Camera::default();
        let eval = scene::evaluate_scene(&self.parts, &self.wires, &HashMap::new());
        self.led_states = eval.leds;
        self.outputs = eval.outputs;
        self.rgb_leds = eval.rgb_leds;
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Croquis diagram", &["json"])
            .pick_file()
        {
            self.open_path(path, &ctx);
        }
    }

    fn save_to(&mut self, path: PathBuf) {
        match scene::save_diagram_file(&path, &self.parts, &self.wires, &self.sketch) {
            Ok(()) => {
                self.path = Some(path);
                self.file_error = None;
            }
            Err(error) => self.file_error = Some(error),
        }
    }

    fn save(&mut self) {
        if let Some(path) = &self.path {
            self.save_to(path.clone());
        } else {
            self.save_as();
        }
    }

    fn save_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Croquis diagram", &["json"])
            .set_file_name("diagram.json")
            .save_file()
        {
            self.save_to(path);
        }
    }

    fn start_sim(&mut self, ctx: &egui::Context) {
        self.run_error = None;
        self.serial.clear();
        self.avr_pins.clear();
        self.led_states = scene::evaluate_with(&self.parts, &self.wires, &self.avr_pins);
        let workdir = std::env::temp_dir().join(format!("croquis-egui-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&workdir);
        match engine::compile(&self.sketch, &workdir) {
            Ok(firmware) => {
                let (sender, receiver) = mpsc::channel();
                let repaint = ctx.clone();
                let emit: engine::Emit = Arc::new(move |event| {
                    let _ = sender.send(event);
                    repaint.request_repaint();
                });
                match engine::Simulator::start(Path::new(&firmware.elf), emit, false) {
                    Ok(sim) => {
                        self.sim = Some(sim);
                        self.events = Some(receiver);
                        self.running = true;
                    }
                    Err(error) => self.run_error = Some(error),
                }
            }
            Err(error) => self.run_error = Some(error),
        }
    }

    fn stop_sim(&mut self) {
        if let Some(mut sim) = self.sim.take() {
            sim.stop();
        }
        self.events = None;
        self.running = false;
        self.avr_pins.clear();
        let eval = scene::evaluate_scene(&self.parts, &self.wires, &self.avr_pins);
        self.led_states = eval.leds;
        self.outputs = eval.outputs;
        self.rgb_leds = eval.rgb_leds;
    }

    fn poll_engine(&mut self) {
        let mut changed = false;
        if let Some(receiver) = &self.events {
            while let Ok(event) = receiver.try_recv() {
                match event {
                    SimEvent::Pin { port, pin, high } => {
                        self.avr_pins
                            .insert(format!("{}{}", port as char, pin), high);
                        changed = true;
                    }
                    SimEvent::Serial(byte) => {
                        self.serial.push(byte as char);
                        changed = true;
                    }
                    SimEvent::Timing { .. } => {}
                }
            }
        }
        if changed {
            let eval = scene::evaluate_scene(&self.parts, &self.wires, &self.avr_pins);
            self.led_states = eval.leds;
            self.outputs = eval.outputs;
            self.rgb_leds = eval.rgb_leds;
        }
    }
}

impl eframe::App for Spike {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.poll_engine();
        ensure_textures(&ctx, &mut self.textures, &self.parts, &self.outputs, &self.rgb_leds);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.wire_start = None;
        }

        egui::Panel::top("topbar")
            .exact_size(38.0)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("New").clicked() {
                            self.new_document();
                        }
                        if ui.button("Open…").clicked() {
                            self.open_dialog(&ctx);
                        }
                        if ui.button("Save").clicked() {
                            self.save();
                        }
                        if ui.button("Save as…").clicked() {
                            self.save_as();
                        }
                        ui.menu_button("Presets", |ui| {
                            for (title, path) in self.presets.clone() {
                                if ui.button(title).clicked() {
                                    self.open_path(path, &ctx);
                                }
                            }
                        });
                        ui.separator();
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Clear sketch").clicked() {
                            self.sketch.clear();
                        }
                    });
                    ui.menu_button("View", |ui| {
                        if ui.button("Reset view").clicked() {
                            self.camera = Camera::default();
                        }
                        if ui.button("Clear wires").clicked() {
                            self.wires.clear();
                            self.wire_start = None;
                        }
                    });
                    ui.menu_button("Run", |ui| {
                        if self.running {
                            if ui.button("Stop").clicked() {
                                self.stop_sim();
                            }
                        } else if ui.button("Run").clicked() {
                            self.start_sim(&ctx);
                        }
                    });
                    ui.separator();
                    if self.running {
                        if ui.button("Stop").clicked() {
                            self.stop_sim();
                        }
                    } else if ui.button("Run").clicked() {
                        self.start_sim(&ctx);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(if self.running { "Running" } else { "Stopped" });
                        if let Some(error) = &self.file_error {
                            ui.colored_label(Color32::from_rgb(0xe5, 0x48, 0x4d), error);
                        }
                        if let Some(error) = &self.run_error {
                            ui.colored_label(
                                Color32::from_rgb(0xe5, 0x48, 0x4d),
                                "Compile/run failed",
                            );
                            ui.label(error.lines().next().unwrap_or(""));
                        }
                    });
                });
            });

        egui::Panel::bottom("serial")
            .resizable(true)
            .default_size(150.0)
            .show(root, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Serial");
                    if ui.button("Clear").clicked() {
                        self.serial.clear();
                    }
                });
                egui::ScrollArea::vertical()
                    .id_salt("serial-output")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.monospace(&self.serial);
                        if let Some(error) = &self.run_error {
                            ui.colored_label(Color32::from_rgb(230, 120, 100), error);
                        }
                    });
            });

        egui::Panel::right("components")
            .exact_size(220.0)
            .show(root, |ui| {
                ui.heading("Components");
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("component-list")
                    .show(ui, |ui| {
                        for def in parts() {
                            if ui.button(def.label).clicked() {
                                let id = format!("{}-{}", def.kind.as_str(), self.next_id);
                                if let Some(part) =
                                    scene::part_from_kind(def.kind, Pos2::new(0.0, 0.0), id)
                                {
                                    self.parts.push(part);
                                    self.next_id += 1;
                                }
                            }
                        }
                    });
            });

        egui::Panel::left("editor")
            .exact_size(420.0)
            .show(root, |ui| {
                ui.heading("Sketch");
                let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                    let mut job = editor::highlight(text.as_str());
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|fonts| fonts.layout_job(job))
                };
                egui::ScrollArea::vertical()
                    .id_salt("sketch-editor")
                    .max_height(320.0)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut self.sketch)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(20)
                                .layouter(&mut layouter),
                        );
                    });
                ui.separator();
                ui.heading("Canvas");
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
                    self.textures.get(&texture_key(
                        part,
                        self.led_states
                            .get(&part.id)
                            .is_some_and(|state| *state == LedState::Lit),
                        &self.outputs,
                        &self.rgb_leds,
                    )),
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

    #[test]
    fn blink_netlist_lights_its_led() {
        let leds = scene::evaluate_blink();
        assert_eq!(leds.get("led1"), Some(&LedState::Lit));
    }

    #[test]
    fn led_state_uses_the_wokwi_glow_rule() {
        let parts = scene::load_blink().0;
        let led = parts
            .iter()
            .find(|part| part.kind == "led")
            .expect("Blink LED");
        let on = scene::led_state_svg(&led.svg, "blue", 1.0, true);
        assert!(on.contains("display: block"));
        assert!(on.contains("#8080ff"));
        assert!(on.contains("opacity: 1"));
        let dim = scene::led_state_svg(&led.svg, "red", 0.5, true);
        assert!(dim.contains("opacity: 0.65"));
        let off = scene::led_state_svg(&led.svg, "red", 1.0, false);
        assert!(off.contains("display: none"));
    }

    #[test]
    fn blink_led_follows_pin_13() {
        let high = HashMap::from([("B5".to_string(), true)]);
        assert_eq!(
            scene::evaluate_blink_with(&high).get("led1"),
            Some(&LedState::Lit)
        );
        let low = HashMap::from([("B5".to_string(), false)]);
        assert_eq!(
            scene::evaluate_blink_with(&low).get("led1"),
            Some(&LedState::Off)
        );
    }

    #[test]
    fn presets_load_through_the_generic_loader() {
        let presets = scene::presets();
        assert!(presets.len() >= 20, "shipped presets are discoverable");
        let (parts, wires, sketch) = scene::load_diagram_file(&presets[0].1);
        assert!(!parts.is_empty());
        assert!(!wires.is_empty());
        assert!(!sketch.is_empty());
    }

    #[test]
    fn a_loaded_diagram_saves_and_reloads() {
        let path = std::env::temp_dir().join("croquis-roundtrip.json");
        let (parts, wires, sketch) = scene::load_diagram_file(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/button.json"),
        );
        scene::save_diagram_file(&path, &parts, &wires, &sketch).expect("save");
        let (reloaded_parts, reloaded_wires, reloaded_sketch) = scene::load_diagram_file(&path);
        assert_eq!(reloaded_parts.len(), parts.len());
        assert_eq!(reloaded_wires.len(), wires.len());
        assert_eq!(reloaded_sketch, sketch);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_led_bar_patches_lit_and_off_segments() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/wokwi/led-bar-graph.svg");
        let svg = std::fs::read_to_string(path).expect("led bar asset");
        let values = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let patched = scene::led_bar_svg(&svg, &values, "red");
        assert_eq!(patched.matches("#dc012d").count(), 1);
        assert_eq!(patched.matches("#444").count(), 9);
    }

    #[test]
    fn seven_segment_patches_lit_segments() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/wokwi/seven-segment.svg");
        let svg = std::fs::read_to_string(path).expect("7-segment asset");
        let patched = scene::seven_segment_svg(&svg, &[1, 0, 0, 0, 0, 0, 0, 0], "red");
        assert_eq!(patched.matches("#dc012d").count(), 1);
        assert_eq!(patched.matches("#444").count(), 7);
    }
}
