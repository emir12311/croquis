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
use std::time::Instant;

use croquis_core::catalogue::parts;
use croquis_core::netlist::{LedState, OutputState, RgbState, SensorState};
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
    sensors: HashMap<String, SensorState>,
    pin_pulses: HashMap<String, f64>,
    segment_memory: HashMap<String, Vec<u8>>,
    seven_pending: HashMap<String, (Vec<u8>, Instant)>,
    seven_stable: HashMap<String, Vec<u8>>,
    servo_angles: HashMap<String, f64>,
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
    auto_run: bool,
    fit_requested: bool,
}

impl Default for Spike {
    fn default() -> Self {
        let preset = std::env::var("CROQUIS_UI_PRESET").ok().map(PathBuf::from);
        let (parts, wires, sketch, path) = if let Some(path) = preset {
            let (parts, wires, sketch) = scene::load_diagram_file(&path);
            (parts, wires, sketch, Some(path))
        } else {
            let (parts, wires) = scene::load_blink();
            (parts, wires, scene::blink_sketch(), None)
        };
        let auto_run = std::env::var("CROQUIS_UI_AUTORUN").is_ok();
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
            sensors: HashMap::new(),
            pin_pulses: HashMap::new(),
            segment_memory: HashMap::new(),
            seven_pending: HashMap::new(),
            seven_stable: HashMap::new(),
            servo_angles: HashMap::new(),
            sketch,
            next_id: 0,
            sim: None,
            events: None,
            running: false,
            avr_pins: HashMap::new(),
            serial: String::new(),
            run_error: None,
            path,
            file_error: None,
            presets: scene::presets(),
            auto_run,
            fit_requested: true,
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
) -> String {
    if part.kind == "led" {
        return format!("led:{lit}");
    }
    if part.kind == "led-bar-graph" {
        let values = outputs
            .get(&part.id)
            .map(|out| out.values.as_slice())
            .unwrap_or(&[]);
        let bits: String = values
            .iter()
            .map(|value| if *value != 0 { '1' } else { '0' })
            .collect();
        let color = part.attrs.get("color").map(String::as_str).unwrap_or("red");
        return format!("bar:{color}:{bits}");
    }
    if part.kind == "rgb-led" {
        // The glow is drawn live in draw_rgb_glow over the base art. It cannot
        // be baked into the texture per state: the element's SVG carries four
        // Gaussian-blur filters, and one resvg pass over them costs about a
        // third of a second, which would re-create the frame-rate collapse
        // that per-state display art caused.
        return "rgb".into();
    }
    if part.kind == "servo" || part.kind == "stepper-motor" {
        let angle = outputs
            .get(&part.id)
            .and_then(|out| out.angle)
            .unwrap_or(0.0);
        return format!("{}-{:.1}", part.kind, angle);
    }
    if part.kind == "seven-segment" || part.kind == "seven-segment-4" {
        // The lit segments are drawn live in draw_segments over the base art,
        // so the texture must not depend on the state. Keying it on the
        // segments re-rasterised the SVG and uploaded a new texture on every
        // scan, which collapsed the frame rate to a few frames a second.
        return part.kind.clone();
    }
    part.kind.clone()
}

fn ensure_textures(
    ctx: &egui::Context,
    textures: &mut HashMap<String, egui::TextureHandle>,
    parts: &[Part],
    outputs: &HashMap<String, OutputState>,
) {
    for part in parts {
        let variants: Vec<(String, bool)> = if part.kind == "led" {
            vec![("led:false".into(), false), ("led:true".into(), true)]
        } else {
            vec![(texture_key(part, false, outputs), false)]
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
                part.svg.clone()
            } else if part.kind == "servo" || part.kind == "stepper-motor" {
                let angle = outputs
                    .get(&part.id)
                    .and_then(|out| out.angle)
                    .unwrap_or(0.0);
                scene::servo_svg(&part.svg, angle)
            } else if part.kind == "seven-segment" || part.kind == "seven-segment-4" {
                part.svg.clone()
            } else {
                part.svg.clone()
            };
            let width = (part.size.x * 3.0).max(1.0) as u32;
            let height = (part.size.y * 3.0).max(1.0) as u32;
            if let Some(image) = art::rasterize(&svg, width, height) {
                let texture =
                    ctx.load_texture(format!("part-{key}"), image, egui::TextureOptions::LINEAR);
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
        self.sensors.clear();
        self.pin_pulses.clear();
        self.segment_memory.clear();
        self.seven_pending.clear();
        self.seven_stable.clear();
        self.servo_angles.clear();
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
        ensure_textures(ctx, &mut self.textures, &parts, &HashMap::new());
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
        let eval =
            scene::evaluate_scene(&self.parts, &self.wires, &HashMap::new(), &self.pin_pulses);
        self.led_states = eval.leds;
        self.outputs = eval.outputs;
        self.rgb_leds = eval.rgb_leds;
        self.sensors = eval.sensors;
        self.apply_output_memory();
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

    fn apply_output_memory(&mut self) {
        for part in &self.parts {
            if part.kind == "seven-segment-4" {
                let memory = self
                    .segment_memory
                    .entry(part.id.clone())
                    .or_insert_with(|| vec![0; 32]);
                if let Some(state) = self.outputs.get(&part.id) {
                    if state.active {
                        if let Some(digit) = state.digit {
                            let start = digit as usize * 8;
                            if start + 8 <= memory.len() && state.values.len() >= 8 {
                                memory[start..start + 8].copy_from_slice(&state.values[..8]);
                            }
                        }
                    }
                }
                let values = memory.clone();
                if let Some(out) = self.outputs.get_mut(&part.id) {
                    out.values = values;
                    out.active = out.values.iter().any(|value| *value != 0);
                }
            } else if part.kind == "seven-segment" {
                let Some(state) = self.outputs.get(&part.id) else {
                    continue;
                };
                let values = state.values.clone();
                let now = Instant::now();
                let stable = match self.seven_pending.get(&part.id) {
                    Some((pending, at))
                        if *pending == values && now.duration_since(*at).as_millis() >= 8 =>
                    {
                        self.seven_stable.insert(part.id.clone(), values.clone());
                        true
                    }
                    _ => {
                        self.seven_pending
                            .insert(part.id.clone(), (values.clone(), now));
                        false
                    }
                };
                if stable {
                    if let Some(out) = self.outputs.get_mut(&part.id) {
                        out.values = values;
                    }
                } else if let Some(stable) = self.seven_stable.get(&part.id) {
                    if let Some(out) = self.outputs.get_mut(&part.id) {
                        out.values = stable.clone();
                    }
                }
            } else if part.kind == "servo" {
                if let Some(state) = self.outputs.get(&part.id) {
                    if let Some(angle) = state.angle {
                        self.servo_angles.insert(part.id.clone(), angle);
                    } else if let Some(angle) = self.servo_angles.get(&part.id).copied() {
                        if let Some(out) = self.outputs.get_mut(&part.id) {
                            out.angle = Some(angle);
                        }
                    }
                }
            }
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
        self.pin_pulses.clear();
        self.segment_memory.clear();
        self.seven_pending.clear();
        self.seven_stable.clear();
        self.servo_angles.clear();
        let eval =
            scene::evaluate_scene(&self.parts, &self.wires, &self.avr_pins, &self.pin_pulses);
        self.led_states = eval.leds;
        self.outputs = eval.outputs;
        self.rgb_leds = eval.rgb_leds;
        self.sensors = eval.sensors;
        self.apply_output_memory();
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
                    SimEvent::Timing {
                        port,
                        pin,
                        high_micros,
                        ..
                    } => {
                        if high_micros > 0 {
                            self.pin_pulses
                                .insert(format!("{}{}", port as char, pin), high_micros as f64);
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            let eval =
                scene::evaluate_scene(&self.parts, &self.wires, &self.avr_pins, &self.pin_pulses);
            self.led_states = eval.leds;
            self.outputs = eval.outputs;
            self.rgb_leds = eval.rgb_leds;
            self.sensors = eval.sensors;
            self.apply_output_memory();
        }
    }
}

impl eframe::App for Spike {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        self.poll_engine();
        if self.auto_run && !self.running {
            self.auto_run = false;
            self.start_sim(&ctx);
        }
        ensure_textures(&ctx, &mut self.textures, &self.parts, &self.outputs);

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
            if self.fit_requested {
                self.camera = fit_camera(ui.available_rect_before_wrap(), &self.parts);
                self.fit_requested = false;
            }
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
                    )),
                    self.rgb_leds.get(&part.id).copied(),
                    self.outputs.get(&part.id).map(|out| out.values.as_slice()),
                );
                if shows_active(part, &self.outputs, &self.sensors) {
                    draw_active_dot(&painter, rect, self.camera, part);
                }
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

fn fit_camera(rect: Rect, parts: &[Part]) -> Camera {
    let mut bounds = Rect::NOTHING;
    for part in parts {
        bounds = bounds.union(part.bounds());
    }
    if parts.is_empty() || !bounds.is_finite() {
        return Camera::default();
    }
    let margin = 32.0;
    let zoom = ((rect.width() - margin * 2.0) / bounds.width().max(1.0))
        .min((rect.height() - margin * 2.0) / bounds.height().max(1.0))
        .clamp(0.1, 1.5);
    Camera {
        zoom,
        pan: -bounds.center().to_vec2() * zoom,
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

fn segment_color(color: &str) -> Color32 {
    match color {
        "green" => Color32::from_rgb(0x9e, 0xff, 0x3c),
        "blue" => Color32::from_rgb(0x2c, 0x95, 0xfa),
        "cyan" => Color32::from_rgb(0x6c, 0xf9, 0xdc),
        "yellow" => Color32::from_rgb(0xf1, 0xd7, 0x3c),
        "white" => Color32::WHITE,
        _ => Color32::from_rgb(0xdc, 0x01, 0x2d),
    }
}

fn draw_segments(painter: &egui::Painter, screen: Rect, part: &Part, values: &[u8]) {
    const DIGITS: [[(f32, f32); 6]; 7] = [
        [
            (2.0, 0.0),
            (8.0, 0.0),
            (9.0, 1.0),
            (8.0, 2.0),
            (2.0, 2.0),
            (1.0, 1.0),
        ],
        [
            (10.0, 2.0),
            (10.0, 8.0),
            (9.0, 9.0),
            (8.0, 8.0),
            (8.0, 2.0),
            (9.0, 1.0),
        ],
        [
            (10.0, 10.0),
            (10.0, 16.0),
            (9.0, 17.0),
            (8.0, 16.0),
            (8.0, 10.0),
            (9.0, 9.0),
        ],
        [
            (8.0, 18.0),
            (2.0, 18.0),
            (1.0, 17.0),
            (2.0, 16.0),
            (8.0, 16.0),
            (9.0, 17.0),
        ],
        [
            (0.0, 16.0),
            (0.0, 10.0),
            (1.0, 9.0),
            (2.0, 10.0),
            (2.0, 16.0),
            (1.0, 17.0),
        ],
        [
            (0.0, 8.0),
            (0.0, 2.0),
            (1.0, 1.0),
            (2.0, 2.0),
            (2.0, 8.0),
            (1.0, 9.0),
        ],
        [
            (2.0, 8.0),
            (8.0, 8.0),
            (9.0, 9.0),
            (8.0, 10.0),
            (2.0, 10.0),
            (1.0, 9.0),
        ],
    ];
    let four = part.kind == "seven-segment-4";
    let view_width = if four { 50.2 } else { 12.55 };
    let scale_x = screen.width() / view_width;
    let scale_y = screen.height() / 22.0;
    let tan = (-8.0f32).to_radians().tan();
    let color = segment_color(part.attrs.get("color").map(String::as_str).unwrap_or("red"));
    let digits = if four { 4 } else { 1 };
    for digit in 0..digits {
        let base = digit * 8;
        let Some(bits) = values.get(base..base + 8) else {
            break;
        };
        let tx = 3.5 + digit as f32 * 12.7;
        let point = |x: f32, y: f32| {
            let sx = x * 0.81 + tx;
            let sy = y * 0.81 + 2.4;
            egui::pos2(
                screen.left() + (sx + sy * tan) * scale_x,
                screen.top() + sy * scale_y,
            )
        };
        for (index, shape) in DIGITS.iter().enumerate() {
            if bits.get(index).copied().unwrap_or(0) == 0 {
                continue;
            }
            let points = shape.iter().map(|(x, y)| point(*x, *y)).collect();
            painter.add(egui::Shape::convex_polygon(
                points,
                color,
                egui::Stroke::NONE,
            ));
        }
        if bits.get(7).copied().unwrap_or(0) != 0 {
            let center = egui::pos2(
                screen.left() + (tx + 7.4) * scale_x,
                screen.top() + 16.0 * scale_y,
            );
            painter.circle_filled(center, 0.89 * scale_x.min(scale_y), color);
        }
    }
}

/// One of the five shapes the RGB LED element paints for its state.
struct Glow {
    x: f32,
    y: f32,
    radius: f32,
    sigma: f32,
    colour: Color32,
    alpha: f32,
    halo: bool,
}

/// The element's five shapes for a state, in the order it paints them.
///
/// Straight out of `rgb-led-element.js`: a channel is a disc of `value * 5 + 2`
/// blurred by `value * 3` at `min(value * 20, 0.3)`; then a disc of the mixed
/// colour blurred by 4 at `0.2 + brightness * 0.6`; then a grey halo of the
/// same opacity.
fn rgb_glows(state: RgbState) -> [Glow; 5] {
    let r = state.r.clamp(0.0, 1.0) as f32;
    let g = state.g.clamp(0.0, 1.0) as f32;
    let b = state.b.clamp(0.0, 1.0) as f32;
    let brightness = r.max(g).max(b);
    let glow = if brightness > 0.0 {
        0.2 + brightness * 0.6
    } else {
        0.0
    };
    // The element's own quirk: blue also feeds the green channel by 90/255.
    let mixed = Color32::from_rgb(
        (r * 255.0).round() as u8,
        (g * 255.0 + b * 90.0).min(255.0).round() as u8,
        (b * 255.0).round() as u8,
    );
    let channel = |x: f32, y: f32, value: f32, colour: Color32| Glow {
        x,
        y,
        radius: value * 5.0 + 2.0,
        sigma: value * 3.0,
        colour,
        alpha: (value * 20.0).min(0.3),
        halo: false,
    };
    [
        channel(1.7, 3.0, r, Color32::from_rgb(255, 0, 0)),
        channel(2.7, 5.0, g, Color32::from_rgb(0, 255, 0)),
        channel(0.7, 5.0, b, Color32::from_rgb(1, 85, 253)),
        Glow {
            x: 1.7,
            y: 4.0,
            radius: 10.0,
            sigma: 4.0,
            colour: mixed,
            alpha: glow,
            halo: false,
        },
        Glow {
            x: 1.7,
            y: 4.0,
            radius: 13.0,
            sigma: 4.0,
            colour: Color32::from_rgb(0x66, 0x66, 0x66),
            alpha: glow,
            halo: true,
        },
    ]
}

/// The view box of the RGB LED asset, and where it starts.
const RGB_VIEW_WIDTH: f32 = 37.3425;
const RGB_VIEW_HEIGHT: f32 = 57.5115;
const RGB_VIEW_X: f32 = -17.0;
const RGB_VIEW_Y: f32 = -10.0;

/// Paint the element's glow over the base art.
///
/// This has to be drawn rather than rasterised: one resvg pass over the
/// element's four Gaussian filters costs about a third of a second here, and
/// the state changes as soon as PWM duty drifts.
fn draw_rgb_glow(painter: &egui::Painter, screen: Rect, state: RgbState) {
    // The asset is fitted into the part rect with `preserveAspectRatio="meet"`,
    // so the glow uses the same mapping to land on the drawn LED.
    let unit = (screen.width() / RGB_VIEW_WIDTH).min(screen.height() / RGB_VIEW_HEIGHT);
    let centre_of_view = egui::vec2(
        RGB_VIEW_X + RGB_VIEW_WIDTH / 2.0,
        RGB_VIEW_Y + RGB_VIEW_HEIGHT / 2.0,
    );
    let at = |x: f32, y: f32| screen.center() + (egui::vec2(x, y) - centre_of_view) * unit;
    for glow in rgb_glows(state) {
        if glow.alpha <= 0.0 {
            continue;
        }
        let bands = if glow.halo {
            halo_bands(glow.radius * unit, glow.sigma * unit, glow.colour, glow.alpha)
        } else {
            disc_bands(glow.radius * unit, glow.sigma * unit, glow.colour, glow.alpha)
        };
        soft_radial(painter, at(glow.x, glow.y), &bands);
    }
}

/// A disc through a Gaussian blur: solid inside, half strength at the radius,
/// gone about three sigmas past it. Without a blur it is a hard-edged disc.
fn disc_bands(radius: f32, sigma: f32, colour: Color32, alpha: f32) -> Vec<(f32, Color32)> {
    let mut bands = vec![(0.0, with_alpha(colour, alpha))];
    if sigma <= 0.01 || radius <= 0.0 {
        bands.push((radius.max(0.0), with_alpha(colour, alpha)));
        return bands;
    }
    let inner = (radius - sigma * 2.0).max(0.0);
    let outer = radius + sigma * 3.0;
    if inner > 0.0 {
        bands.push((inner, with_alpha(colour, alpha)));
    }
    const STEPS: usize = 6;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let distance = inner + (outer - inner) * t;
        bands.push((distance, with_alpha(colour, alpha * (1.0 - smoothstep(t)))));
    }
    bands
}

/// A one-unit stroke through a Gaussian blur: a faint ring, not a disc.
fn halo_bands(radius: f32, sigma: f32, colour: Color32, alpha: f32) -> Vec<(f32, Color32)> {
    if sigma <= 0.01 {
        return vec![(radius, with_alpha(colour, alpha))];
    }
    // Peak alpha of a stroke spread over a Gaussian of this width.
    let peak = (1.0 / (sigma * (std::f32::consts::TAU).sqrt())).min(1.0) * alpha;
    let inner = (radius - sigma * 3.0).max(0.0);
    let outer = radius + sigma * 3.0;
    let mut bands = vec![(inner, with_alpha(colour, 0.0))];
    const STEPS: usize = 8;
    for step in 1..=STEPS {
        let t = step as f32 / STEPS as f32;
        let distance = inner + (outer - inner) * t;
        let offset = (distance - radius) / sigma;
        bands.push((
            distance,
            with_alpha(colour, peak * (-0.5 * offset * offset).exp()),
        ));
    }
    bands
}

/// Fill a radial shape from the inside out, interpolating between bands.
fn soft_radial(painter: &egui::Painter, centre: Pos2, bands: &[(f32, Color32)]) {
    const SEGMENTS: usize = 48;
    let Some((first_radius, first_colour)) = bands.first().copied() else {
        return;
    };
    if first_radius > 0.0 {
        painter.circle_filled(centre, first_radius, first_colour);
    }
    for pair in bands.windows(2) {
        let (inner, inner_colour) = pair[0];
        let (outer, outer_colour) = pair[1];
        if outer <= inner {
            continue;
        }
        let mut mesh = egui::Mesh::default();
        for index in 0..SEGMENTS {
            let angle = index as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            let direction = egui::vec2(angle.cos(), angle.sin());
            mesh.colored_vertex(centre + direction * inner, inner_colour);
            mesh.colored_vertex(centre + direction * outer, outer_colour);
        }
        for index in 0..SEGMENTS {
            let next = (index + 1) % SEGMENTS;
            let (a, b) = ((index * 2) as u32, (index * 2 + 1) as u32);
            let (c, d) = ((next * 2) as u32, (next * 2 + 1) as u32);
            mesh.add_triangle(a, b, d);
            mesh.add_triangle(a, d, c);
        }
        painter.add(egui::Shape::mesh(mesh));
    }
}

fn with_alpha(colour: Color32, alpha: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(
        colour.r(),
        colour.g(),
        colour.b(),
        (alpha.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Which parts the oracle marks as doing something: an energised relay, a
/// tilted tilt switch, a PIR that sees motion.
fn shows_active(
    part: &Part,
    outputs: &HashMap<String, OutputState>,
    sensors: &HashMap<String, SensorState>,
) -> bool {
    match part.kind.as_str() {
        "relay" => outputs.get(&part.id).is_some_and(|out| out.active),
        "tilt-switch" | "pir-motion-sensor" => {
            sensors.get(&part.id).is_some_and(|state| state.active)
        }
        _ => false,
    }
}

/// The oracle's `drawActive`: a five-unit dot just inside the top-left corner.
fn draw_active_dot(painter: &egui::Painter, rect: Rect, camera: Camera, part: &Part) {
    let centre = camera.world_to_screen(rect, part.bounds().min + Vec2::new(4.0, 4.0));
    let radius = 5.0 * camera.zoom;
    painter.circle_filled(centre, radius, Color32::from_rgb(108, 203, 123));
    painter.circle_stroke(
        centre,
        radius,
        Stroke::new(1.0, Color32::from_rgb(26, 26, 28)),
    );
}

fn draw_part(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera,
    part: &Part,
    selected: bool,
    hovered_pin: Option<PinRef>,
    texture: Option<&egui::TextureHandle>,
    rgb: Option<RgbState>,
    segments: Option<&[u8]>,
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
    if let Some(state) = rgb {
        draw_rgb_glow(painter, screen, state);
    }
    if let Some(values) = segments {
        draw_segments(painter, screen, part, values);
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
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wokwi/led-bar-graph.svg");
        let svg = std::fs::read_to_string(path).expect("led bar asset");
        let values = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let patched = scene::led_bar_svg(&svg, &values, "red");
        assert_eq!(patched.matches("#dc012d").count(), 1);
        assert_eq!(patched.matches("#444").count(), 9);
    }

    #[test]
    fn seven_segment_patches_lit_segments() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wokwi/seven-segment.svg");
        let svg = std::fs::read_to_string(path).expect("7-segment asset");
        let patched = scene::seven_segment_svg(&svg, &[1, 0, 0, 0, 0, 0, 0, 0], "red");
        assert_eq!(patched.matches("#dc012d").count(), 1);
        assert_eq!(patched.matches("#444").count(), 7);
    }

    #[test]
    fn rgb_glow_numbers_come_from_the_element() {
        let glows = rgb_glows(RgbState {
            r: 1.0,
            g: 0.0,
            b: 0.0,
        });
        assert_eq!(glows[0].radius, 7.0, "a full channel is a radius 7 disc");
        assert_eq!(glows[0].sigma, 3.0, "blurred by 3");
        assert!((glows[0].alpha - 0.3).abs() < 1e-6, "at min(value * 20, 0.3)");
        assert_eq!(glows[1].alpha, 0.0, "green is off");
        assert_eq!(glows[2].alpha, 0.0, "blue is off");
        assert_eq!(
            glows[3].colour,
            Color32::from_rgb(255, 0, 0),
            "the glow takes the mixed colour"
        );
        assert!(
            (glows[3].alpha - 0.8).abs() < 1e-6,
            "and 0.2 + brightness * 0.6"
        );
        assert!(glows[4].halo, "the last shape is the grey halo");
        assert_eq!(glows[4].colour, Color32::from_rgb(0x66, 0x66, 0x66));
        assert!((glows[4].alpha - glows[3].alpha).abs() < 1e-6);
    }

    #[test]
    fn rgb_glow_keeps_the_elements_blue_in_green_quirk() {
        let blue = rgb_glows(RgbState {
            r: 0.0,
            g: 0.0,
            b: 1.0,
        });
        assert_eq!(blue[3].colour, Color32::from_rgb(0, 90, 255));
        let white = rgb_glows(RgbState {
            r: 1.0,
            g: 1.0,
            b: 1.0,
        });
        assert_eq!(
            white[3].colour,
            Color32::from_rgb(255, 255, 255),
            "255 + 90 clamps back to 255"
        );
        let dark = rgb_glows(RgbState {
            r: 0.0,
            g: 0.0,
            b: 0.0,
        });
        assert_eq!(dark[3].alpha, 0.0, "an unlit LED does not glow at all");
    }

    #[test]
    fn a_blurred_disc_fades_outward() {
        let bands = disc_bands(10.0, 4.0, Color32::WHITE, 1.0);
        assert_eq!(bands[0].0, 0.0, "starts at the centre");
        assert!(
            bands.windows(2).all(|pair| pair[1].0 >= pair[0].0),
            "radii only grow"
        );
        assert!(
            bands.windows(2).all(|pair| pair[1].1.a() <= pair[0].1.a()),
            "alpha never rises outward"
        );
        let outer = bands.last().expect("bands").0;
        assert!(
            outer > 10.0 + 2.0 * 4.0,
            "the blur reaches well past the edge, got {outer}"
        );
        let hard = disc_bands(10.0, 0.0, Color32::WHITE, 1.0);
        assert_eq!(hard.len(), 2, "no blur means one hard edge");
        assert_eq!(hard[1].0, 10.0);
    }

    #[test]
    fn the_oracle_marks_only_relays_and_on_off_sensors() {
        use croquis_core::catalogue::PartKind;
        let relay =
            scene::part_from_kind(PartKind::Relay, Pos2::ZERO, "relay".into()).expect("relay part");
        let tilt = scene::part_from_kind(PartKind::TiltSwitch, Pos2::ZERO, "tilt".into())
            .expect("tilt part");
        let pir = scene::part_from_kind(PartKind::PirMotionSensor, Pos2::ZERO, "pir".into())
            .expect("pir part");
        let led = scene::part_from_kind(PartKind::Led, Pos2::ZERO, "led".into()).expect("led part");

        let mut outputs = HashMap::new();
        let mut sensors = HashMap::new();
        assert!(
            !shows_active(&relay, &outputs, &sensors),
            "a relay at rest draws nothing"
        );
        outputs.insert(
            "relay".into(),
            OutputState {
                active: true,
                ..Default::default()
            },
        );
        assert!(
            shows_active(&relay, &outputs, &sensors),
            "an energised relay is marked"
        );
        assert!(
            !shows_active(&tilt, &outputs, &sensors),
            "an upright tilt switch is not"
        );
        for id in ["tilt", "pir"] {
            sensors.insert(
                id.into(),
                SensorState {
                    powered: true,
                    active: true,
                },
            );
        }
        assert!(shows_active(&tilt, &outputs, &sensors));
        assert!(shows_active(&pir, &outputs, &sensors));
        assert!(
            !shows_active(&led, &outputs, &sensors),
            "no other part gets a dot"
        );
    }

    #[test]
    fn multiplexed_display_memory_keeps_all_digits() {        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/four-digit.json");
        let (parts, _, _) = scene::load_diagram_file(&path);
        let display = parts
            .iter()
            .find(|part| part.kind == "seven-segment-4")
            .expect("four-digit part")
            .id
            .clone();
        let mut spike = Spike::default();
        spike.parts = parts;
        spike.outputs.insert(
            display.clone(),
            OutputState {
                active: true,
                digit: Some(0),
                values: vec![1, 0, 0, 0, 0, 0, 0, 0],
                ..Default::default()
            },
        );
        spike.apply_output_memory();
        spike.outputs.insert(
            display.clone(),
            OutputState {
                active: true,
                digit: Some(1),
                values: vec![0, 1, 0, 0, 0, 0, 0, 0],
                ..Default::default()
            },
        );
        spike.apply_output_memory();
        let values = &spike.outputs[&display].values;
        assert_eq!(values.len(), 32);
        assert_eq!(values[0], 1, "values={values:?}");
        assert_eq!(values[9], 1, "values={values:?}");
    }

    #[test]
    fn a_servo_holds_its_last_angle_without_a_pulse() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/servo.json");
        let (parts, _, _) = scene::load_diagram_file(&path);
        let servo = parts
            .iter()
            .find(|part| part.kind == "servo")
            .expect("servo part")
            .id
            .clone();
        let mut spike = Spike::default();
        spike.parts = parts;
        spike.outputs.insert(
            servo.clone(),
            OutputState {
                active: true,
                values: vec![],
                angle: Some(90.0),
                ..Default::default()
            },
        );
        spike.apply_output_memory();
        spike.outputs.insert(
            servo.clone(),
            OutputState {
                active: false,
                values: vec![],
                angle: None,
                ..Default::default()
            },
        );
        spike.apply_output_memory();
        assert_eq!(spike.outputs[&servo].angle, Some(90.0));
    }

    #[test]
    fn the_servo_preset_turns_from_a_pin_pulse() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/servo.json");
        let (parts, wires, _) = scene::load_diagram_file(&path);
        let servo = parts
            .iter()
            .find(|part| part.kind == "servo")
            .expect("servo part")
            .id
            .clone();
        let avr = HashMap::from([("B1".to_string(), true)]);
        let pulses = HashMap::from([("B1".to_string(), 1472.0)]);
        let eval = scene::evaluate_scene(&parts, &wires, &avr, &pulses);
        let angle = eval.outputs[&servo].angle.expect("servo angle");
        assert!((85.0..=95.0).contains(&angle), "angle={angle}");
    }

    #[test]
    fn all_four_digits_survive_the_real_preset_pins() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/four-digit.json");
        let (parts, wires, _) = scene::load_diagram_file(&path);
        let display = parts
            .iter()
            .find(|part| part.kind == "seven-segment-4")
            .expect("four-digit part")
            .id
            .clone();
        let mut spike = Spike::default();
        spike.parts = parts;
        spike.wires = wires;
        for digit in 0..4u8 {
            let mut avr = HashMap::new();
            // D2-D7 are port D; D8 is PB0. D9-D12 are PB1-PB4.
            for pin in 2..=7 {
                avr.insert(format!("D{pin}"), true);
            }
            avr.insert("B0".to_string(), true);
            for common in 1..=4 {
                avr.insert(format!("B{common}"), true);
            }
            avr.insert(format!("B{}", 1 + digit), false);
            let eval = scene::evaluate_scene(&spike.parts, &spike.wires, &avr, &HashMap::new());
            spike.outputs = eval.outputs;
            spike.apply_output_memory();
        }
        let values = &spike.outputs[&display].values;
        assert_eq!(values.len(), 32);
        for digit in 0..4 {
            assert!(
                values[digit * 8..digit * 8 + 7]
                    .iter()
                    .all(|value| *value == 1),
                "digit {digit} missing: {values:?}"
            );
        }
    }
}
