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
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use croquis_core::catalogue::{parse_kind, parts, PartKind};
use croquis_core::netlist::{
    AnalogReading, CircuitEval, Drive, IrReceiver, KeyPosition, LcdWiring, LedState, OutputState,
    RgbState, SensorState, StepperWiring,
};
use croquis_core::route::{expand_path, leg_bend, Point as CorePoint};
use croquis_core::shift_register::{step_shift_register, ShiftState, ShiftWiring};
use croquis_sim::engine::{self, SimEvent};
use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::{self, Align2, Color32, FontId, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};

mod art;
mod buzzer;
mod editor;
mod generated;
mod lcd;
mod lcd_font;
mod knobs;
mod locales;
mod properties;
mod remote;
mod sensors;
mod stepper;
mod scene;

use locales::Texts;
use scene::{Part, PinRef, Wire};

const GRID: f32 = 24.0;
const PIN_HIT_RADIUS: f32 = 9.0;
/// How close a click has to be to a wire, in screen pixels, to select it. A
/// wire is two and a half pixels wide, so this is the tolerance a hand needs.
const WIRE_HIT_RADIUS: f32 = 6.0;
/// How close a dragged pin has to come to a still pin, in screen pixels, before
/// it locks onto it. A little larger than the hit radius: the pull should start
/// just before the hand has to be exact.
const SNAP_RADIUS: f32 = 10.0;
/// The height a docked panel's head occupies. A collapsed serial monitor keeps
/// exactly this much of itself, so it can be unfolded again.
const SERIAL_HEAD_HEIGHT: f32 = 32.0;
/// How tall the serial monitor's log is when it is unfolded.
const SERIAL_LOG_HEIGHT: f32 = 150.0;

/// The prefix every palette thumbnail is keyed with. A part's canvas art is
/// keyed by anything else, so this is what tells them apart.
const PALETTE_PREFIX: &str = "palette:";

/// Drop the canvas art and keep the palette's.
///
/// A new document has no parts, so their textures are dead — but the palette's
/// thumbnails are the catalogue's, not the document's. Clearing those too made the
/// pictures vanish for a frame and then rasterised thirty-one SVGs to get them
/// back, which is what made File → New feel like it had hung.
fn keep_palette<T>(textures: &mut HashMap<String, T>) {
    textures.retain(|key, _| key.starts_with(PALETTE_PREFIX));
}

/// What a new sketch starts as.
///
/// This is code a reader copies, not interface text: the comments stay in English
/// so the template goes on matching every Arduino tutorial on the internet.
const SKETCH_TEMPLATE: &str = "\
void setup() {
  // write your setup code here, to run once
}

void loop() {
  // write your main code here, to run repeatedly
}
";

/// How much of the circuit panel the serial monitor takes: nothing while it is
/// off, its head while it is folded, its log while it is open.
///
/// The folded case is never zero — a panel that takes no space is a panel with no
/// chevron left to unfold it, which is what made collapsing it feel like losing it.
fn serial_height(open: bool, collapsed: bool) -> f32 {
    if !open {
        0.0
    } else if collapsed {
        SERIAL_HEAD_HEIGHT
    } else {
        SERIAL_LOG_HEIGHT
    }
}

/// The shell's own chrome, from v0.2's tokens (`--surface`, `--border`,
/// `--text-muted`, `--accent`).
const SURFACE: Color32 = Color32::from_rgb(0x21, 0x21, 0x24);
const BORDER: Color32 = Color32::from_rgb(0x35, 0x35, 0x3a);
const MUTED: Color32 = Color32::from_rgb(0xa6, 0xa4, 0xa0);
const ERROR_TEXT: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
const WARNING_TEXT: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);

/// The sketch editor's id. A diagnostic jump writes the caret through
/// `TextEditState`, and that needs the widget to keep one id across frames.
fn sketch_editor_id() -> egui::Id {
    egui::Id::unique("croquis-sketch-editor")
}

/// Put the caret where a diagnostic points. `false` means the editor has not
/// written its state yet (it has not been shown), so there is nothing to move.
fn place_caret(ctx: &egui::Context, sketch: &str, line: usize, column: usize) -> bool {
    let index = editor::line_col_to_char_index(sketch, line, column);
    let Some(mut state) = egui::text_edit::TextEditState::load(ctx, sketch_editor_id()) else {
        return false;
    };
    state
        .cursor
        .set_char_range(Some(CCursorRange::one(CCursor::new(index))));
    state.store(ctx, sketch_editor_id());
    true
}

/// The palette's thumbnails: the part itself, small, rasterised once.
///
/// v0.2's palette is a list of pictures, not labels; the art is the same art the
/// canvas uses, so this is the same drawing at a smaller size.
/// The raster height palette pictures are baked at. Tiles scale with the panel,
/// so the art is baked larger than any tile and drawn down: a fixed-size bake
/// would go soft the first time the panel was widened.
const PALETTE_ART_HEIGHT: u32 = 128;

fn ensure_palette_textures(
    ctx: &egui::Context,
    textures: &mut HashMap<String, egui::TextureHandle>,
) {
    for def in parts() {
        let key = format!("palette:{}", def.kind.as_str());
        if textures.contains_key(&key) {
            continue;
        }
        let Some(part) = scene::part_from_kind(def.kind, Pos2::ZERO, key.clone()) else {
            continue;
        };
        // The picture's own aspect, not a clamped box: rasterising a resistor
        // into 96×48 squashed it to half its height and it read as a fat bar.
        let aspect = part.size.x / part.size.y.max(1.0);
        let height = PALETTE_ART_HEIGHT;
        let width = ((height as f32 * aspect).round() as u32).clamp(8, 1024);
        let Some(image) = art::rasterize(&scene::resting_svg(&part), width, height) else {
            continue;
        };
        let texture = ctx.load_texture(key.clone(), image, egui::TextureOptions::LINEAR);
        textures.insert(key, texture);
    }
}

/// One palette tile: the part's picture above its name, as v0.2's `.part`.
///
/// The tile is as wide as its column, so resizing the panel makes the boxes and
/// the pictures grow rather than just fitting more of them.
fn palette_tile(
    ui: &mut egui::Ui,
    textures: &HashMap<String, egui::TextureHandle>,
    key: &str,
    label: &str,
    enabled: bool,
    tile: Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(tile, egui::Sense::click_and_drag());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let raised = Color32::from_rgb(0x26, 0x26, 0x2a);
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, raised);
    painter.rect_stroke(
        rect,
        0.0,
        egui::Stroke::new(
            1.0,
            if response.hovered() && enabled {
                Color32::from_rgb(0xd9, 0xa4, 0x41)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );

    // Two bands that cannot meet: the picture above, the name below. Scaling to
    // the tile's height instead let a wide part like the resistor grow into the
    // label, which is what put text on top of the artwork.
    const LABEL_BAND: f32 = 30.0;
    const PAD: f32 = 8.0;
    let picture = (tile.y - LABEL_BAND - 5.0).max(8.0);
    if let Some(texture) = textures.get(key) {
        let size = texture.size_vec2();
        let band = egui::Rect::from_min_max(
            egui::pos2(rect.left() + PAD, rect.top() + 5.0),
            egui::pos2(rect.right() - PAD, rect.top() + 5.0 + picture),
        );
        let scale = (band.width() / size.x.max(1.0)).min(band.height() / size.y.max(1.0));
        let drawn = size * scale;
        let at = egui::Rect::from_center_size(band.center(), drawn);
        painter.image(
            texture.id(),
            at,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            if enabled {
                Color32::WHITE
            } else {
                Color32::from_rgba_unmultiplied(255, 255, 255, 110)
            },
        );
    }

    // The name, under it, centred and wrapped.
    let label_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 4.0, rect.bottom() - LABEL_BAND),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
    );
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(label_rect)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    child.add(
        egui::Label::new(
            egui::RichText::new(label)
                .size(11.0)
                .color(if enabled { MUTED } else { Color32::from_rgb(0x64, 0x64, 0x68) }),
        )
        .selectable(false)
        .wrap(),
    );
    response
}

/// The icons the shell needs, drawn rather than typed.
///
/// Plex's Latin subset has no arrows or box-drawing glyphs, and neither does
/// egui's fallback, so glyph buttons came out as empty squares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Serial,
    Fit,
    Chevron(Chevron),
}

/// The way a chevron points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chevron {
    Left,
    Right,
    Up,
    Down,
}

/// The edge a panel is docked to, which is also the way it folds.
///
/// A collapse control has to agree with what it collapses: a panel docked at the
/// left folds leftward, and a chevron pointing right is then pointing at the
/// wrong half of the screen. The serial monitor is docked along the bottom, so
/// its chevron points down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dock {
    Left,
    Right,
    Bottom,
}

impl Dock {
    /// The way the chevron points while the panel is open: toward the edge it
    /// will fold into.
    fn closing(self) -> Chevron {
        match self {
            Self::Left => Chevron::Left,
            Self::Right => Chevron::Right,
            Self::Bottom => Chevron::Down,
        }
    }

    /// And the way back out, which is the opposite.
    fn opening(self) -> Chevron {
        match self {
            Self::Left => Chevron::Right,
            Self::Right => Chevron::Left,
            Self::Bottom => Chevron::Up,
        }
    }
}

/// A chevron as two points and an apex, measured from the button's centre, with
/// the apex pointing `direction`.
fn chevron_wedge(direction: Chevron) -> ((f32, f32), [(f32, f32); 2]) {
    match direction {
        Chevron::Right => ((4.0, 0.0), [(-4.0, -5.0), (-4.0, 5.0)]),
        Chevron::Left => ((-4.0, 0.0), [(4.0, -5.0), (4.0, 5.0)]),
        Chevron::Down => ((0.0, 4.0), [(-5.0, -4.0), (5.0, -4.0)]),
        Chevron::Up => ((0.0, -4.0), [(-5.0, 4.0), (5.0, 4.0)]),
    }
}

fn icon_button(ui: &mut egui::Ui, icon: Icon, hover: &str) -> bool {
    let size = Vec2::splat(20.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if response.hovered() {
            painter.rect_filled(rect, 0.0, Color32::from_rgb(0x2a, 0x2a, 0x2e));
        }
        let colour = if response.hovered() { Color32::WHITE } else { MUTED };
        let stroke = egui::Stroke::new(1.3, colour);
        let c = rect.center();
        match icon {
            Icon::Serial => {
                for offset in [-4.0, 0.0, 4.0] {
                    painter.hline(
                        (c.x - 6.0)..=(c.x + 6.0),
                        c.y + offset,
                        stroke,
                    );
                }
            }
            Icon::Fit => {
                painter.rect_stroke(
                    egui::Rect::from_center_size(c, Vec2::splat(12.0)),
                    0.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                painter.rect_filled(
                    egui::Rect::from_center_size(c, Vec2::splat(4.0)),
                    0.0,
                    colour,
                );
            }
            Icon::Chevron(direction) => {
                let (apex, wings) = chevron_wedge(direction);
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        egui::pos2(c.x + apex.0, c.y + apex.1),
                        egui::pos2(c.x + wings[0].0, c.y + wings[0].1),
                        egui::pos2(c.x + wings[1].0, c.y + wings[1].1),
                    ],
                    colour,
                    egui::Stroke::NONE,
                ));
            }
        }
    }
    response.on_hover_text(hover).clicked()
}

/// The chevron that closes a panel, as v0.2's `.icon-btn` in a head.
fn panel_toggle(ui: &mut egui::Ui, open: &mut bool, texts: &Texts, dock: Dock) -> bool {
    let (icon, hover) = if *open {
        (Icon::Chevron(dock.closing()), texts.t("panel.collapse"))
    } else {
        (Icon::Chevron(dock.opening()), texts.t("panel.expand"))
    };
    if icon_button(ui, icon, hover) {
        *open = !*open;
        return true;
    }
    false
}

/// A panel's head, as v0.2's `.panel__head`: 32px tall, a muted uppercase title,
/// a hairline under it, and room for controls on the right.
fn panel_head(ui: &mut egui::Ui, title: &str, controls: impl FnOnce(&mut egui::Ui)) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 32.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, SURFACE);
    painter.hline(
        rect.x_range(),
        rect.max.y - 0.5,
        egui::Stroke::new(1.0, BORDER),
    );
    // The rule is drawn along the bottom edge, so the controls sit in a band
    // that stops short of it. Centred in the full height, a filled button like
    // Run came within a couple of pixels of the rule and read as part of it.
    const RULE_CLEARANCE: f32 = 5.0;
    let band = egui::Rect::from_min_max(
        rect.min,
        egui::pos2(rect.max.x, rect.max.y - RULE_CLEARANCE),
    );
    let mut head = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(band.shrink2(Vec2::new(12.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    head.label(
        egui::RichText::new(title.to_uppercase())
            .size(12.0)
            .color(MUTED),
    );
    head.with_layout(egui::Layout::right_to_left(egui::Align::Center), controls);
}

/// Install the bundled IBM Plex faces: Sans for the interface, Mono for code.
///
/// The Latin subset has no arrows or chevrons, so egui's own faces stay behind
/// these as fallbacks rather than being replaced.
fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "plex-sans",
            include_bytes!("../../../assets/fonts/IBMPlexSans-Regular.ttf") as &[u8],
        ),
        (
            "plex-sans-medium",
            include_bytes!("../../../assets/fonts/IBMPlexSans-Medium.ttf"),
        ),
        (
            "plex-sans-semibold",
            include_bytes!("../../../assets/fonts/IBMPlexSans-SemiBold.ttf"),
        ),
        (
            "plex-mono",
            include_bytes!("../../../assets/fonts/IBMPlexMono-Regular.ttf"),
        ),
        (
            "plex-mono-medium",
            include_bytes!("../../../assets/fonts/IBMPlexMono-Medium.ttf"),
        ),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
    }

    for (family, faces) in [
        // Regular first: a family is a fallback order per glyph, not a weight
        // scale. Putting the heavier faces first set the whole interface in
        // SemiBold, which is not what the old shell looks like.
        (
            egui::FontFamily::Proportional,
            &["plex-sans"][..],
        ),
        (egui::FontFamily::Monospace, &["plex-mono"][..]),
    ] {
        let list = fonts.families.entry(family).or_default();
        for face in faces.into_iter().rev() {
            list.insert(0, (*face).to_owned());
        }
    }
    fonts.families.insert(
        egui::FontFamily::Name("semibold".into()),
        vec!["plex-sans-semibold".to_owned(), "plex-sans".to_owned()],
    );
    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    install_fonts(ctx);
    use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

    let bg = Color32::from_rgb(0x1a, 0x1a, 0x1c);
    let surface = Color32::from_rgb(0x21, 0x21, 0x24);
    let raised = Color32::from_rgb(0x2a, 0x2a, 0x2e);
    let border = Color32::from_rgb(0x35, 0x35, 0x3a);
    let text = Color32::from_rgb(0xe6, 0xe4, 0xe0);
    let muted = Color32::from_rgb(0xa6, 0xa4, 0xa0);
    let accent = Color32::from_rgb(0xd9, 0xa4, 0x41);

    let mut visuals = egui::Visuals::dark();
    // v0.2 fills panels with `--surface` and only the canvas with `--bg`, which
    // is what separates the docked chrome from the circuit.
    visuals.panel_fill = surface;
    visuals.window_fill = surface;
    // Text edits and scroll wells sit on the panel, not in a darker hole: v0.2's
    // editor and lists are `--surface` with a 1px border.
    visuals.extreme_bg_color = surface;
    visuals.faint_bg_color = surface;
    visuals.window_stroke = Stroke::new(1.0, border);
    // v0.2's shape rule: "no rounded corners on everything" — panels and menus
    // are square, controls carry the 2px token.
    visuals.window_corner_radius = CornerRadius::ZERO;
    visuals.menu_corner_radius = CornerRadius::ZERO;
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
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
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
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn main() -> eframe::Result {
    // The window title is set before the app exists, so it reads the catalogue
    // itself — in the language the last run saved, which is what the app will
    // start in a moment later.
    let texts = locales::Texts::load(locales::saved_locale());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 500.0])
            .with_title(texts.t("app.name")),
        ..Default::default()
    };
    eframe::run_native(
        "croquis-egui-spike",
        options,
        Box::new(|cc| {
            configure_style(&cc.egui_ctx);
            Ok(Box::new(Spike::starting_in(locales::saved_locale())))
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

/// What is selected: indices into `parts` and `wires`.
///
/// v0.2 lets a part and a wire be picked together, so this holds two lists
/// rather than one choice. The properties strip edits the single selected part.
#[derive(Default, Clone)]
struct Selection {
    parts: Vec<usize>,
    wires: Vec<usize>,
}

impl Selection {
    fn clear(&mut self) {
        self.parts.clear();
        self.wires.clear();
    }

    fn is_empty(&self) -> bool {
        self.parts.is_empty() && self.wires.is_empty()
    }

    fn has_part(&self, index: usize) -> bool {
        self.parts.contains(&index)
    }

    fn has_wire(&self, index: usize) -> bool {
        self.wires.contains(&index)
    }

    /// The part whose fields the properties strip shows: exactly one, or none.
    fn only_part(&self) -> Option<usize> {
        match self.parts.as_slice() {
            [index] if self.wires.is_empty() => Some(*index),
            _ => None,
        }
    }

    fn select_part(&mut self, index: usize) {
        self.clear();
        self.parts.push(index);
    }

    fn select_wire(&mut self, index: usize) {
        self.clear();
        self.wires.push(index);
    }

    fn toggle_part(&mut self, index: usize) {
        match self.parts.iter().position(|part| *part == index) {
            Some(at) => {
                self.parts.remove(at);
            }
            None => self.parts.push(index),
        }
    }

    fn toggle_wire(&mut self, index: usize) {
        match self.wires.iter().position(|wire| *wire == index) {
            Some(at) => {
                self.wires.remove(at);
            }
            None => self.wires.push(index),
        }
    }
}

/// Parts and the wires between them, as they were copied. A wire is kept as a
/// pair of indices into `parts`, so a paste connects the pasted copies to each
/// other rather than back to the originals.
#[derive(Clone, Default)]
struct Clipboard {
    parts: Vec<Part>,
    /// A wire as the indices of the copied parts it joins, the pins at each end,
    /// and its bends — so a paste is wired to its own copies.
    wires: Vec<(PinRef, PinRef, Vec<Pos2>)>,
}

struct Spike {
    camera: Camera,
    parts: Vec<Part>,
    wires: Vec<Wire>,
    wire_start: Option<PinRef>,
    /// Bends pinned while routing: click empty space (or a part's body) to hold
    /// the wire's last corner, so the rest can be routed around things. v0.2
    /// does the same, and the model already carries `waypoints` per wire.
    wire_waypoints: Vec<Pos2>,
    selection: Selection,
    /// The rubber band a right-drag is drawing, in screen coordinates.
    marquee: Option<(Pos2, Pos2)>,
    clipboard: Clipboard,
    dragging: Option<usize>,
    /// Where each dragged part was when the drag began, and how far the pointer
    /// has travelled since.
    ///
    /// Positions are rebuilt from these every frame rather than accumulated.
    /// A snap that was added straight into `center` was baked in: the next frame
    /// found the pin already on its target and pulled it back again, so the part
    /// had to be ripped off the hole instead of simply dragged away.
    drag_origin: Vec<(usize, Pos2)>,
    drag_travel: Vec2,
    hovered_pin: Option<PinRef>,
    textures: HashMap<String, egui::TextureHandle>,
    led_states: HashMap<String, LedState>,
    /// How hard each LED is being driven, 0 to 1, so a PWM fade is visible.
    led_brightness: HashMap<String, f64>,
    outputs: HashMap<String, OutputState>,
    rgb_leds: HashMap<String, RgbState>,
    sensors: HashMap<String, SensorState>,
    /// Each 74HC595's clocked and latched bits, and how its inputs are wired.
    /// A `shiftOut` burst is several clock edges in a few microseconds, so the
    /// register is stepped per edge rather than once per evaluation.
    shift_states: HashMap<String, ShiftState>,
    shift_wiring: HashMap<String, ShiftWiring>,
    /// The sensors that must be answered within a millisecond of the sketch's
    /// edge — an ultrasonic's echo and a DHT's frame. It is shared with the
    /// simulator's event thread, which is where the answering happens.
    responder: Arc<Mutex<sensors::Responder>>,
    /// Where pin changes go for the responder thread. Dropping it ends the
    /// thread, which is what happens when the simulator stops.
    pin_responder: Option<mpsc::Sender<(String, u8, u8, bool)>>,
    /// Which parts the pointer is holding down, by part id.
    pressed: HashMap<String, bool>,
    /// The key held on each keypad, and when its minimum hold ends.
    keypad_keys: HashMap<String, Option<KeyPosition>>,
    keypad_until: HashMap<String, Instant>,
    /// Frames left in a keypad tap. A tap has to survive the frame that records
    /// it *and* the solve on the next one; on a slow frame 80 ms is not enough,
    /// so a tap also holds for a few frames.
    keypad_frames: HashMap<String, u32>,
    /// A press on a button or a keypad, waiting to become a drag. A click has a
    /// pixel or two of hand jitter, and moving the part by that much walks it
    /// under the cursor, so a press there only becomes a drag once the pointer
    /// has actually travelled.
    press_drag: Option<(usize, Pos2)>,
    /// The joystick being dragged, whether it has moved, and where the press was.
    joystick: Option<usize>,
    joystick_moved: bool,
    joystick_start: Option<Pos2>,
    /// The potentiometer or slide pot whose knob is being dragged, and how far
    /// the pointer sat from the slide pot's handle when it was grabbed.
    knob: Option<usize>,
    slide_grab: f32,
    /// The levels and voltages last pushed into the simulator, so an unchanged
    /// drive is not re-sent every frame.
    last_drives: HashMap<(char, u8), bool>,
    last_analogs: HashMap<u8, u16>,
    /// The LCDs' wiring, and what their controllers have been told so far.
    lcd_wiring: HashMap<String, LcdWiring>,
    lcd_states: HashMap<String, lcd::LcdState>,
    /// Set when a property was edited, so the circuit is solved again even
    /// though no pin changed.
    reevaluate: bool,
    /// Where the IR receivers listen, and which remote button is being held.
    ir_receivers: HashMap<String, IrReceiver>,
    remote_hold: Option<(usize, &'static str)>,
    /// The steppers' coil wiring, the phase each is in, and how far it has turned.
    stepper_wiring: HashMap<String, StepperWiring>,
    stepper_phases: HashMap<String, usize>,
    stepper_angles: HashMap<String, f64>,
    buzzer: buzzer::BuzzerAudio,
    pin_pulses: HashMap<String, f64>,
    /// The duty cycle measured on each PWM pin, 0 to 1.
    pin_duty: HashMap<String, f64>,
    /// The frequency measured on each pin, in hertz — a buzzer's pitch.
    pin_frequency: HashMap<String, f64>,
    /// The board's own "L" LED, which follows pin 13.
    pin13: bool,
    /// Which docked panels are open. A closed panel leaves a rail behind, which
    /// is how v0.2 keeps the editor's history while clipping it to nothing.
    sketch_open: bool,
    components_open: bool,
    serial_open: bool,
    /// Folded down to its head, still docked. The monitor used to switch itself
    /// off when collapsed, which took away the very control that brought it back.
    serial_collapsed: bool,
    /// The catalogue entry being dragged out of the palette, if any. v0.2 places
    /// a part by dragging it onto the canvas, not by clicking a list.
    palette_drag: Option<croquis_core::catalogue::PartKind>,

    /// The interface's text, in the language the reader chose. Everything the
    /// catalogue and the shell display goes through this.
    texts: locales::Texts,

    /// What the netlist has to say about a part — an unpowered chip, an
    /// untied MR pin. v0.2 shows these in the properties strip.
    part_warnings: HashMap<String, String>,

    /// Which side a panel is docked on: "movable" in the old shell's sense —
    /// same panel, same size, docked elsewhere.
    segment_memory: HashMap<String, Vec<u8>>,
    /// When each single 7-segment display last changed, for the settle guard.
    seven_changed_at: HashMap<String, Instant>,
    /// The pattern each single display is showing.
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
    /// What the compiler said, one entry per problem. The raw transcript stays
    /// in `run_error`, because a linker failure has no line to point at.
    diagnostics: Vec<editor::Diagnostic>,
    /// A line and column a click asked the editor to put the caret on. Taken by
    /// the sketch panel on the frame after the click.
    pending_jump: Option<(usize, usize)>,
    path: Option<PathBuf>,
    file_error: Option<String>,
    presets: Vec<scene::Preset>,
    auto_run: bool,
    fit_requested: bool,
}

impl Spike {
    /// The app as a reader starts it: the English shell, in the language their
    /// last run saved.
    ///
    /// Separate from `Default` on purpose. `Default` is what the tests build, and
    /// a test that read this machine's config file would pass or fail depending
    /// on who ran it.
    fn starting_in(locale: locales::Locale) -> Self {
        let mut spike = Self::default();
        spike.texts.set_locale(locale);
        spike
    }
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
            wire_waypoints: Vec::new(),
            selection: Selection::default(),
            marquee: None,
            clipboard: Clipboard::default(),
            dragging: None,
            drag_origin: Vec::new(),
            drag_travel: Vec2::ZERO,
            hovered_pin: None,
            textures: HashMap::new(),
            led_states: HashMap::new(),
            led_brightness: HashMap::new(),
            outputs: HashMap::new(),
            rgb_leds: HashMap::new(),
            sensors: HashMap::new(),
            shift_states: HashMap::new(),
            shift_wiring: HashMap::new(),
            responder: Arc::new(Mutex::new(sensors::Responder::default())),
            pin_responder: None,
            pressed: HashMap::new(),
            keypad_keys: HashMap::new(),
            keypad_until: HashMap::new(),
            keypad_frames: HashMap::new(),
            press_drag: None,
            joystick: None,
            joystick_moved: false,
            joystick_start: None,
            knob: None,
            slide_grab: 0.0,
            last_drives: HashMap::new(),
            last_analogs: HashMap::new(),
            lcd_wiring: HashMap::new(),
            lcd_states: HashMap::new(),
            reevaluate: false,
            ir_receivers: HashMap::new(),
            remote_hold: None,
            stepper_wiring: HashMap::new(),
            stepper_phases: HashMap::new(),
            stepper_angles: HashMap::new(),
            buzzer: buzzer::BuzzerAudio::default(),
            pin_pulses: HashMap::new(),
            pin_duty: HashMap::new(),
            pin_frequency: HashMap::new(),
            pin13: false,
            sketch_open: true,
            components_open: true,
            serial_open: false,
            serial_collapsed: false,
            palette_drag: None,
            // English, always: `Default` is the shell the tests build, and it
            // must not depend on whatever language this machine saved last. The
            // app starts through `starting_in`, which applies the saved one.
            texts: locales::Texts::load(locales::Locale::English),
            part_warnings: HashMap::new(),
            segment_memory: HashMap::new(),
            seven_changed_at: HashMap::new(),
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
            diagnostics: Vec::new(),
            pending_jump: None,
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

/// The pin under the pointer, ignoring any pin that a part is drawn over.
///
/// A pin is not a layer of its own: it is drawn with the part that owns it, so a
/// part lying on top of it has to be what the pointer finds. Testing every pin
/// regardless let a covered pin be grabbed through the piece covering it.
fn hit_pin(parts: &[Part], world: Pos2, radius: f32) -> Option<PinRef> {
    let radius_sq = radius * radius;
    for (part_index, part) in parts.iter().enumerate().rev() {
        for (pin_index, _) in part.pins.iter().enumerate().rev() {
            let at = part.pin_world(pin_index);
            if at.distance_sq(world) > radius_sq {
                continue;
            }
            // The test is at the pin's own position, not the pointer's: a pin
            // sticks out past its body, so the pointer can be beside the part
            // that would hide it.
            let covering = hit_part(parts, at);
            if covering.is_none_or(|covering| covering == part_index) {
                return Some((part_index, pin_index));
            }
        }
    }
    None
}

/// The nudge that makes a dragged pick's pin land exactly on a still pin.
///
/// v0.2 snapped while you moved: a pin that came within a few pixels of another
/// part's pin jumped the rest of the way, which is how a leg ends up sitting in a
/// breadboard hole instead of beside it. The closest such pair wins, and the
/// nudge is the whole gap between them, so the part stops a hair short rather
/// than overlapping. `moving` is the whole selection: a pin on any dragged part
/// can be the one that locks, and the nudge moves them all, which keeps the pick's
/// own shape.
///
/// The drag itself goes through [`snap_nudge_at`], which measures from positions
/// the pick does not occupy yet; this form, at the pick's current centres, is what
/// the tests state their cases in.
#[cfg(test)]
fn snap_nudge(parts: &[Part], moving: &[usize], radius: f32) -> Option<Vec2> {
    let centres: Vec<(usize, Pos2)> = moving
        .iter()
        .filter_map(|&index| parts.get(index).map(|part| (index, part.center)))
        .collect();
    snap_nudge_at(parts, &centres, radius)
}

/// The same nudge, measured from positions the pick does not occupy yet.
///
/// A drag works out where the pick *would* be from the pointer's travel, and the
/// snap has to be measured there. Measured at the snapped position it produced
/// last frame, the gap is always zero, so the nudge re-applies every frame and the
/// part cannot be dragged off the pin — it has to be ripped.
fn snap_nudge_at(parts: &[Part], moving: &[(usize, Pos2)], radius: f32) -> Option<Vec2> {
    let mut best: Option<(f32, Vec2)> = None;
    for (index, centre) in moving {
        let Some(part) = parts.get(*index) else {
            continue;
        };
        for pin in 0..part.pins.len() {
            let from = *centre + scene::rotate_offset(part.pins[pin].offset, part.rotation);
            for (other_index, other) in parts.iter().enumerate() {
                // A pick cannot snap to itself, or a wire's two ends would pull
                // the part back to where it already is.
                if moving.iter().any(|(index, _)| *index == other_index) {
                    continue;
                }
                for other_pin in 0..other.pins.len() {
                    let offset = other.pin_world(other_pin) - from;
                    let distance = offset.length();
                    if distance > radius || distance <= f32::EPSILON {
                        continue;
                    }
                    if best.is_none_or(|(closest, _)| distance < closest) {
                        best = Some((distance, offset));
                    }
                }
            }
        }
    }
    best.map(|(_, offset)| offset)
}

/// The parts a drag has to carry: the pick itself, plus anything seated in a
/// breadboard that is part of the pick.
///
/// A breadboard is a part like any other, so moving one used to slide the board
/// out from under the circuit built on it and leave the legs hanging in space. A
/// leg that is in a hole sits within a hole's width of the hole's own pin, so
/// proximity is the whole test — the same coincidence the netlist uses to join a
/// strip.
fn seated_with(parts: &[Part], pick: &[usize]) -> Vec<usize> {
    /// How close a leg has to be to a hole to count as being in it, in world
    /// units. A breadboard hole is about 2.5 units across.
    const SEATED_RADIUS: f32 = 2.5;

    let mut moving: Vec<usize> = pick.to_vec();
    for &index in pick {
        let Some(board) = parts.get(index) else {
            continue;
        };
        let Some(kind) = parse_kind(&board.kind) else {
            continue;
        };
        if !matches!(kind, PartKind::Breadboard | PartKind::BreadboardFull) {
            continue;
        }
        let holes: Vec<Pos2> = (0..board.pins.len()).map(|pin| board.pin_world(pin)).collect();
        let radius_sq = SEATED_RADIUS * SEATED_RADIUS;
        for (other_index, other) in parts.iter().enumerate() {
            if moving.contains(&other_index) {
                continue;
            }
            let within = |pin: usize| {
                let at = other.pin_world(pin);
                holes
                    .iter()
                    .any(|hole| hole.distance_sq(at) <= radius_sq)
            };
            if (0..other.pins.len()).any(within) {
                moving.push(other_index);
            }
        }
    }
    moving
}

/// Where a dragged pick lands this frame: where the pointer has taken it, plus the
/// nudge that closes a pin's gap if one is within reach of the raw position.
///
/// Nothing here reads the pick's current centre, so the answer depends only on
/// where the drag started and how far it has gone. That is what makes a snap
/// something the part can leave again.
fn drag_placement(
    parts: &[Part],
    origin: &[(usize, Pos2)],
    travel: Vec2,
    radius: f32,
) -> Vec<(usize, Pos2)> {
    let raw: Vec<(usize, Pos2)> = origin
        .iter()
        .map(|(index, start)| (*index, *start + travel))
        .collect();
    let nudge = snap_nudge_at(parts, &raw, radius).unwrap_or(Vec2::ZERO);
    raw.into_iter()
        .map(|(index, at)| (index, at + nudge))
        .collect()
}

/// What the pointer is on.
///
/// The order is the canvas's own: pins are the smallest target and the one a wire
/// is drawn from, so they are asked first; wires are painted after the parts, so a
/// wire crossing a piece is what the pointer is on; then the parts themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pick {
    Pin(PinRef),
    Wire(usize),
    Part(usize),
    Nothing,
}

fn pick_at(
    parts: &[Part],
    wires: &[Wire],
    camera: Camera,
    rect: Rect,
    screen: Pos2,
    pin_radius: f32,
) -> Pick {
    let world = camera.screen_to_world(rect, screen);
    if let Some(pin) = hit_pin(parts, world, pin_radius) {
        return Pick::Pin(pin);
    }
    if let Some(index) = hit_wire(parts, wires, camera, rect, screen, WIRE_HIT_RADIUS) {
        return Pick::Wire(index);
    }
    if let Some(index) = hit_part(parts, world) {
        return Pick::Part(index);
    }
    Pick::Nothing
}

/// A wire's route in screen space: the pins either end, the bends between them,
/// and the orthogonal corners the router adds — the polyline the painter draws.
fn wire_screen_path(part_list: &[Part], wire: &Wire, camera: Camera, rect: Rect) -> Vec<Pos2> {
    let a = camera.world_to_screen(rect, part_list[wire.a.0].pin_world(wire.a.1));
    let b = camera.world_to_screen(rect, part_list[wire.b.0].pin_world(wire.b.1));
    let mut route = vec![CorePoint::new(a.x as f64, a.y as f64)];
    route.extend(wire.waypoints.iter().map(|point| {
        let at = camera.world_to_screen(rect, *point);
        CorePoint::new(at.x as f64, at.y as f64)
    }));
    route.push(CorePoint::new(b.x as f64, b.y as f64));
    expand_path(&route)
        .into_iter()
        .map(|point| Pos2::new(point.x as f32, point.y as f32))
        .collect()
}

/// Distance from a point to a line segment.
fn distance_to_segment(point: Pos2, a: Pos2, b: Pos2) -> f32 {
    let along = b - a;
    let length_sq = along.length_sq();
    if length_sq <= f32::EPSILON {
        return point.distance(a);
    }
    let t = ((point - a).dot(along) / length_sq).clamp(0.0, 1.0);
    point.distance(a + along * t)
}

/// The topmost wire within `tolerance` screen pixels of a point.
fn hit_wire(
    part_list: &[Part],
    wires: &[Wire],
    camera: Camera,
    rect: Rect,
    screen: Pos2,
    tolerance: f32,
) -> Option<usize> {
    wires.iter().enumerate().rev().find_map(|(index, wire)| {
        let path = wire_screen_path(part_list, wire, camera, rect);
        let near = path
            .windows(2)
            .any(|pair| distance_to_segment(screen, pair[0], pair[1]) <= tolerance);
        near.then_some(index)
    })
}

/// Whether a segment crosses a rectangle at all. Liang–Barsky, so a wire drawn
/// right through the marquee counts even though neither end is inside it.
fn segment_hits_rect(a: Pos2, b: Pos2, rect: Rect) -> bool {
    if rect.contains(a) || rect.contains(b) {
        return true;
    }
    let direction = b - a;
    let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
    for (p, q) in [
        (-direction.x, a.x - rect.min.x),
        (direction.x, rect.max.x - a.x),
        (-direction.y, a.y - rect.min.y),
        (direction.y, rect.max.y - a.y),
    ] {
        if p.abs() <= f32::EPSILON {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                if r > t1 {
                    return false;
                }
                t0 = t0.max(r);
            } else {
                if r < t0 {
                    return false;
                }
                t1 = t1.min(r);
            }
        }
    }
    true
}

fn texture_key(
    part: &Part,
    lit: bool,
    outputs: &HashMap<String, OutputState>,
    led_brightness: &HashMap<String, f64>,
    pressed: &HashMap<String, bool>,
    pin13: bool,
) -> String {
    if part.kind == "pushbutton" {
        // The cap's fill is the whole difference, so two textures are enough.
        let down = pressed.get(&part.id).copied().unwrap_or(false);
        return format!("button:{down}");
    }
    if part.kind == "uno" {
        return format!("uno:{pin13}");
    }
    if part.kind == "led" {
        // A PWM LED is as bright as its duty cycle, so the art has to follow it.
        // Sixteen levels keep the cache small and the ramp smooth enough to read
        // as a fade rather than as steps.
        let colour = part.attrs.get("color").map(String::as_str).unwrap_or("red");
        let brightness = if lit {
            led_brightness.get(&part.id).copied().unwrap_or(1.0)
        } else {
            0.0
        };
        let level = (brightness.clamp(0.0, 1.0) * 16.0).round() as u8;
        return format!("led:{colour}:{lit}:{level}");
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
    if part.kind == "potentiometer" {
        // The tick turns live, so the art does not depend on the value: baking
        // it per value cost ~40 ms a state, measured, on every step of a drag.
        return "pot".into();
    }
    if part.kind == "slide-potentiometer" {
        return "slide".into();
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
    led_states: &HashMap<String, LedState>,
    outputs: &HashMap<String, OutputState>,
    led_brightness: &HashMap<String, f64>,
    pressed: &HashMap<String, bool>,
    pin13: bool,
) {
    for part in parts {
        let lit = led_states
            .get(&part.id)
            .is_some_and(|state| !matches!(state, LedState::Off));
        let key = texture_key(part, lit, outputs, led_brightness, pressed, pin13);
        {
            if textures.contains_key(&key) {
                continue;
            }
            let svg = if part.kind == "led" {
                let brightness = if lit {
                    led_brightness.get(&part.id).copied().unwrap_or(1.0)
                } else {
                    0.0
                };
                scene::led_state_svg(
                    &part.svg,
                    part.attrs.get("color").map(String::as_str).unwrap_or("red"),
                    brightness,
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
            } else if part.kind == "pushbutton" {
                let down = pressed.get(&part.id).copied().unwrap_or(false);
                scene::pushbutton_svg(&part.svg, down)
            } else if part.kind == "uno" {
                scene::uno_svg(&part.svg, pin13)
            } else if part.kind == "potentiometer" {
                scene::potentiometer_svg(&part.svg)
            } else if part.kind == "slide-potentiometer" {
                scene::slide_pot_svg(&part.svg)
            } else if part.kind == "rgb-led" {
                part.svg.clone()
            } else if part.kind == "analog-joystick" {
                scene::joystick_svg(&part.svg)
            } else if part.kind == "servo" {
                let angle = outputs
                    .get(&part.id)
                    .and_then(|out| out.angle)
                    .unwrap_or(0.0);
                scene::servo_svg(&part.svg, angle)
            } else if part.kind == "stepper-motor" {
                // No angle in the art: the rotor is drawn live. Baking it in meant
                // a full re-rasterisation of the motor for every angle it passed
                // through, which is what made a turning stepper stall.
                scene::stepper_base_svg(&part.svg)
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
        self.sketch = SKETCH_TEMPLATE.to_string();
        self.selection.clear();
        self.wire_start = None;
        self.led_states.clear();
        self.led_brightness.clear();
        self.outputs.clear();
        self.rgb_leds.clear();
        self.sensors.clear();
        self.shift_states.clear();
        self.shift_wiring.clear();
        if let Ok(mut responder) = self.responder.lock() {
            responder.clear();
        }
        self.lcd_states.clear();
        self.forget_input();
        self.pin_pulses.clear();
        self.segment_memory.clear();
        self.seven_changed_at.clear();
        self.seven_stable.clear();
        self.stepper_phases.clear();
        self.stepper_angles.clear();
        self.servo_angles.clear();
        self.serial.clear();
        self.run_error = None;
        self.diagnostics.clear();
        self.pending_jump = None;
        self.file_error = None;
        self.path = None;
        // The canvas art belongs to the parts that were just cleared; the palette
        // thumbnails belong to the catalogue and are still what the panel draws.
        keep_palette(&mut self.textures);
        self.camera = Camera::default();
    }

    fn open_path(&mut self, path: PathBuf, ctx: &egui::Context) {
        self.stop_sim();
        let (parts, wires, sketch) = scene::load_diagram_file(&path);
        if parts.is_empty() {
            self.file_error = Some(
                self.texts
                    .tf("error.load", &[("name", &path.display().to_string())]),
            );
            return;
        }
        // Rasterise the new scene before swapping it in. Otherwise the canvas
        // paints the blue fallback for a frame while textures load.
        ensure_textures(
            ctx,
            &mut self.textures,
            &parts,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &self.pressed,
            self.pin13,
        );
        self.parts = parts;
        self.wires = wires;
        self.sketch = sketch;
        self.path = Some(path);
        self.selection.clear();
        self.wire_start = None;
        self.serial.clear();
        self.run_error = None;
        self.diagnostics.clear();
        self.pending_jump = None;
        self.file_error = None;
        self.camera = Camera::default();
        let eval =
            scene::evaluate_scene(
                &self.parts,
                &self.wires,
                &self.pressed,
                &HashMap::new(),
                &self.pin_duty,
                &self.pin_pulses,
                &self.keypad_keys,
                &self.shift_states,
            &self.pin_frequency,
        );
        self.store_eval(eval);
    }

    fn open_dialog(&mut self, ctx: &egui::Context) {
        // Owned before the dialog opens: the picker's result is used inside a
        // body that borrows `self` mutably.
        let filter = self.texts.t("file.document_filter").to_string();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(&filter, &["json"])
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
        let filter = self.texts.t("file.document_filter").to_string();
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(&filter, &["json"])
            .set_file_name("diagram.json")
            .save_file()
        {
            self.save_to(path);
        }
    }

    /// Take one evaluation: hold its inputs for the sketch, then keep the
    /// states the canvas draws from.
    fn store_eval(&mut self, eval: CircuitEval) {
        // Hand the circuit's wishes to the simulator: a pressed button pulling
        // its pin low, a keypad column, a potentiometer's voltage. Without this
        // the netlist is computed, drawn, and never felt by the sketch — which
        // is why buttons and keypads did nothing while the IR remote, which
        // pulses the simulator directly, worked.
        self.apply_drives(&eval.drives, &eval.analogs);
        self.part_warnings = eval.warnings;
        self.led_states = eval.leds;
        self.led_brightness = eval.led_brightness;
        self.outputs = eval.outputs;
        self.rgb_leds = eval.rgb_leds;
        self.sensors = eval.sensors;
        self.shift_states = eval.shift_registers.clone();
        self.shift_wiring = eval.shift_wiring.clone();
        if let Ok(mut responder) = self.responder.lock() {
            responder.set_circuit(
                eval.echoes
                    .iter()
                    .map(|echo| {
                        (
                            format!("{}{}", echo.trigger.0, echo.trigger.1),
                            (
                                echo.echo.0 as u8,
                                echo.echo.1,
                                u64::from(echo.delay_us),
                                u64::from(echo.width_us),
                            ),
                        )
                    })
                    .collect(),
                eval.dht_sensors
                    .values()
                    .map(|sensor| (sensor.pin.clone(), (sensor.humidity, sensor.temperature)))
                    .collect(),
            );
        }
        self.lcd_wiring = eval.lcd_wiring;
        self.ir_receivers = eval.ir_receivers;
        self.stepper_wiring = eval.stepper_wiring;
        self.apply_output_memory();
        self.apply_stepper_angles();
    }

    /// One enable pulse ended: hand the four data lines to every LCD that uses
    /// this pin, in whichever register its RS line selects.
    fn latch_lcds(&mut self, enable_key: &str) {
        let level = |pins: &HashMap<String, bool>, key: &str| {
            pins.get(key).copied().unwrap_or(false)
        };
        for (id, wiring) in &self.lcd_wiring {
            if wiring.pins.e != enable_key {
                continue;
            }
            let data = [
                level(&self.avr_pins, &wiring.pins.d4),
                level(&self.avr_pins, &wiring.pins.d5),
                level(&self.avr_pins, &wiring.pins.d6),
                level(&self.avr_pins, &wiring.pins.d7),
            ];
            let rs = level(&self.avr_pins, &wiring.pins.rs);
            let state = self.lcd_states.entry(id.clone()).or_default();
            lcd::latch(state, rs, data);
        }
    }

    /// Turn each stepper by the step its coils just moved to.
    ///
    /// The core reports the wiring and the step angle and leaves the shaft
    /// alone, so the angle is remembered here — one step of `360 / steps` per
    /// phase change, the way the driver would turn it.
    fn advance_steppers(&mut self) {
        for (id, wiring) in &self.stepper_wiring {
            if wiring.pins.len() < 4 {
                continue;
            }
            let levels = [
                self.avr_pins.get(&wiring.pins[0]).copied().unwrap_or(false),
                self.avr_pins.get(&wiring.pins[1]).copied().unwrap_or(false),
                self.avr_pins.get(&wiring.pins[2]).copied().unwrap_or(false),
                self.avr_pins.get(&wiring.pins[3]).copied().unwrap_or(false),
            ];
            let Some(phase) = stepper::phase_of(levels) else {
                continue;
            };
            match self.stepper_phases.get(id).copied() {
                None => {
                    self.stepper_phases.insert(id.clone(), phase);
                }
                Some(previous) if previous == phase => {}
                Some(previous) => {
                    let step = stepper::step_degrees(previous, phase, wiring.step_angle);
                    let angle = self.stepper_angles.get(id).copied().unwrap_or(0.0) + step;
                    self.stepper_angles.insert(id.clone(), angle);
                    self.stepper_phases.insert(id.clone(), phase);
                }
            }
        }
    }

    /// Follow the pointer with a knob: the potentiometer's angle or the slide
    /// pot's position, written into its attribute — where the netlist reads it.
    fn drag_knob(&mut self, index: usize, world: Pos2) {
        let part = &self.parts[index];
        let value = match part.kind.as_str() {
            "potentiometer" => knobs::pot_value(part, world),
            "slide-potentiometer" => Some(knobs::slide_value_grabbed(part, world, self.slide_grab)),
            _ => None,
        };
        let Some(value) = value else {
            return;
        };
        let text = if (value - value.round()).abs() < 1e-9 {
            format!("{}", value.round() as i64)
        } else {
            format!("{value}")
        };
        if self.parts[index].attrs.get("value") == Some(&text) {
            return;
        }
        self.parts[index].attrs.insert("value".into(), text);
        // The netlist reads the attribute, so a turn has to be solved for.
        self.reevaluate = true;
    }

    /// Put each stepper's remembered angle on its part, which is what the art
    /// is drawn from.
    fn apply_stepper_angles(&mut self) {
        for (id, angle) in &self.stepper_angles {
            if !self.stepper_wiring.contains_key(id) {
                continue;
            }
            let out = self.outputs.entry(id.clone()).or_default();
            out.angle = Some(*angle);
        }
    }

    /// Send one NEC frame down every wired receiver's pin, the way pressing the
    /// button on the remote does.
    fn send_ir(&mut self, command: u8) {
        let durations = remote::nec_durations(command);
        let Some(sim) = self.sim.as_ref() else {
            return;
        };
        for receiver in self.ir_receivers.values() {
            let mut parts = receiver.pin.chars();
            let Some(port) = parts.next() else {
                continue;
            };
            let Ok(pin) = parts.as_str().parse::<u8>() else {
                continue;
            };
            sim.pulse_script(port as u8, pin, 0, durations.clone());
        }
    }

    /// Push what the netlist makes of the input into the simulator: a held
    /// button pulls its pin, a sensor's comparator drives its output.
    fn apply_drives(&mut self, drives: &[Drive], analogs: &[AnalogReading]) {
        let Some(sim) = self.sim.as_ref() else {
            return;
        };
        for drive in drives {
            let key = (drive.port, drive.pin);
            if self.last_drives.get(&key) == Some(&drive.high) {
                continue;
            }
            sim.set_pin(drive.port as u8, drive.pin, drive.high);
            self.last_drives.insert(key, drive.high);
        }
        for reading in analogs {
            if self.last_analogs.get(&reading.channel) == Some(&reading.millivolts) {
                continue;
            }
            sim.set_analog(reading.channel, reading.millivolts);
            self.last_analogs.insert(reading.channel, reading.millivolts);
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
                let shown = self.seven_stable.get(&part.id).cloned();
                let since = self
                    .seven_changed_at
                    .get(&part.id)
                    .map(|at| now.duration_since(*at));
                let next = match &shown {
                    Some(shown) if !segment_settled(shown, &values, since) => shown.clone(),
                    _ => {
                        self.seven_changed_at.insert(part.id.clone(), now);
                        self.seven_stable.insert(part.id.clone(), values.clone());
                        values
                    }
                };
                if let Some(out) = self.outputs.get_mut(&part.id) {
                    out.values = next;
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

    /// Take a press on a part: hold a button, tap a key, grab the stick, or
    /// start dragging everything else. `screen` is where the pointer was, for
    /// the stick's movement threshold.
    /// A press landed on a pin: that is the start of a wire, not a grab of the
    /// part the pin belongs to.
    fn wire_press(&mut self, pin: PinRef) {
        if self.wire_start.is_none() {
            self.wire_start = Some(pin);
        }
    }

    /// Pin a bend at `world` while a wire is being routed.
    ///
    /// The corner of the current leg is committed, not the cursor: the next leg
    /// then starts where the wire visibly turns. Storing the cursor instead puts
    /// the wire through the pointer and needs two extra elbows to do it, which is
    /// what made a routed wire look like it wandered. A leg that is already
    /// straight has no corner, so the cursor stands.
    fn wire_bend(&mut self, world: Pos2) {
        let Some(start) = self.wire_start else {
            return
        };
        let last = self
            .wire_waypoints
            .last()
            .copied()
            .unwrap_or_else(|| self.parts[start.0].pin_world(start.1));
        let corner = leg_bend(
            self.wire_waypoints.len(),
            CorePoint::new(last.x as f64, last.y as f64),
            CorePoint::new(world.x as f64, world.y as f64),
        )
        .map(|bend| Pos2::new(bend.x as f32, bend.y as f32))
        .unwrap_or(world);
        self.wire_waypoints.push(corner);
    }

    /// Give up on the wire being drawn.
    fn cancel_wire(&mut self) {
        self.wire_start = None;
        self.wire_waypoints.clear();
    }

    /// The pointer came up over `pin`, or over nothing at all.
    ///
    /// Releasing on a different pin finishes the wire; releasing anywhere else
    /// leaves it armed, which is what makes click-then-click work as well as
    /// drag-and-drop.
    fn wire_release(&mut self, pin: Option<PinRef>) {
        let (Some(start), Some(target)) = (self.wire_start, pin) else {
            return;
        };
        if start == target {
            return;
        }
        self.wires.push(Wire {
            a: start,
            b: target,
            waypoints: std::mem::take(&mut self.wire_waypoints),
        });
        self.wire_start = None;
        self.reevaluate = true;
    }

    fn press_control(&mut self, index: usize, world: Pos2, screen: Pos2) {
        // A button or a key does not change a pin; it changes what the netlist
        // is told. Without this the press is recorded and never solved, which is
        // why buttons and keypads did nothing at all.
        self.reevaluate = true;
        let id = self.parts[index].id.clone();
        match self.parts[index].kind.as_str() {
            "pushbutton" => {
                self.pressed.insert(id, true);
                // Movable, but not by the jitter inside a click: a real drag
                // promotes this in the drag branch.
                self.press_drag = Some((index, screen));
            }
            "membrane-keypad" => {
                if let Some(key) = keypad_key(&self.parts[index], world) {
                    self.keypad_keys.insert(id.clone(), Some(key));
                    self.keypad_until.insert(id.clone(), Instant::now() + KEY_HOLD);
                    self.keypad_frames.insert(id, KEY_HOLD_FRAMES);
                }
                self.press_drag = Some((index, screen));
            }
            "analog-joystick" => {
                if knobs::on_stick(&self.parts[index], world) {
                    self.joystick = Some(index);
                    self.joystick_moved = false;
                    self.joystick_start = Some(screen);
                    self.pressed.insert(id, true);
                } else {
                    // The stick is the thing that moves; the body is a part.
                    self.dragging = Some(index);
                }
            }
            // A knob is grabbed where it is and follows the pointer.
            "potentiometer" => {
                if knobs::on_pot_knob(&self.parts[index], world) {
                    self.knob = Some(index);
                    self.drag_knob(index, world);
                } else {
                    self.dragging = Some(index);
                }
            }
            "slide-potentiometer" => {
                if knobs::on_slide_handle(&self.parts[index], world) {
                    self.knob = Some(index);
                    // Remember where on the handle the grab landed, so it
                    // follows the pointer instead of jumping to it.
                    self.slide_grab = knobs::slide_grab_offset(&self.parts[index], world);
                } else {
                    self.dragging = Some(index);
                }
            }
            // The remote is a transmitter, not a switch: pressing a button sends
            // one NEC frame to every receiver in the circuit.
            "ir-remote" => {
                if let Some((button, command)) = remote::button_at(&self.parts[index], world) {
                    self.remote_hold = Some((index, button));
                    self.send_ir(command);
                } else {
                    // Every button has a code; the body is not a button, so a
                    // press there picks the remote up and moves it.
                    self.dragging = Some(index);
                }
            }
            _ => self.dragging = Some(index),
        }
    }

    /// Drop every held input and any pending keypad tap.
    fn forget_input(&mut self) {
        self.release_input();
        self.keypad_keys.clear();
        self.keypad_until.clear();
    }

    /// Nothing stays held once the pointer lifts: buttons and the stick are
    /// momentary. A keypad key keeps its minimum hold, so `tick_input` ends it.
    fn release_input(&mut self) {
        self.reevaluate = true;
        self.pressed.clear();
        self.remote_hold = None;
        self.knob = None;
        self.slide_grab = 0.0;
        if let Some(index) = self.joystick.take() {
            // A stick is a momentary input, not a positioned part.
            self.parts[index].attrs.insert("x".into(), "50".into());
            self.parts[index].attrs.insert("y".into(), "50".into());
        }
        self.joystick_moved = false;
        self.joystick_start = None;
    }

    /// The keyboard's share of the canvas. Skipped while the sketch has the
    /// keyboard, so Ctrl+C in the editor copies text and Delete deletes a
    /// character rather than a part.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (delete, copy, cut, paste) = ctx.input(|input| {
            (
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace),
                input.modifiers.command && input.key_pressed(egui::Key::C),
                input.modifiers.command && input.key_pressed(egui::Key::X),
                input.modifiers.command && input.key_pressed(egui::Key::V),
            )
        });
        // R turns a quarter clockwise, Shift+R half a turn — v0.2's bindings.
        let (turn, half_turn) = ctx.input(|input| {
            (
                !input.modifiers.command && !input.modifiers.shift && input.key_pressed(egui::Key::R),
                !input.modifiers.command && input.modifiers.shift && input.key_pressed(egui::Key::R),
            )
        });
        if half_turn {
            self.rotate_selection(180.0);
        } else if turn {
            self.rotate_selection(90.0);
        }
        if copy {
            self.copy_selection();
        }
        if cut {
            self.copy_selection();
            self.delete_selection();
        }
        if delete {
            self.delete_selection();
        }
        if paste {
            self.paste();
        }
    }

    /// Turn the selected parts. The pins move with them, because `pin_world`
    /// applies the rotation, so the wires follow without being touched.
    fn rotate_selection(&mut self, degrees: f32) {
        if self.selection.parts.is_empty() || self.running {
            return;
        }
        for index in self.selection.parts.clone() {
            if let Some(part) = self.parts.get_mut(index) {
                part.rotation = (part.rotation + degrees).rem_euclid(360.0);
            }
        }
        self.reevaluate = true;
    }

    /// Copy the selected parts and the wires among them.
    fn copy_selection(&mut self) {
        if self.selection.parts.is_empty() {
            return;
        }
        let mut picked = self.selection.parts.clone();
        picked.sort_unstable();
        let clipboard_parts: Vec<Part> = picked
            .iter()
            .filter_map(|index| self.parts.get(*index).cloned())
            .collect();
        let mut clipboard_wires = Vec::new();
        for wire in &self.wires {
            let a = picked.iter().position(|index| *index == wire.a.0);
            let b = picked.iter().position(|index| *index == wire.b.0);
            if let (Some(a), Some(b)) = (a, b) {
                clipboard_wires.push(((a, wire.a.1), (b, wire.b.1), wire.waypoints.clone()));
            }
        }
        self.clipboard = Clipboard {
            parts: clipboard_parts,
            wires: clipboard_wires,
        };
    }

    /// Paste the clipboard a grid step down and right, and select the copies.
    fn paste(&mut self) {
        if self.clipboard.parts.is_empty() || self.running {
            return;
        }
        self.selection.clear();
        let base = self.parts.len();
        let offset = Vec2::new(GRID, GRID);
        for (index, original) in self.clipboard.parts.iter().enumerate() {
            let mut copy = original.clone();
            copy.id = format!("{}-{}", original.kind, self.next_id + index);
            copy.center += offset;
            self.parts.push(copy);
            self.selection.parts.push(base + index);
        }
        self.next_id += self.clipboard.parts.len();
        for (a, b, waypoints) in &self.clipboard.wires {
            self.wires.push(Wire {
                a: (base + a.0, a.1),
                b: (base + b.0, b.1),
                // The bends come along, so a routed wire pastes routed.
                waypoints: waypoints.iter().map(|point| *point + offset).collect(),
            });
        }
        self.reevaluate = true;
    }

    /// Delete the selection: the wires outright, the parts and anything wired to
    /// them. Indices are walked high to low so a removal cannot move the next.
    fn delete_selection(&mut self) {
        if self.selection.is_empty() || self.running {
            return;
        }
        let mut wires = self.selection.wires.clone();
        wires.sort_unstable_by(|a, b| b.cmp(a));
        for index in wires {
            if index < self.wires.len() {
                self.wires.remove(index);
            }
        }
        let mut parts = self.selection.parts.clone();
        parts.sort_unstable_by(|a, b| b.cmp(a));
        for index in parts {
            if index >= self.parts.len() {
                continue;
            }
            self.parts.remove(index);
            self.wires.retain(|wire| wire.a.0 != index && wire.b.0 != index);
            for wire in &mut self.wires {
                if wire.a.0 > index {
                    wire.a.0 -= 1;
                }
                if wire.b.0 > index {
                    wire.b.0 -= 1;
                }
            }
        }
        self.selection.clear();
        self.reevaluate = true;
    }

    /// Select everything inside a world-space box (the marquee's release), or
    /// add to the selection when `additive`.
    fn select_in(&mut self, area: Rect, additive: bool) {
        if !additive {
            self.selection.clear();
        }
        for index in 0..self.parts.len() {
            let inside = self.parts[index].bounds().intersects(area);
            if inside && !self.selection.has_part(index) {
                self.selection.parts.push(index);
            }
        }
        let mut inside_wires = Vec::new();
        for (index, wire) in self.wires.iter().enumerate() {
            let a = self.parts[wire.a.0].pin_world(wire.a.1);
            let b = self.parts[wire.b.0].pin_world(wire.b.1);
            let mut points = vec![a];
            points.extend(wire.waypoints.iter().copied());
            points.push(b);
            let inside = points
                .windows(2)
                .any(|pair| segment_hits_rect(pair[0], pair[1], area));
            if inside {
                inside_wires.push(index);
            }
        }
        for index in inside_wires {
            if !self.selection.has_wire(index) {
                self.selection.wires.push(index);
            }
        }
    }

    /// End keypad taps whose minimum hold has passed.
    fn tick_input(&mut self) {
        let now = Instant::now();
        for frames in self.keypad_frames.values_mut() {
            *frames = frames.saturating_sub(1);
        }
        let expired: Vec<String> = self
            .keypad_until
            .iter()
            .filter(|(id, until)| {
                **until <= now && self.keypad_frames.get(*id).copied().unwrap_or(0) == 0
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.keypad_until.remove(&id);
            self.keypad_frames.remove(&id);
            self.keypad_keys.insert(id, None);
            self.reevaluate = true;
        }
    }

    fn start_sim(&mut self, ctx: &egui::Context) {
        self.run_error = None;
        self.diagnostics.clear();
        self.serial.clear();
        self.avr_pins.clear();
        self.last_drives.clear();
        self.last_analogs.clear();
        // Evaluate once before the simulator starts. This is what learns the
        // LCDs' wiring, and a sketch that writes to one in `setup()` sends all
        // of its enable pulses before the first poll would have learned it.
        let eval = scene::evaluate_scene(
            &self.parts,
            &self.wires,
            &self.pressed,
            &self.avr_pins,
            &self.pin_duty,
            &self.pin_pulses,
            &self.keypad_keys,
            &self.shift_states,
            &self.pin_frequency,
        );
        self.store_eval(eval);
        let workdir = std::env::temp_dir().join(format!("croquis-egui-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&workdir);
        match engine::compile(&self.sketch, &workdir) {
            Ok(firmware) => {
                let (sender, receiver) = mpsc::channel();
                let repaint = ctx.clone();
                // A DHT read gives up after about a millisecond, so the answer
                // goes out here, on the thread that sees the edge, rather than
                // from the frame loop tens of milliseconds later.
                // Answering from inside simavr's pin callback raises its IRQ
                // re-entrantly, and a DHT's forty-edge frame comes out
                // truncated: measured as `start 80 41` on the sketch's own
                // clock. So the event goes to a thread of its own, which is
                // outside that callback and still only microseconds behind.
                let (pin_sender, pin_receiver) = mpsc::channel::<(String, u8, u8, bool)>();
                let responder = self.responder.clone();
                std::thread::spawn(move || {
                    while let Ok((key, port, pin, high)) = pin_receiver.recv() {
                        if let Ok(mut responder) = responder.lock() {
                            responder.on_pin(&key, port, pin, high);
                        }
                    }
                });
                self.pin_responder = Some(pin_sender.clone());
                let emit: engine::Emit = Arc::new(move |event| {
                    if let engine::SimEvent::Pin { port, pin, high } = &event {
                        let key = format!("{}{}", *port as char, pin);
                        let _ = pin_sender.send((key, *port, *pin, *high));
                    }
                    let _ = sender.send(event);
                    repaint.request_repaint();
                });
                match engine::Simulator::start(Path::new(&firmware.elf), emit, false) {
                    Ok(sim) => {
                        // The responder can answer now: it holds the same pulse
                        // channels the simulator listens on.
                        if let Ok(mut responder) = self.responder.lock() {
                            responder.set_pulses(Some(sim.pulse_handle()));
                        }
                        self.sim = Some(sim);
                        self.events = Some(receiver);
                        self.running = true;
                    }
                    Err(error) => self.run_error = Some(error),
                }
            }
            Err(error) => {
                // The raw transcript still goes in the run error: a linker
                // failure has nothing the caret can be put on, and dropping it
                // would leave the top bar saying only "failed".
                self.diagnostics = editor::parse_diagnostics(&error);
                self.run_error = Some(error);
                // v0.2 shows these in the serial panel, so it opens with them
                // rather than waiting to be found under the View menu.
                self.serial_open = true;
            }
        }
    }

    fn stop_sim(&mut self) {
        // Ends the responder thread: nothing more to answer for.
        self.pin_responder = None;
        if let Some(mut sim) = self.sim.take() {
            sim.stop();
        }
        self.events = None;
        self.running = false;
        self.forget_input();
        self.avr_pins.clear();
        self.pin_pulses.clear();
        self.pin_duty.clear();
        self.pin_frequency.clear();
        self.pin13 = false;
        self.segment_memory.clear();
        self.seven_changed_at.clear();
        self.seven_stable.clear();
        self.stepper_phases.clear();
        self.stepper_angles.clear();
        self.servo_angles.clear();
        let eval =
            scene::evaluate_scene(
                &self.parts,
                &self.wires,
                &self.pressed,
                &self.avr_pins,
                &self.pin_duty,
                &self.pin_pulses,
                &self.keypad_keys,
                &self.shift_states,
            &self.pin_frequency,
        );
        self.store_eval(eval);
    }

    fn poll_engine(&mut self) {
        // Drain first, then act: taking an LCD nibble needs to mutate state
        // while the events are being read.
        let mut pending: Vec<SimEvent> = Vec::new();
        if let Some(receiver) = &self.events {
            while let Ok(event) = receiver.try_recv() {
                pending.push(event);
            }
        }
        let mut changed = false;
        for event in pending {
            match event {
                SimEvent::Pin { port, pin, high } => {
                    let key = format!("{}{}", port as char, pin);
                    if port == b'B' && pin == 5 {
                        // Pin 13's LED, as v0.2 ("pin13") lights it.
                        self.pin13 = high;
                    }
                    let previous = self.avr_pins.get(&key).copied();
                    self.avr_pins.insert(key.clone(), high);
                    changed = true;
                    // The enable line has to fall for a nibble to be taken, and
                    // only a real high-to-low counts: the simulator announces
                    // every pin's level at start-up, and taking that lone low as
                    // a pulse puts every later byte half a nibble out of step.
                    if !high && previous == Some(true) {
                        self.latch_lcds(&key);
                    }
                    // A shift register clocks on its clock line and latches on
                    // its latch line, and `shiftOut` makes those edges far
                    // faster than a frame, so they are taken as they arrive.
                    for (part_id, wiring) in &self.shift_wiring {
                        if !wiring.pins.values().any(|pin| pin == &key) {
                            continue;
                        }
                        let state = self.shift_states.get(part_id).copied().unwrap_or_default();
                        let stepped = step_shift_register(state, wiring, &self.avr_pins);
                        self.shift_states.insert(part_id.clone(), stepped);
                    }
                    // A stepper steps on its coil edges, and a sketch writes
                    // dozens of those between two repaints.
                    if self
                        .stepper_wiring
                        .values()
                        .any(|wiring| wiring.pins.iter().any(|pin| pin == &key))
                    {
                        self.advance_steppers();
                    }
                }
                SimEvent::Serial(byte) => {
                    self.serial.push(byte as char);
                    changed = true;
                }
                SimEvent::Timing {
                    port,
                    pin,
                    duty_per_mille,
                    high_micros,
                    period_micros,
                } => {
                    let key = format!("{}{}", port as char, pin);
                    if high_micros > 0 {
                        self.pin_pulses.insert(key.clone(), high_micros as f64);
                        changed = true;
                    }
                    // Without this the netlist reads every pin as fully on, so a
                    // PWM fade never dims anything.
                    if duty_per_mille > 0 {
                        self.pin_duty.insert(
                            key.clone(),
                            (f64::from(duty_per_mille) / 1000.0).clamp(0.0, 1.0),
                        );
                        changed = true;
                    }
                    // A buzzer sounds at this rate, so `tone(pin, 440)` is a
                    // pitch rather than one fixed note.
                    if period_micros > 0 {
                        self.pin_frequency
                            .insert(key, 1_000_000.0 / period_micros as f64);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.evaluate_now();
        }
    }

    /// Solve the circuit as it stands. Called when the simulator reports a
    /// change, and when a property was edited — the netlist reads attributes, so
    /// an edit has to re-solve even though no pin moved.
    fn evaluate_now(&mut self) {
        let eval = scene::evaluate_scene(
            &self.parts,
            &self.wires,
            &self.pressed,
            &self.avr_pins,
            &self.pin_duty,
            &self.pin_pulses,
            &self.keypad_keys,
            &self.shift_states,
            &self.pin_frequency,
        );
        self.store_eval(eval);
    }
}

impl eframe::App for Spike {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        ensure_palette_textures(&ctx, &mut self.textures);
        self.poll_engine();
        self.handle_shortcuts(&ctx);
        if self.auto_run && !self.running {
            self.auto_run = false;
            self.start_sim(&ctx);
        }
        if self.reevaluate {
            self.reevaluate = false;
            self.evaluate_now();
        }
        self.tick_input();
        // A running simulation needs a steady frame: a press is solved on the
        // next frame, and a tap has to survive a few of them. Without this the
        // app only repaints when the simulator happens to change a pin, so a
        // click could be solved at once, late, or only after the tap had
        // expired — which is exactly what made it feel like it sometimes
        // needed a hold or a double click.
        if self.running {
            ctx.request_repaint();
        }
        self.buzzer.update(&self.outputs);
        ensure_textures(
            &ctx,
            &mut self.textures,
            &self.parts,
            &self.led_states,
            &self.outputs,
            &self.led_brightness,
            &self.pressed,
            self.pin13,
        );

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.cancel_wire();
        }

        egui::Panel::top("topbar")
            .exact_size(40.0)
            .show(root, |ui| {
                // Items sit flush, so the menu reads as a bar rather than as a
                // row of separate buttons: v0.2's `.menubar` uses a 2px gap.
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().button_padding = Vec2::new(8.0, 3.0);
                // No box around a menu name: v0.2's `.menubar__btn` is plain
                // text with a transparent border until it is hovered or open.
                let widgets = &mut ui.style_mut().visuals.widgets;
                widgets.inactive.bg_fill = Color32::TRANSPARENT;
                widgets.inactive.bg_stroke = egui::Stroke::NONE;
                widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                widgets.hovered.bg_fill = Color32::from_rgb(0x2a, 0x2a, 0x2e);
                widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER);
                let bar = ui.max_rect();
                ui.painter().hline(
                    bar.x_range(),
                    bar.max.y,
                    egui::Stroke::new(1.0, BORDER),
                );
                // One copy of the text for the whole bar: `t` borrows the
                // `Texts` it reads from, and the menus below want `self` mutably.
                let texts = self.texts.clone();
                ui.horizontal_centered(|ui| {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(texts.t("app.name"))
                            .size(14.0)
                            .family(egui::FontFamily::Name("semibold".into())),
                    );
                    ui.add_space(10.0);
                    ui.menu_button(texts.t("menu.file"), |ui| {
                        if ui.button(texts.t("file.new")).clicked() {
                            self.new_document();
                        }
                        if ui.button(texts.t("file.open")).clicked() {
                            self.open_dialog(&ctx);
                        }
                        if ui.button(texts.t("file.save")).clicked() {
                            self.save();
                        }
                        if ui.button(texts.t("file.save_as")).clicked() {
                            self.save_as();
                        }
                        // Nested the way the old app nests them: Presets → a group
                        // → the circuits in it. The disclosure-with-tabs version
                        // needed a click to open it and a second to pick a group,
                        // and could only ever show one group at a time.
                        let mut choice = None;
                        ui.menu_button(texts.t("presets.heading"), |ui| {
                            for group in scene::PresetGroup::ALL {
                                ui.menu_button(texts.t(group.label_key()), |ui| {
                                    for preset in self
                                        .presets
                                        .iter()
                                        .filter(|preset| preset.group == group)
                                    {
                                        if ui.button(&preset.title).clicked() {
                                            choice = Some(preset.path.clone());
                                        }
                                    }
                                });
                            }
                        });
                        if let Some(path) = choice {
                            self.open_path(path, &ctx);
                        }
                        ui.separator();
                        if ui.button(texts.t("file.quit")).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button(texts.t("menu.edit"), |ui| {
                        if ui.button(texts.t("edit.clear_sketch")).clicked() {
                            self.sketch.clear();
                        }
                    });
                    ui.menu_button(texts.t("menu.view"), |ui| {
                        if ui.button(texts.t("view.reset")).clicked() {
                            self.camera = Camera::default();
                        }
                        if ui.button(texts.t("edit.clear_wires")).clicked() {
                            self.wires.clear();
                            self.wire_start = None;
                        }
                        ui.separator();
                        ui.checkbox(&mut self.sketch_open, texts.t("panel.sketch"));
                        ui.checkbox(&mut self.components_open, texts.t("panel.components"));
                        ui.checkbox(&mut self.serial_open, texts.t("panel.serial"));
                        ui.separator();
                        // Each language names itself: someone who cannot read the
                        // current one still has to be able to find their way out.
                        ui.menu_button(texts.t("menu.language"), |ui| {
                            for locale in locales::Locale::ALL {
                                let active = self.texts.locale() == locale;
                                if ui.selectable_label(active, locale.name()).clicked() {
                                    self.texts.set_locale(locale);
                                    locales::save_locale(locale);
                                }
                            }
                        });
                    });
                    ui.menu_button(texts.t("menu.run"), |ui| {
                        if self.running {
                            if ui.button(texts.t("run.stop")).clicked() {
                                self.stop_sim();
                            }
                        } else if ui.button(texts.t("run.start")).clicked() {
                            self.start_sim(&ctx);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(if self.running {
                            texts.t("run.running")
                        } else {
                            texts.t("run.stopped")
                        });
                        if let Some(error) = &self.file_error {
                            ui.colored_label(Color32::from_rgb(0xe5, 0x48, 0x4d), error);
                        }
                        if let Some(error) = &self.run_error {
                            // Right to left, so the detail is added first and
                            // reads to the right of the status word.
                            match self.diagnostics.first() {
                                Some(problem) => ui.label(format!(
                                    "line {}: {}",
                                    problem.line, problem.message
                                )),
                                None => ui.label(error.lines().next().unwrap_or("")),
                            };
                            ui.colored_label(ERROR_TEXT, texts.t("run.failed"));
                        }
                    });
                });
            });

        // v0.2's default width for this panel is 240, and its edge drags.
                if self.components_open {
    egui::Panel::right("components")
                .resizable(true)
                .default_size(240.0)
                .min_size(200.0)
                .max_size(560.0)
                .show(root, |ui| {
                    let texts = self.texts.clone();
                    panel_head(ui, texts.t("panel.components"), |ui| {
                        let mut open = true;
                        if panel_toggle(ui, &mut open, &texts, Dock::Right) && !open {
                            self.components_open = false;
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("component-list")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            // v0.2's palette: two columns of picture tiles, the
                            // name under each, not a column of text buttons. A
                            // wider panel makes the tiles larger, which is what
                            // resizing a palette should do — the grid keeps two
                            // columns rather than turning into more, smaller
                            // ones.
                            const COLUMNS: usize = 2;
                            const GAP: f32 = 6.0;
                            let tile_width = ((ui.available_width() - GAP) / COLUMNS as f32)
                                .clamp(60.0, 400.0);
                            // v0.2's tile is 104×84; keep that shape as it grows.
                            let tile_size = Vec2::new(tile_width, (tile_width * 0.81).max(58.0));
                            egui::Grid::new("palette-grid")
                                .num_columns(COLUMNS)
                                .min_col_width(tile_width)
                                .max_col_width(tile_width)
                                .spacing(Vec2::new(GAP, GAP))
                                .show(ui, |ui| {
                                    for (index, def) in parts().into_iter().enumerate() {
                                        let key =
                                            format!("palette:{}", def.kind.as_str());
                                        let tile = palette_tile(
                                            ui,
                                            &self.textures,
                                            &key,
                                            self.texts.t(&def.label_key),
                                            !self.running,
                                            tile_size,
                                        );
                                        if tile.drag_started() && !self.running {
                                            self.palette_drag = Some(def.kind);
                                            // Build the part and warm its canvas
                                            // texture now: rasterising an SVG costs
                                            // about 40 ms, and paying that on the
                                            // drop is what made dragging feel slow.
                                            let warm = format!(
                                                "{}-{}",
                                                def.kind.as_str(),
                                                self.next_id
                                            );
                                            if let Some(part) = scene::part_from_kind(
                                                def.kind,
                                                Pos2::ZERO,
                                                warm,
                                            ) {
                                                ensure_textures(
                                                    &ctx,
                                                    &mut self.textures,
                                                    std::slice::from_ref(&part),
                                                    &HashMap::new(),
                                                    &HashMap::new(),
                                                    &HashMap::new(),
                                                    &self.pressed,
                                                    self.pin13,
                                                );
                                            }
                                        }
                                        if tile.clicked() && !self.running {
                                            let id = format!(
                                                "{}-{}",
                                                def.kind.as_str(),
                                                self.next_id
                                            );
                                            // Drop it where the view is looking, stepped
                                            // a little so two adds do not land on each
                                            // other.
                                            let step = 12.0 * (self.next_id % 5) as f32;
                                            let at = Pos2::new(
                                                -self.camera.pan.x / self.camera.zoom + step,
                                                -self.camera.pan.y / self.camera.zoom + step,
                                            );
                                            if let Some(part) =
                                                scene::part_from_kind(def.kind, at, id)
                                            {
                                                self.parts.push(part);
                                                self.next_id += 1;
                                            }
                                        }
                                        if (index + 1) % COLUMNS == 0 {
                                            ui.end_row();
                                        }
                                    }
                                });
                        });
                });
} else {
            egui::Panel::right("components-rail")
                .exact_size(30.0)
                .show(root, |ui| {
                    let texts = self.texts.clone();
                    if icon_button(ui, Icon::Chevron(Dock::Right.opening()), texts.t("panel.components.show")) {
                        self.components_open = true;
                    }
                });
        }

                if self.sketch_open {
    egui::Panel::left("editor")
                .resizable(true)
                .default_size(320.0)
                .min_size(200.0)
                .max_size(560.0)
                .show(root, |ui| {
                    let texts = self.texts.clone();
                    panel_head(ui, texts.t("panel.sketch"), |ui| {
                        let mut open = true;
                        if panel_toggle(ui, &mut open, &texts, Dock::Left) && !open {
                            self.sketch_open = false;
                        }
                    });
                    let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
                        let mut job = editor::highlight(text.as_str());
                        job.wrap.max_width = wrap_width;
                        ui.fonts_mut(|fonts| fonts.layout_job(job))
                    };
                    // The editor is the panel: no well, no border, no inset, and
                    // it fills the height it is given.
                    let widgets = &mut ui.style_mut().visuals.widgets;
                    widgets.inactive.bg_stroke = egui::Stroke::NONE;
                    widgets.hovered.bg_stroke = egui::Stroke::NONE;
                    widgets.active.bg_stroke = egui::Stroke::NONE;
                    egui::ScrollArea::vertical()
                        .id_salt("sketch-editor")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let editor = ui.add_sized(
                                ui.available_size(),
                                egui::TextEdit::multiline(&mut self.sketch)
                                    .id(sketch_editor_id())
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .layouter(&mut layouter),
                            );
                            // A click on a problem in the serial panel asks for
                            // the caret here. The state is written after the
                            // widget has run, so the next frame both shows the
                            // caret and scrolls it into view.
                            if let Some((line, column)) = self.pending_jump.take() {
                                // Written after the widget has run, so the next
                                // frame shows the caret and scrolls it into view:
                                // TextEdit scrolls when the range changes.
                                if place_caret(&ctx, &self.sketch, line, column) {
                                    editor.request_focus();
                                    ctx.request_repaint();
                                }
                            }
                        });
                });
} else {
            egui::Panel::left("editor-rail")
                .exact_size(30.0)
                .show(root, |ui| {
                    let texts = self.texts.clone();
                    if icon_button(ui, Icon::Chevron(Dock::Left.opening()), texts.t("panel.sketch.show")) {
                        self.sketch_open = true;
                    }
                });
        }

        egui::CentralPanel::default().show(root, |ui| {
            let texts = self.texts.clone();
            panel_head(ui, texts.t("panel.circuit"), |ui| {
                // Right to left, as v0.2's circuit bar reads: the run button at
                // the far edge, then zoom, then the view controls.
                if self.running {
                    if ui.button(texts.t("run.stop")).clicked() {
                        self.stop_sim();
                    }
                } else {
                    let run = egui::Button::new(
                        egui::RichText::new(texts.t("run.start"))
                            .color(Color32::from_rgb(0x1a, 0x1a, 0x1c)),
                    )
                    .fill(Color32::from_rgb(0xd9, 0xa4, 0x41));
                    if ui.add(run).clicked() {
                        self.start_sim(&ctx);
                    }
                }
                ui.add_space(6.0);
                if ui
                    .add(egui::Button::new("+").frame(false))
                    .on_hover_text(texts.t("view.zoom_in"))
                    .clicked()
                {
                    self.camera.zoom = (self.camera.zoom * 1.2).min(8.0);
                }
                ui.label(format!("{:.0}%", self.camera.zoom * 100.0));
                if ui
                    .add(egui::Button::new("-").frame(false))
                    .on_hover_text(texts.t("view.zoom_out"))
                    .clicked()
                {
                    self.camera.zoom = (self.camera.zoom / 1.2).max(0.1);
                }
                ui.add_space(6.0);
                if icon_button(ui, Icon::Fit, texts.t("view.fit")) {
                    self.fit_requested = true;
                }
                if icon_button(ui, Icon::Serial, texts.t("panel.serial.toggle")) {
                    self.serial_open = !self.serial_open;
                    // Turning it back on shows the log, not a folded head: the
                    // button is the way in, so it should open all the way.
                    if self.serial_open {
                        self.serial_collapsed = false;
                    }
                }
            });
            // The properties strip, docked under the circuit bar. Always present,
            // so the canvas does not jump when the selection changes — v0.2's
            // `.props`, and its comment says exactly that.
            // One row, fixed height. The strip used to stack its fields and its
            // warning, so selecting a part with more of either grew it and
            // pushed the canvas down — under the pointer, while wiring.
            const STRIP_HEIGHT: f32 = 36.0;
            let strip = egui::Frame::new()
                .fill(SURFACE)
                .inner_margin(egui::Margin::symmetric(12, 0))
                .show(ui, |ui| {
                    ui.set_height(STRIP_HEIGHT);
                    egui::ScrollArea::horizontal()
                        .id_salt("properties-strip")
                        // No scrollbar: it would add its own height to the strip
                        // and the canvas would move after all.
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysHidden,
                        )
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| match self.selection.only_part() {
                                Some(index) if index < self.parts.len() => {
                                    let id = self.parts[index].id.clone();
                                    let warning = self.part_warnings.get(&id).cloned();
                                    let part = &mut self.parts[index];
                                    let changed = properties::row(ui, part, &self.texts);
                                    if let Some(warning) = warning {
                                        ui.separator();
                                        ui.colored_label(
                                            Color32::from_rgb(0xd9, 0xa4, 0x41),
                                            warning,
                                        );
                                    }
                                    changed
                                }
                                _ => {
                                    ui.weak(texts.t("properties.empty"));
                                    false
                                }
                            })
                            .inner
                        })
                        .inner
                });
            ui.painter().hline(
                strip.response.rect.x_range(),
                strip.response.rect.max.y,
                egui::Stroke::new(1.0, BORDER),
            );
            if strip.inner {
                self.reevaluate = true;
            }
            if self.fit_requested {
                self.camera = fit_camera(ui.available_rect_before_wrap(), &self.parts);
                self.fit_requested = false;
            }
            // The serial monitor is docked under the canvas, inside the circuit
            // panel, as in v0.2 — not across the whole window. Collapsed it keeps
            // its head, so the chevron that folded it is still there to unfold it.
            let serial_height = serial_height(self.serial_open, self.serial_collapsed);
            let canvas_height = (ui.available_height() - serial_height).max(80.0);
            let (response, painter) = ui.allocate_painter(
                Vec2::new(ui.available_width(), canvas_height),
                Sense::click_and_drag(),
            );
            let rect = response.rect;

            painter.rect_filled(rect, 0.0, Color32::from_rgb(26, 26, 28));
            draw_grid(&painter, rect, self.camera);

            // The context menu belongs to the right button, which also pans. egui
            // tells a click from a drag, so a right button that comes up where it
            // went down opens the menu, and one that travelled pans — the two
            // never both fire.
            let menu_texts = self.texts.clone();
            response.context_menu(|ui| {
                let some_parts = !self.selection.parts.is_empty();
                if ui
                    .add_enabled(some_parts, egui::Button::new(menu_texts.t("edit.copy")))
                    .clicked()
                {
                    self.copy_selection();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !self.clipboard.parts.is_empty(),
                        egui::Button::new(menu_texts.t("edit.paste")),
                    )
                    .clicked()
                {
                    self.paste();
                    ui.close();
                }
                if ui
                    .add_enabled(
                        !self.selection.is_empty(),
                        egui::Button::new(menu_texts.t("edit.delete")),
                    )
                    .clicked()
                {
                    self.delete_selection();
                    ui.close();
                }
                ui.separator();
                if ui.button(menu_texts.t("edit.clear_wires")).clicked() {
                    self.wires.clear();
                    self.wire_start = None;
                    ui.close();
                }
            });

            // A part dragged out of the palette follows the pointer and lands
            // where it is let go.
            if let Some(kind) = self.palette_drag {
                let at = ctx.input(|i| i.pointer.interact_pos());
                let released = ctx.input(|i| i.pointer.primary_released());
                if let Some(at) = at {
                    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                    if let Some(texture) = self
                        .textures
                        .get(&format!("palette:{}", kind.as_str()))
                    {
                        let size = texture.size_vec2();
                        let scale = 44.0 / size.y.max(1.0);
                        let drawn = size * scale;
                        let ghost = Rect::from_center_size(at, drawn);
                        painter.image(
                            texture.id(),
                            ghost,
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            Color32::from_rgba_unmultiplied(255, 255, 255, 190),
                        );
                    }
                }
                if released {
                    if let Some(at) = at.filter(|at| rect.contains(*at)) {
                        let world = self.camera.screen_to_world(rect, at);
                        let id = format!("{}-{}", kind.as_str(), self.next_id);
                        if let Some(part) = scene::part_from_kind(kind, world, id) {
                            self.parts.push(part);
                            self.next_id += 1;
                            self.reevaluate = true;
                        }
                    }
                    self.palette_drag = None;
                }
            }

            let pointer = response.hover_pos();
            let world_pointer = pointer.map(|p| self.camera.screen_to_world(rect, p));
            // The highlight follows the same order a click does: a pin under a
            // wire, or under another part, is not what the pointer is on.
            self.hovered_pin = pointer.and_then(|p| {
                match pick_at(
                    &self.parts,
                    &self.wires,
                    self.camera,
                    rect,
                    p,
                    PIN_HIT_RADIUS / self.camera.zoom,
                ) {
                    Pick::Pin(pin) => Some(pin),
                    _ => None,
                }
            });

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

            // A press picks up whatever control is under the pointer. This is
            // the pointer's own press, not a drag start: egui only reports a
            // drag once the pointer has moved, and a button that only reacts
            // after the pointer has travelled somewhere is a button that does
            // not work on a click.
            let pressed_now = ctx.input(|input| input.pointer.primary_pressed());
            let released_now = ctx.input(|input| input.pointer.primary_released());
            let shift = ctx.input(|input| input.modifiers.shift);
            if pressed_now {
                if let Some(pos) = ctx.input(|input| input.pointer.interact_pos()) {
                    if rect.contains(pos) {
                        let world = self.camera.screen_to_world(rect, pos);
                        let pick = pick_at(
                            &self.parts,
                            &self.wires,
                            self.camera,
                            rect,
                            pos,
                            PIN_HIT_RADIUS / self.camera.zoom,
                        );
                        if let Pick::Pin(pin) = pick {
                            // Dragging from a pin draws a wire. Before this, the
                            // press grabbed the part and dragged the Arduino out
                            // from under itself.
                            self.wire_press(pin);
                        } else if self.wire_start.is_some() {
                            // Mid-wire, the click branch below pins a bend.
                        } else if let Pick::Wire(index) = pick {
                            // A wire crossing a piece is what the pointer is on:
                            // wires are painted after the parts. Testing the part
                            // first made a wire lying over a part impossible to
                            // pick.
                            if shift {
                                self.selection.toggle_wire(index);
                            } else {
                                self.selection.select_wire(index);
                            }
                        } else if let Pick::Part(index) = pick {
                            // Shift adds to or takes from the selection; a plain
                            // press on something already selected keeps the rest,
                            // so a multi-part drag does not collapse to one part.
                            if shift {
                                self.selection.toggle_part(index);
                            } else if !self.selection.has_part(index) {
                                self.selection.select_part(index);
                            }
                            self.press_control(index, world, pos);
                        } else {
                            if !shift {
                                self.selection.clear();
                            }
                            // Nothing under the pointer: the left button draws a
                            // selection box. Panning is the right and middle
                            // buttons' job, so a left drag never moves the canvas
                            // out from under what it is selecting.
                            self.marquee = Some((pos, pos));
                        }
                    }
                }
            }
            // The selection box follows the pointer while the left button is down
            // and is applied when it comes up. Panning belongs to the right and
            // middle buttons — one gesture, one job, so a right drag can no
            // longer scroll the canvas and select at the same time.
            if let Some((start, _)) = self.marquee {
                if let Some(now) = pointer {
                    self.marquee = Some((start, now));
                }
            }
            if released_now {
                if let Some((start, end)) = self.marquee.take() {
                    let area = Rect::from_two_pos(
                        self.camera.screen_to_world(rect, start),
                        self.camera.screen_to_world(rect, end),
                    );
                    self.select_in(area, shift);
                }
            }
            if response.dragged_by(PointerButton::Primary) {
                if let Some((index, from)) = self.press_drag {
                    // Four pixels of travel is a drag; less than that is a click
                    // whose hand wobbled.
                    if pointer.is_some_and(|now| now.distance(from) > 4.0) {
                        // Dragging one part of a selection drags all of it, the
                        // way v0.2 moves the whole pick.
                        if !self.selection.has_part(index) {
                            self.selection.select_part(index);
                        }
                        self.dragging = Some(index);
                        self.press_drag = None;
                    }
                }
                if let Some(index) = self.joystick {
                    if !self.joystick_moved {
                        let moved = self
                            .joystick_start
                            .zip(pointer)
                            .is_some_and(|(start, now)| now.distance(start) > 4.0);
                        if moved {
                            // A moved stick is not a click on its button.
                            self.joystick_moved = true;
                            let id = self.parts[index].id.clone();
                            self.pressed.insert(id, false);
                        }
                    }
                    if self.joystick_moved {
                        if let Some(world) = world_pointer {
                            let (x, y) = joystick_axes(&self.parts[index], world);
                            self.reevaluate = true;
                            self.parts[index]
                                .attrs
                                .insert("x".into(), x.to_string());
                            self.parts[index]
                                .attrs
                                .insert("y".into(), y.to_string());
                        }
                    }
                } else if let Some(index) = self.knob {
                    if let Some(world) = world_pointer {
                        self.drag_knob(index, world);
                    }
                } else if self.dragging.is_some() {
                    // The pick is captured once, on the first frame of the drag:
                    // it is the set that moves and the set a pin may snap from.
                    // A breadboard brings what is plugged into it.
                    if self.drag_origin.is_empty() {
                        let moving = seated_with(&self.parts, &self.selection.parts);
                        self.drag_origin = moving
                            .iter()
                            .filter_map(|&index| {
                                self.parts.get(index).map(|part| (index, part.center))
                            })
                            .collect();
                        self.drag_travel = Vec2::ZERO;
                    }
                    self.drag_travel += response.drag_delta() / self.camera.zoom;
                    // Where the pick goes is a function of where the drag started
                    // and how far the pointer has travelled — never of where the
                    // pick already is, which is what let a snap re-apply itself
                    // every frame and pin the part to the hole.
                    let placed = drag_placement(
                        &self.parts,
                        &self.drag_origin,
                        self.drag_travel,
                        SNAP_RADIUS / self.camera.zoom,
                    );
                    for (index, at) in placed {
                        if let Some(part) = self.parts.get_mut(index) {
                            part.center = at;
                        }
                    }
                }
            }
            // The canvas is a grab surface: an open hand over empty space, a
            // closed one while a pan is actually in progress. Panning is the
            // right and middle buttons now, so the closed hand follows those.
            let over_empty = world_pointer.is_some_and(|p| {
                self.hovered_pin.is_none() && hit_part(&self.parts, p).is_none()
            });
            if response.dragged_by(PointerButton::Middle)
                || response.dragged_by(PointerButton::Secondary)
            {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if over_empty {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }

            if released_now || response.drag_stopped() {
                self.wire_release(self.hovered_pin);
                self.dragging = None;
                self.press_drag = None;
                // The next drag captures its own origins; a stale set would place
                // the next pick where the last one started.
                self.drag_origin.clear();
                self.drag_travel = Vec2::ZERO;
                self.release_input();
            }

            if response.clicked_by(PointerButton::Primary) && self.hovered_pin.is_none() {
                if let Some(world) = world_pointer {
                    if self.wire_start.is_some() {
                        // Mid-wire, a click off a pin pins a bend where it
                        // landed, which is how a wire is routed around things.
                        self.wire_bend(world);
                    } else if let Some(pos) = pointer {
                        // The same order as the press, so a click and a press can
                        // never disagree about what is under the pointer.
                        match pick_at(
                            &self.parts,
                            &self.wires,
                            self.camera,
                            rect,
                            pos,
                            PIN_HIT_RADIUS / self.camera.zoom,
                        ) {
                            Pick::Wire(index) => self.selection.select_wire(index),
                            Pick::Part(index) => {
                                if !self.selection.has_part(index) {
                                    self.selection.select_part(index);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            if self.serial_open {
                let texts = self.texts.clone();
                panel_head(ui, texts.t("panel.serial"), |ui| {
                    if ui.button(texts.t("edit.clear")).clicked() {
                        self.serial.clear();
                    }
                    let mut expanded = !self.serial_collapsed;
                    if panel_toggle(ui, &mut expanded, &texts, Dock::Bottom) {
                        // Fold it down to its head rather than dismissing it. The
                        // chevron stays on screen, so the same control brings it
                        // back and there is nothing to hunt for in the View menu.
                        self.serial_collapsed = !expanded;
                    }
                });
                // The compiler's problems, each one clickable: v0.2 puts the
                // caret on the line a clicked error names, so this does too.
                if !self.serial_collapsed && !self.diagnostics.is_empty() {
                    let mut jump = None;
                    egui::ScrollArea::vertical()
                        .id_salt("compile-problems")
                        .max_height(96.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for problem in &self.diagnostics {
                                let colour = match problem.severity {
                                    editor::Severity::Error => ERROR_TEXT,
                                    editor::Severity::Warning => WARNING_TEXT,
                                };
                                let text = format!(
                                    "{}:{}  {}",
                                    problem.line, problem.column, problem.message
                                );
                                let row = egui::RichText::new(text).monospace().color(colour);
                                if problem.in_sketch() {
                                    let clicked = ui
                                        .add(egui::Label::new(row).sense(Sense::click()))
                                        .on_hover_text(texts.t("editor.jump"))
                                        .clicked();
                                    if clicked {
                                        jump = Some((problem.line, problem.column));
                                    }
                                } else {
                                    // A library's line is not in this editor, so
                                    // the row says what it is and does nothing.
                                    ui.label(row);
                                }
                            }
                        });
                    if let Some((line, column)) = jump {
                        self.pending_jump = Some((line, column));
                        self.sketch_open = true;
                        ctx.request_repaint();
                    }
                    ui.separator();
                }
                if !self.serial_collapsed {
                    egui::ScrollArea::vertical()
                        .id_salt("serial-output")
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            ui.monospace(&self.serial);
                            if let Some(error) = &self.run_error {
                                ui.colored_label(Color32::from_rgb(230, 120, 100), error);
                            }
                        });
                }
            }

            // A grab can change a part's art (a knob, a slider). Refresh the
            // textures once more after the input, so the frame that draws it
            // does not fall back to a plain rectangle for one frame.
            ensure_textures(
                &ctx,
                &mut self.textures,
                &self.parts,
                &self.led_states,
                &self.outputs,
                &self.led_brightness,
                &self.pressed,
                self.pin13,
            );
            for (index, part) in self.parts.iter().enumerate() {
                draw_part(
                    &painter,
                    rect,
                    self.camera,
                    part,
                    self.selection.has_part(index),
                    self.hovered_pin
                        .filter(|(part_index, _)| *part_index == index),
                    self.textures.get(&texture_key(
                        part,
                        self.led_states
                            .get(&part.id)
                            .is_some_and(|state| !matches!(state, LedState::Off)),
                        &self.outputs,
                        &self.led_brightness,
                        &self.pressed,
                        self.pin13,
                    )),
                    self.rgb_leds.get(&part.id).copied(),
                    // Segments are a display's business. The LED bar carries ten
                    // values of its own, and handing those to draw_segments drew
                    // a seven-segment digit on top of it.
                    match part.kind.as_str() {
                        "seven-segment" | "seven-segment-4" => {
                            self.outputs.get(&part.id).map(|out| out.values.as_slice())
                        }
                        _ => None,
                    },
                );
                if shows_active(part, &self.outputs, &self.sensors) {
                    draw_active_dot(&painter, rect, self.camera, part);
                }
                if part.kind == "analog-joystick" {
                    draw_joystick_knob(&painter, rect, self.camera, part);
                }
                if part.kind == "potentiometer" {
                    draw_pot_tick(&painter, rect, self.camera, part);
                }
                if part.kind == "slide-potentiometer" {
                    draw_slide_tip(&painter, rect, self.camera, part);
                }
                if part.kind == "stepper-motor" {
                    let degrees = self
                        .outputs
                        .get(&part.id)
                        .and_then(|out| out.angle)
                        .unwrap_or(0.0) as f32;
                    draw_stepper_hub(&painter, rect, self.camera, part, degrees);
                }
                if let Some((_, button)) = self.remote_hold.filter(|(held, _)| *held == index) {
                    if let Some(centre) = remote::button_centre(part, button) {
                        let at = self.camera.world_to_screen(rect, centre);
                        // The button radius is in the element's pixels; scale it
                        // the same way the part itself is scaled.
                        let unit = self.camera.zoom * part.size.x / remote::VIEW.x;
                        painter.circle_stroke(
                            at,
                            17.6 * unit,
                            Stroke::new(2.0, Color32::from_rgb(217, 164, 65)),
                        );
                    }
                }
                if part.kind == "membrane-keypad" {
                    // The element shows nothing for a tap, so a held key gets a
                    // mark of its own — otherwise a click is invisible.
                    if let Some(Some(key)) = self.keypad_keys.get(&part.id) {
                        if let Some(area) = keypad_key_world(part, *key) {
                            let min = self.camera.world_to_screen(rect, area.min);
                            let max = self.camera.world_to_screen(rect, area.max);
                            Turned::new(
                                &painter,
                                part,
                                self.camera.world_to_screen(rect, part.center),
                            )
                            .rect_filled(
                                Rect::from_min_max(min, max),
                                2.0,
                                Color32::from_rgba_unmultiplied(217, 164, 65, 110),
                            );
                        }
                    }
                }
                if part.kind == "lcd1602" {
                    if let Some(state) = self.lcd_states.get(&part.id) {                        draw_lcd_text(
                            &painter,
                            rect,
                            self.camera,
                            part,
                            state,
                            ctx.input(|input| input.time),
                        );
                    }
                }
            }

            for (index, wire) in self.wires.iter().enumerate() {
                let a = self
                    .camera
                    .world_to_screen(rect, self.parts[wire.a.0].pin_world(wire.a.1));
                let b = self
                    .camera
                    .world_to_screen(rect, self.parts[wire.b.0].pin_world(wire.b.1));
                let mut route = vec![CorePoint::new(a.x as f64, a.y as f64)];
                route.extend(wire.waypoints.iter().map(|point| {
                    // Waypoints are world coordinates, like the pins either side of
                    // them. Drawing them raw put the wire near the top-left corner
                    // — a wire with a bend in it appeared to shoot off the canvas.
                    let at = self.camera.world_to_screen(rect, *point);
                    CorePoint::new(at.x as f64, at.y as f64)
                }));
                route.push(CorePoint::new(b.x as f64, b.y as f64));
                let selected = self.selection.has_wire(index);
                let colour = if selected {
                    Color32::from_rgb(217, 164, 65)
                } else {
                    Color32::from_rgb(108, 203, 123)
                };
                let thickness = if selected { 3.5 } else { 2.5 };
                let line = expand_path(&route);
                for pair in line.windows(2) {
                    painter.line_segment(
                        [
                            Pos2::new(pair[0].x as f32, pair[0].y as f32),
                            Pos2::new(pair[1].x as f32, pair[1].y as f32),
                        ],
                        Stroke::new(thickness, colour),
                    );
                }
            }

            // The marquee is drawn last, over the circuit it is selecting.
            if let Some((start, end)) = self.marquee {
                let band = Rect::from_two_pos(start, end);
                painter.rect_filled(
                    band,
                    0.0,
                    Color32::from_rgba_unmultiplied(217, 164, 65, 26),
                );
                painter.rect_stroke(
                    band,
                    0.0,
                    Stroke::new(1.0, Color32::from_rgb(217, 164, 65)),
                    egui::StrokeKind::Inside,
                );
            }

            if let (Some(start), Some(pointer)) = (self.wire_start, pointer) {
                let pa = self
                    .camera
                    .world_to_screen(rect, self.parts[start.0].pin_world(start.1));
                let mut route = vec![CorePoint::new(pa.x as f64, pa.y as f64)];
                route.extend(self.wire_waypoints.iter().map(|bend| {
                    let at = self.camera.world_to_screen(rect, *bend);
                    CorePoint::new(at.x as f64, at.y as f64)
                }));
                route.push(CorePoint::new(pointer.x as f64, pointer.y as f64));
                for bend in &self.wire_waypoints {
                    let at = self.camera.world_to_screen(rect, *bend);
                    painter.circle_filled(at, 3.0, Color32::from_rgb(217, 164, 65));
                }
                let preview = expand_path(&route);
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

/// A painter for everything drawn on top of a part's sprite: geometry is given
/// in the part's own unrotated frame and mapped through its quarter turn.
///
/// egui has no canvas transform, so a turned part's sprite is a rotated mesh and
/// its overlays go through here. Only quarter turns exist, which keeps a turned
/// rectangle a rectangle.
struct Turned<'a> {
    painter: &'a egui::Painter,
    centre: Pos2,
    radians: f32,
}

impl<'a> Turned<'a> {
    fn new(painter: &'a egui::Painter, part: &Part, centre: Pos2) -> Self {
        Self {
            painter,
            centre,
            radians: part.rotation.to_radians(),
        }
    }

    /// For things a turn cannot change — the centre of a disc, a radial glow.
    fn painter(&self) -> &egui::Painter {
        self.painter
    }

    fn point(&self, point: Pos2) -> Pos2 {
        if self.radians == 0.0 {
            return point;
        }
        let (sin, cos) = self.radians.sin_cos();
        let offset = point - self.centre;
        self.centre
            + Vec2::new(
                offset.x * cos - offset.y * sin,
                offset.x * sin + offset.y * cos,
            )
    }

    fn rect(&self, rect: Rect) -> Rect {
        Rect::from_two_pos(self.point(rect.min), self.point(rect.max))
    }

    fn rect_filled(&self, rect: Rect, rounding: f32, colour: Color32) {
        self.painter.rect_filled(self.rect(rect), rounding, colour);
    }

    fn circle_filled(&self, centre: Pos2, radius: f32, colour: Color32) {
        self.painter.circle_filled(self.point(centre), radius, colour);
    }

    fn circle_stroke(&self, centre: Pos2, radius: f32, stroke: Stroke) {
        self.painter.circle_stroke(self.point(centre), radius, stroke);
    }

    fn convex_polygon(&self, points: Vec<Pos2>, fill: Color32, stroke: Stroke) {
        let points = points.into_iter().map(|point| self.point(point)).collect();
        self.painter
            .add(egui::Shape::convex_polygon(points, fill, stroke));
    }

    fn mesh(&self, mut mesh: egui::Mesh) {
        for vertex in &mut mesh.vertices {
            vertex.pos = self.point(vertex.pos);
        }
        self.painter.add(egui::Shape::mesh(mesh));
    }

    fn image(&self, texture: &egui::TextureHandle, rect: Rect, uv: Rect, tint: Color32) {
        if self.radians == 0.0 {
            self.painter.image(texture.id(), rect, uv, tint);
            return;
        }
        let mut mesh = egui::Mesh::with_texture(texture.id());
        let corners = [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ];
        let uvs = [
            uv.left_top(),
            uv.right_top(),
            uv.right_bottom(),
            uv.left_bottom(),
        ];
        for (corner, uv) in corners.into_iter().zip(uvs) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos: self.point(corner),
                uv,
                color: tint,
            });
        }
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        self.painter.add(egui::Shape::mesh(mesh));
    }

    fn text(&self, pos: Pos2, anchor: Align2, text: &str, font: FontId, colour: Color32) {
        if self.radians == 0.0 {
            self.painter.text(pos, anchor, text, font, colour);
            return;
        }
        let galley = self.painter.layout_no_wrap(text.to_owned(), font, colour);
        let at = anchor.anchor_size(self.point(pos), galley.size());
        let mut shape = egui::epaint::TextShape::new(at.min, galley, colour);
        shape.angle = self.radians;
        self.painter.add(shape);
    }
}

fn draw_segments(painter: &Turned, screen: Rect, part: &Part, values: &[u8]) {
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
            painter.convex_polygon(points, color, egui::Stroke::NONE);
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
fn draw_rgb_glow(paint: &Turned, screen: Rect, state: RgbState) {
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
        soft_radial(paint.painter(), paint.point(at(glow.x, glow.y)), &bands);
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

/// The potentiometer's tick, at the angle its value asks for.
///
/// This is the art's own `<rect id="rotating">`, drawn here so the texture can
/// stay one image: the element turns it about (10, 8) with CSS, and resvg does
/// not resolve the custom property that drives it.
fn draw_pot_tick(painter: &egui::Painter, rect: Rect, camera: Camera, part: &Part) {
    const VIEW_WIDTH: f32 = 20.0;
    let bounds = part.rect();
    let screen = Rect::from_min_max(
        camera.world_to_screen(rect, bounds.min),
        camera.world_to_screen(rect, bounds.max),
    );
    let unit = screen.width() / VIEW_WIDTH;
    let corners = knobs::pot_tick(attr_number(part, "value", 0.0));
    Turned::new(painter, part, screen.center()).convex_polygon(
        corners
            .iter()
            .map(|corner| screen.min + Vec2::new(corner.0, corner.1) * unit)
            .collect(),
        Color32::from_rgb(0x1a, 0x1a, 0x1a),
        Stroke::NONE,
    );
}

/// The stepper's hub, turned to the rotor's angle.
///
/// Same reason as the potentiometer's tick: the element rewrites the hub's own
/// `transform` for every angle, and baking that into the drawing meant rasterising
/// the whole motor — gradients, filters and all — again on every frame it turned.
/// The hub is a shape, so it can simply be drawn.
fn draw_stepper_hub(painter: &egui::Painter, rect: Rect, camera: Camera, part: &Part, degrees: f32) {
    // The element's viewBox, which is the frame its geometry is stated in.
    const VIEW: Vec2 = Vec2::new(220.374, 235.494);
    let bounds = part.rect();
    let screen = Rect::from_min_max(
        camera.world_to_screen(rect, bounds.min),
        camera.world_to_screen(rect, bounds.max),
    );
    let scale = Vec2::new(screen.width() / VIEW.x, screen.height() / VIEW.y);
    let outline = scene::STEPPER_ROTOR
        .hub_outline(degrees)
        .into_iter()
        .map(|(x, y)| screen.min + Vec2::new(x * scale.x, y * scale.y))
        .collect();
    // The hub's fill, from the element's own `#4d4d4d`.
    Turned::new(painter, part, screen.center()).convex_polygon(
        outline,
        Color32::from_rgb(0x4d, 0x4d, 0x4d),
        Stroke::NONE,
    );
}

/// The slide pot's tip, at the position its value asks for.
///
/// Same reason as the tick: the handle is a gradient inside `<g id="tip">`, and
/// baking it per value costs ~36 ms a state.
fn draw_slide_tip(painter: &egui::Painter, rect: Rect, camera: Camera, part: &Part) {
    const VIEW_WIDTH: f32 = 55.0;
    let bounds = part.rect();
    let screen = Rect::from_min_max(
        camera.world_to_screen(rect, bounds.min),
        camera.world_to_screen(rect, bounds.max),
    );
    let unit = screen.width() / VIEW_WIDTH;
    let (origin_x, origin_y) = knobs::slide_tip_origin(attr_number(part, "value", 0.0));
    let at =
        |x: f32, y: f32| screen.min + Vec2::new(origin_x + x, origin_y + y) * unit;
    let quad = |left: f32, top: f32, right: f32, bottom: f32| {
        Rect::from_min_max(at(left, top), at(right, bottom))
    };

    // The handle: dark at both edges, lighter through the middle — the element's
    // `tipGradient`, which runs along x.
    let dark = Color32::from_rgb(0x1a, 0x1a, 0x1a);
    let light = Color32::from_rgb(0x59, 0x59, 0x59);
    let left = at(16.5, 0.0).x;
    let right = at(28.5, 0.0).x;
    let top = at(16.5, 0.0).y;
    let bottom = at(16.5, 19.0).y;
    let mut mesh = egui::Mesh::default();
    for (index, colour) in [dark, light, dark].into_iter().enumerate() {
        let x = egui::lerp(left..=right, index as f32 / 2.0);
        let first = (index * 2) as u32;
        mesh.colored_vertex(egui::pos2(x, top), colour);
        mesh.colored_vertex(egui::pos2(x, bottom), colour);
        debug_assert_eq!(first, mesh.vertices.len() as u32 - 2);
    }
    mesh.add_triangle(0, 1, 3);
    mesh.add_triangle(0, 3, 2);
    mesh.add_triangle(2, 3, 5);
    mesh.add_triangle(2, 5, 4);
    let paint = Turned::new(painter, part, screen.center());
    paint.mesh(mesh);

    // The light strip down its right side, and the dark band across it.
    paint.rect_filled(
        quad(22.2, 0.0, 22.8, 19.0),
        0.0,
        Color32::from_rgb(0xef, 0xef, 0xef),
    );
    paint.rect_filled(quad(19.75, 8.6, 25.25, 10.4), 0.0, dark);
}

/// The joystick's ball, drawn where its axes say it is.
///
/// The element translates the knob by 2.5 units at full deflection: x grows to
/// the right, y upwards. A texture could not do this without being rebuilt for
/// every step of a drag, so the ball is drawn on top of the base art.
fn draw_joystick_knob(painter: &egui::Painter, rect: Rect, camera: Camera, part: &Part) {
    const VIEW_WIDTH: f32 = 27.2;
    const REST: Vec2 = Vec2::new(13.6, 13.6);
    let bounds = part.rect();
    let screen = Rect::from_min_max(
        camera.world_to_screen(rect, bounds.min),
        camera.world_to_screen(rect, bounds.max),
    );
    let unit = screen.width() / VIEW_WIDTH;
    let axis = |name: &str| {
        part.attrs
            .get(name)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(50.0)
            .clamp(0.0, 100.0)
    };
    let offset = Vec2::new((axis("x") - 50.0) / 50.0, -(axis("y") - 50.0) / 50.0) * 2.5;
    let centre = screen.min + (REST + offset) * unit;
    let radius = 10.6 * unit;
    let paint = Turned::new(painter, part, screen.center());
    paint.circle_filled(centre, radius, Color32::from_rgb(0x22, 0x22, 0x22));
    paint.circle_stroke(
        centre,
        radius,
        Stroke::new(1.2 * unit, Color32::from_rgb(0x77, 0x77, 0x77)),
    );
}

/// A part's numeric attribute, or a default when it has not been set.
fn attr_number(part: &Part, name: &str, fallback: f64) -> f64 {
    part.attrs
        .get(name)
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(fallback)
}

/// The LCD element's drawing, and its character grid inside it: 16x2 cells of
/// 3.55 x 5.95 from (12.45, 12.55), each glyph five by eight dots of 0.55 x
/// 0.65 spaced 0.6 across and 0.7 down.
const LCD_VIEW: Vec2 = Vec2::new(80.0, 36.0);
const LCD_ORIGIN: Vec2 = Vec2::new(12.45, 12.55);
const LCD_CELL: Vec2 = Vec2::new(3.55, 5.95);
const LCD_DOT: Vec2 = Vec2::new(0.6, 0.7);
const LCD_DOT_SIZE: Vec2 = Vec2::new(0.55, 0.65);

/// Draw what the LCD1602's controller has been told, in the element's own dots.
fn draw_lcd_text(
    painter: &egui::Painter,
    rect: Rect,
    camera: Camera,
    part: &Part,
    state: &lcd::LcdState,
    now: f64,
) {
    if !state.display_on {
        return;
    }
    let bounds = part.rect();
    let screen = Rect::from_min_max(
        camera.world_to_screen(rect, bounds.min),
        camera.world_to_screen(rect, bounds.max),
    );
    let paint = Turned::new(painter, part, screen.center());
    let unit = screen.width() / LCD_VIEW.x;
    let at = |x: f32, y: f32| screen.min + Vec2::new(x, y) * unit;
    let dwell = |column: f32, row: f32| LCD_ORIGIN + Vec2::new(column, row) * Vec2::new(LCD_CELL.x, LCD_CELL.y);
    for (index, code) in state.chars.iter().enumerate() {
        let Some(glyph) = lcd_font::glyph(*code) else {
            continue;
        };
        let cell = dwell((index % 16) as f32, (index / 16) as f32);
        for (row, bits) in glyph.iter().enumerate() {
            // Merge a run of lit dots into one rect: a fifth of the shapes.
            let mut column = 0;
            while column < 5 {
                if bits & (1 << column) == 0 {
                    column += 1;
                    continue;
                }
                let mut run = 1;
                while column + run < 5 && bits & (1 << (column + run)) != 0 {
                    run += 1;
                }
                let origin = at(
                    cell.x + column as f32 * LCD_DOT.x,
                    cell.y + row as f32 * LCD_DOT.y,
                );
                let size = Vec2::new(
                    LCD_DOT_SIZE.x + LCD_DOT.x * (run - 1) as f32,
                    LCD_DOT_SIZE.y,
                ) * unit;
                paint.rect_filled(Rect::from_min_size(origin, size), 0.0, Color32::WHITE);
                column += run;
            }
        }
    }
    // Cursor and blink sit in the cell the address counter points at.
    let Some(cell_index) = lcd::lcd_index(state.address) else {
        return;
    };
    let cell = dwell((cell_index % 16) as f32, (cell_index / 16) as f32);
    if state.blink_on && (now % 1.0) < 0.5 {
        let origin = at(cell.x, cell.y);
        let size = Vec2::new(2.95, 5.55) * unit;
        paint.rect_filled(Rect::from_min_size(origin, size), 0.0, Color32::WHITE);
    }
    if state.cursor_on {
        let origin = at(cell.x, cell.y + LCD_DOT.y * 7.0);
        let size = Vec2::new(2.95, 0.65) * unit;
        paint.rect_filled(Rect::from_min_size(origin, size), 0.0, Color32::WHITE);
    }
}

/// The keypad element's key grid, in its own 70.336 x 91 units: each key is an
/// 11.2 x 11 rect whose corner sits at (column, row) in these lists.
const KEYPAD_VIEW: Vec2 = Vec2::new(70.336, 91.0);
const KEYPAD_COLUMNS: [f32; 4] = [7.0, 22.0, 37.0, 52.0];
const KEYPAD_ROWS: [f32; 4] = [10.7, 25.0, 39.3, 53.6];
const KEYPAD_KEY: Vec2 = Vec2::new(11.2, 11.0);

/// The stick's travel as a fraction of its 27.2 x 31.8 drawing, as v0.2 has it.
const JOYSTICK_TRAVEL_X: f32 = 10.6 / 27.2;
const JOYSTICK_TRAVEL_Y: f32 = 10.6 / 31.8;

/// A tap has to outlive one scan of the keypad matrix to be seen at all.
const KEY_HOLD: std::time::Duration = std::time::Duration::from_millis(80);

/// How many frames a tap is held for at least, so a slow frame cannot swallow
/// it before the netlist is solved.
const KEY_HOLD_FRAMES: u32 = 3;

/// The keypad key under a canvas point, if the point is on one.
/// Where a key's face sits, in world coordinates — the inverse of
/// `keypad_key`, used to mark the key a tap landed on.
fn keypad_key_world(part: &Part, key: KeyPosition) -> Option<Rect> {
    let bounds = part.bounds();
    let unit = bounds.width() / KEYPAD_VIEW.x;
    let x = *KEYPAD_COLUMNS.get(key.column)?;
    let y = *KEYPAD_ROWS.get(key.row)?;
    let min = bounds.min + Vec2::new(x, y) * unit;
    Some(Rect::from_min_size(min, KEYPAD_KEY * unit))
}

fn keypad_key(part: &Part, world: Pos2) -> Option<KeyPosition> {
    let bounds = part.bounds();
    let local = (world - bounds.min) / (bounds.width() / KEYPAD_VIEW.x);
    let column = KEYPAD_COLUMNS
        .iter()
        .position(|start| local.x >= *start && local.x <= *start + KEYPAD_KEY.x)?;
    let row = KEYPAD_ROWS
        .iter()
        .position(|start| local.y >= *start && local.y <= *start + KEYPAD_KEY.y)?;
    Some(KeyPosition { row, column })
}

/// The joystick's axes for a canvas point, as the percentages the netlist reads.
fn joystick_axes(part: &Part, world: Pos2) -> (u8, u8) {
    let bounds = part.bounds();
    let local = ((world - bounds.min) / bounds.size()).clamp(Vec2::ZERO, Vec2::splat(1.0));
    let x = 50.0 + ((local.x - 0.5) / JOYSTICK_TRAVEL_X) * 50.0;
    let y = 50.0 - ((local.y - 0.5) / JOYSTICK_TRAVEL_Y) * 50.0;
    (
        x.round().clamp(0.0, 100.0) as u8,
        y.round().clamp(0.0, 100.0) as u8,
    )
}

/// Whether a display should take a new pattern.
///
/// A sketch writes its seven segments one at a time, so one frame can catch a
/// half-written digit. That is what the settle window is for, and it is short
/// on purpose: a frame that arrives later than that is a real change, and a
/// display that sits on an old pattern is worse than one that shows a digit a
/// frame early. The old rule here held every pattern until it had been seen
/// twice, which froze the display on whatever it happened to see first.
fn segment_settled(shown: &[u8], values: &[u8], since_change: Option<Duration>) -> bool {
    if shown == values {
        return false;
    }
    match since_change {
        Some(since) => since >= SEGMENT_SETTLE,
        None => true,
    }
}

/// How soon after one change a second one is treated as half a write.
const SEGMENT_SETTLE: Duration = Duration::from_millis(8);

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
    // The dot sits in the part's own top-left corner, so it turns with it.
    let paint = Turned::new(painter, part, camera.world_to_screen(rect, part.center));
    let centre = camera.world_to_screen(rect, part.rect().min + Vec2::new(4.0, 4.0));
    let radius = 5.0 * camera.zoom;
    paint.circle_filled(centre, radius, Color32::from_rgb(108, 203, 123));
    paint.circle_stroke(
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
    let centre = camera.world_to_screen(rect, part.center);
    let paint = Turned::new(painter, part, centre);
    // The sprite is measured in the part's own frame and turned into place;
    // every overlay below follows the same turn.
    let unrotated = part.rect();
    let screen = Rect::from_min_max(
        camera.world_to_screen(rect, unrotated.min),
        camera.world_to_screen(rect, unrotated.max),
    );
    if let Some(texture) = texture {
        paint.image(
            texture,
            screen,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        paint.rect_filled(screen, 0.0, part.color);
    }
    if let Some(state) = rgb {
        draw_rgb_glow(&paint, screen, state);
    }
    if let Some(values) = segments {
        draw_segments(&paint, screen, part, values);
    }
    if selected {
        // The box the part takes up on screen is the turned one.
        let min = camera.world_to_screen(rect, part.bounds().min);
        let max = camera.world_to_screen(rect, part.bounds().max);
        painter.rect_stroke(
            Rect::from_min_max(min, max),
            0.0,
            Stroke::new(1.5, Color32::from_rgb(217, 164, 65)),
            egui::StrokeKind::Outside,
        );
    }
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
        paint.text(
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
    fn tmp_led_art_follows_brightness() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wokwi/led.svg");
        let svg = std::fs::read_to_string(path).expect("led asset");
        for brightness in [1.0, 0.5, 0.16, 0.0] {
            let patched = scene::led_state_svg(&svg, "red", brightness, brightness > 0.0);
            let image = art::rasterize(&patched, 202, 136).expect("rasterise");
            let energy: u64 = image
                .pixels
                .iter()
                .map(|pixel| u64::from(pixel.r()) + u64::from(pixel.g()) + u64::from(pixel.b()))
                .sum();
            println!("brightness {brightness}: energy {energy}");
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
    fn presets_are_grouped_into_v0_2s_three_tabs() {
        use scene::PresetGroup;

        let presets = scene::presets();
        let group = |id: &str| {
            presets
                .iter()
                .find(|preset| preset.id == id)
                .map(|preset| preset.group)
        };
        assert_eq!(group("blink"), Some(PresetGroup::Basics));
        assert_eq!(group("slide-pot"), Some(PresetGroup::Basics));
        assert_eq!(group("photoresistor"), Some(PresetGroup::Sensors));
        assert_eq!(group("ultrasonic"), Some(PresetGroup::Sensors));
        assert_eq!(group("lcd"), Some(PresetGroup::Outputs));
        assert_eq!(group("shift-register"), Some(PresetGroup::Outputs));
        // A preset hidden behind no tab is a preset nobody can open.
        let listed: usize = PresetGroup::ALL
            .iter()
            .map(|group| presets.iter().filter(|preset| preset.group == *group).count())
            .sum();
        assert_eq!(listed, presets.len());
        assert!(
            PresetGroup::ALL
                .iter()
                .all(|group| presets.iter().any(|preset| preset.group == *group)),
            "every tab has to hold something"
        );
        // Id order, the teaching order v0.2 lists them in.
        let ids: Vec<&str> = presets.iter().map(|preset| preset.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);
    }

    #[test]
    fn a_palette_picture_keeps_the_parts_aspect() {
        // The resistor is 5.2:1. Rasterising it into a 2:1 box squashed it into
        // the fat bar the tile used to show.
        let resistor = scene::part_from_kind(
            croquis_core::catalogue::PartKind::Resistor,
            Pos2::ZERO,
            "r".into(),
        )
        .expect("the catalogue has a resistor");
        let aspect = resistor.size.x / resistor.size.y;
        assert!(
            (aspect - 5.22).abs() < 0.05,
            "the manifest's resistor is wide: {aspect}"
        );
        let height = PALETTE_ART_HEIGHT;
        let width = ((height as f32 * aspect).round() as u32).clamp(8, 1024);
        assert!(
            (width as f32 / height as f32 - aspect).abs() < 0.05,
            "the picture is baked at the part's own aspect"
        );
    }

    #[test]
    fn a_quarter_turn_swaps_the_box_and_carries_the_pins() {
        let mut spike = Spike::default();
        let led = spike
            .parts
            .iter()
            .position(|part| part.kind == "led")
            .expect("blink has an LED");
        let centre = spike.parts[led].center;
        let size = spike.parts[led].size;
        let before = spike.parts[led].pin_world(0) - centre;

        spike.selection.select_part(led);
        spike.rotate_selection(90.0);

        let part = &spike.parts[led];
        assert_eq!(part.rotation, 90.0);
        assert_eq!(part.rotated_size(), Vec2::new(size.y, size.x));
        assert_eq!(part.bounds().center(), centre);
        let after = part.pin_world(0) - centre;
        assert!((after.length() - before.length()).abs() < 0.01);
        // A quarter turn clockwise sends "up" to "right".
        assert!(
            (after - Vec2::new(-before.y, before.x)).length() < 0.01,
            "the pin turned with the part: {before:?} -> {after:?}"
        );
    }

    #[test]
    fn four_quarter_turns_come_back_where_they_started() {
        let mut spike = Spike::default();
        spike.selection.parts = vec![0, 1, 2];
        let before: Vec<Pos2> = (0..spike.parts.len())
            .map(|index| spike.parts[index].pin_world(0))
            .collect();
        for _ in 0..4 {
            spike.rotate_selection(90.0);
        }
        for index in 0..spike.parts.len() {
            assert_eq!(spike.parts[index].rotation, 0.0);
            assert!((spike.parts[index].pin_world(0) - before[index]).length() < 0.01);
        }
    }

    #[test]
    fn a_turn_survives_a_save_and_a_load() {
        let mut spike = Spike::default();
        spike.selection.parts = vec![0, 1, 2];
        spike.rotate_selection(90.0);
        let path = std::env::temp_dir().join("croquis-rotate-roundtrip.json");
        scene::save_diagram_file(&path, &spike.parts, &spike.wires, &spike.sketch).expect("save");

        let (parts, wires, _sketch) = scene::load_diagram_file(&path);
        assert_eq!(parts.len(), spike.parts.len());
        assert_eq!(wires.len(), spike.wires.len());
        for (before, after) in spike.parts.iter().zip(parts.iter()) {
            assert_eq!(after.rotation, 90.0, "{} lost its turn", before.id);
            assert_eq!(after.center, before.center);
            for index in 0..before.pins.len() {
                assert!(
                    (after.pin_world(index) - before.pin_world(index)).length() < 0.01,
                    "{} pin {} moved",
                    before.id,
                    index
                );
            }
        }
    }

    #[test]
    fn deleting_a_parts_takes_its_wires_and_leaves_the_rest_alone() {
        // Blink: the Uno, a resistor and an LED, wired end to end.
        let mut spike = Spike::default();
        assert_eq!(spike.parts.len(), 3);
        assert_eq!(spike.wires.len(), 3);

        // The resistor in the middle: its two wires go, the LED–Uno wire stays.
        spike.selection.select_part(1);
        spike.delete_selection();

        assert_eq!(spike.parts.len(), 2);
        assert_eq!(
            spike.wires.len(),
            1,
            "only the wire that joins the two survivors is left"
        );
        assert!(spike.selection.is_empty());
        for wire in &spike.wires {
            assert!(wire.a.0 < spike.parts.len() && wire.b.0 < spike.parts.len());
        }
    }

    #[test]
    fn deleting_one_of_two_parts_keeps_the_others_wires_indexed() {
        // Four parts, wired 0-1, 1-2, 2-3: removing part 1 must drop the first
        // two wires and pull the third's indices down with it.
        let mut spike = Spike::default();
        let kinds = [
            croquis_core::catalogue::PartKind::Resistor,
            croquis_core::catalogue::PartKind::Resistor,
        ];
        for (offset, kind) in kinds.into_iter().enumerate() {
            let id = format!("extra-{offset}");
            let mut part = scene::part_from_kind(kind, Pos2::new(600.0 + 40.0 * offset as f32, 600.0), id)
                .expect("resistor");
            part.pins = vec![part.pins[0].clone(), part.pins[1].clone()];
            spike.parts.push(part);
        }
        spike.wires.push(Wire {
            a: (3, 1),
            b: (4, 0),
            waypoints: Vec::new(),
        });

        spike.selection.select_part(1);
        spike.delete_selection();

        assert_eq!(spike.parts.len(), 4);
        let wire = spike.wires.last().expect("the far wire survives");
        assert!(
            wire.a.0 < spike.parts.len() && wire.b.0 < spike.parts.len(),
            "indices above the hole move down: {} -> {}",
            wire.a.0,
            wire.b.0
        );
        assert_eq!(wire.a.0, 2);
        assert_eq!(wire.b.0, 3);
    }

    #[test]
    fn copy_and_paste_copies_the_wiring_between_the_parts() {
        let mut spike = Spike::default();
        spike.selection.parts = vec![0, 1, 2];
        spike.copy_selection();
        spike.paste();

        assert_eq!(spike.parts.len(), 6);
        assert_eq!(spike.wires.len(), 6, "the pasted parts are wired to each other");
        assert_eq!(spike.selection.parts, vec![3, 4, 5]);
        // The pasted copies are new parts, offset by a grid step, not the same
        // parts moved.
        assert_ne!(spike.parts[3].id, spike.parts[0].id);
        assert_eq!(
            spike.parts[3].center - spike.parts[0].center,
            Vec2::new(GRID, GRID)
        );
        for wire in &spike.wires[3..] {
            assert!(
                wire.a.0 >= 3 && wire.b.0 >= 3,
                "wired to the copies: {} -> {}",
                wire.a.0,
                wire.b.0
            );
        }
    }

    #[test]
    fn a_pasted_wire_keeps_its_bends() {
        let mut spike = Spike::default();
        spike.wires[0].waypoints = vec![Pos2::new(80.0, 90.0)];
        spike.selection.parts = vec![0, 1, 2];
        spike.copy_selection();
        spike.paste();
        let pasted = &spike.wires[3];
        assert_eq!(pasted.waypoints.len(), 1);
        assert_eq!(
            pasted.waypoints[0],
            Pos2::new(80.0, 90.0) + Vec2::new(GRID, GRID)
        );
    }

    #[test]
    fn a_folded_serial_monitor_keeps_its_head_on_screen() {
        // Off takes no room; open takes its log; folded takes its head and never
        // nothing — the chevron that unfolds it lives in that head.
        assert_eq!(serial_height(false, false), 0.0);
        assert_eq!(serial_height(false, true), 0.0, "off beats folded");
        assert_eq!(serial_height(true, false), SERIAL_LOG_HEIGHT);
        assert_eq!(serial_height(true, true), SERIAL_HEAD_HEIGHT);
        assert!(
            serial_height(true, true) > 0.0,
            "a folded monitor still has to show the control that unfolds it"
        );
        assert!(
            serial_height(true, true) < serial_height(true, false),
            "and folding it has to give the canvas the room back"
        );
    }

    #[test]
    fn a_panels_chevron_points_the_way_it_folds() {
        // The sketch panel is docked left and folds leftward; the components
        // panel is docked right; the serial monitor runs along the bottom. Each
        // one used to draw the same sideways chevron they all shared.
        assert_eq!(Dock::Left.closing(), Chevron::Left);
        assert_eq!(Dock::Left.opening(), Chevron::Right);
        assert_eq!(Dock::Right.closing(), Chevron::Right);
        assert_eq!(Dock::Right.opening(), Chevron::Left);
        assert_eq!(Dock::Bottom.closing(), Chevron::Down);
        assert_eq!(Dock::Bottom.opening(), Chevron::Up);
    }

    #[test]
    fn a_chevron_apex_points_the_way_its_name_says() {
        for (direction, outward) in [
            (Chevron::Left, Vec2::new(-1.0, 0.0)),
            (Chevron::Right, Vec2::new(1.0, 0.0)),
            (Chevron::Up, Vec2::new(0.0, -1.0)),
            (Chevron::Down, Vec2::new(0.0, 1.0)),
        ] {
            let (apex, wings) = chevron_wedge(direction);
            let tip = Vec2::new(apex.0, apex.1);
            assert!(
                tip.dot(outward) > 0.0,
                "{direction:?}'s apex does not point outward: {tip:?}"
            );
            // The wings have to sit behind the apex, or the wedge is not a wedge.
            for wing in wings {
                let at = Vec2::new(wing.0, wing.1);
                assert!(
                    at.dot(outward) < 0.0,
                    "{direction:?} has a wing in front of its apex: {at:?}"
                );
            }
        }
    }

    #[test]
    fn the_marquee_selects_the_parts_it_covers() {
        let mut spike = Spike::default();
        let led = spike
            .parts
            .iter()
            .position(|part| part.kind == "led")
            .expect("blink has an LED");
        let area = spike.parts[led].bounds();
        spike.select_in(area, false);
        assert!(spike.selection.has_part(led));
        assert_eq!(spike.selection.parts.len(), 1, "only what it covers");

        // Shift-marquee adds rather than replaces.
        let uno = spike
            .parts
            .iter()
            .position(|part| part.kind == "uno")
            .expect("blink has an Uno");
        spike.select_in(spike.parts[uno].bounds(), true);
        assert!(spike.selection.has_part(led) && spike.selection.has_part(uno));
    }

    /// A bare part: a box with a pin at each offset from its centre.
    fn part_of_kind(kind: &str, center: Pos2, pins: &[(f32, f32)]) -> Part {
        Part {
            id: format!("{kind}-at-{}-{}", center.x, center.y),
            kind: kind.into(),
            name_key: format!("part.{kind}"),
            center,
            size: Vec2::new(20.0, 20.0),
            rotation: 0.0,
            color: Color32::BLACK,
            pins: pins
                .iter()
                .map(|(x, y)| scene::Pin {
                    name: "p".into(),
                    offset: Vec2::new(*x, *y),
                })
                .collect(),
            attrs: std::collections::HashMap::new(),
            svg: String::new(),
        }
    }

    fn snapping_part(center: Pos2, pins: &[(f32, f32)]) -> Part {
        part_of_kind("led", center, pins)
    }

    #[test]
    fn dragging_a_breadboard_carries_the_parts_seated_in_it() {
        // A breadboard's holes are its pins. One LED has a leg in a hole; another
        // is somewhere else on the canvas entirely.
        let board = part_of_kind(
            "breadboard",
            Pos2::new(100.0, 100.0),
            &[(0.0, 0.0), (20.0, 0.0)],
        );
        let seated = part_of_kind("led", Pos2::new(102.0, 100.0), &[(-2.0, 0.0)]);
        let elsewhere = part_of_kind("led", Pos2::new(300.0, 300.0), &[(0.0, 0.0)]);
        let parts = vec![board, seated, elsewhere];
        assert_eq!(
            seated_with(&parts, &[0]),
            vec![0, 1],
            "the board and the leg in it, and nothing else"
        );
    }

    #[test]
    fn a_part_that_is_merely_near_the_board_stays_behind() {
        let board = part_of_kind("breadboard", Pos2::new(100.0, 100.0), &[(0.0, 0.0)]);
        // Six units away: beside the board, in none of its holes.
        let nearby = part_of_kind("led", Pos2::new(106.0, 100.0), &[(0.0, 0.0)]);
        let parts = vec![board, nearby];
        assert_eq!(seated_with(&parts, &[0]), vec![0]);
    }

    #[test]
    fn dragging_only_the_part_leaves_the_board_alone() {
        let board = part_of_kind("breadboard", Pos2::new(100.0, 100.0), &[(0.0, 0.0)]);
        let seated = part_of_kind("led", Pos2::new(100.0, 100.0), &[(0.0, 0.0)]);
        let parts = vec![board, seated];
        assert_eq!(
            seated_with(&parts, &[1]),
            vec![1],
            "a part carried by a board does not drag the board with it"
        );
    }

    #[test]
    fn a_pin_that_comes_close_to_another_lands_on_it() {
        let still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let moved = snapping_part(Pos2::new(92.0, 100.0), &[(-10.0, 0.0)]);
        // The pins are eight apart — inside the radius, so the eight pixels are
        // what the nudge has to cover, exactly and in one step.
        let nudge = snap_nudge(&[still, moved], &[1], SNAP_RADIUS).expect("a snap");
        assert!(
            (nudge.x - 8.0).abs() < 0.001 && nudge.y.abs() < 0.001,
            "expected the whole gap, got {nudge:?}"
        );
    }

    #[test]
    fn a_pin_outside_the_snap_radius_is_left_alone() {
        let still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let moved = snapping_part(Pos2::new(90.0 - SNAP_RADIUS - 1.0, 100.0), &[(-10.0, 0.0)]);
        assert!(
            snap_nudge(&[still, moved], &[1], SNAP_RADIUS).is_none(),
            "a pin must not jump from outside the radius"
        );
    }

    #[test]
    fn a_pick_does_not_snap_to_itself() {
        let a = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let b = snapping_part(Pos2::new(104.0, 100.0), &[(-10.0, 0.0)]);
        assert!(
            snap_nudge(&[a, b], &[0, 1], SNAP_RADIUS).is_none(),
            "two pins being dragged together are already where the hand put them"
        );
    }

    #[test]
    fn a_snapped_pin_does_not_creep_past_its_target() {
        let still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        // The dragged part's pin is at x = 90 too, from the other side, so the
        // two pins already coincide and the nudge has nothing left to close.
        let moved = snapping_part(Pos2::new(80.0, 100.0), &[(10.0, 0.0)]);
        assert_eq!(moved.pin_world(0), still.pin_world(0), "the pins coincide");
        assert!(
            snap_nudge(&[still, moved], &[1], SNAP_RADIUS).is_none(),
            "a pin sitting on its target has no gap left to close"
        );
    }

    #[test]
    fn the_closest_pin_pair_wins_in_a_pick_of_several() {
        let still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0), (10.0, 0.0)]);
        // The still part's pins sit at x = 90 and x = 110. The first dragged part
        // is eight short of 90, the second three short of 110: the nearer pair is
        // the one the pick locks onto, and the nudge is that pair's gap.
        let far = snapping_part(Pos2::new(92.0, 100.0), &[(-10.0, 0.0)]);
        let near = snapping_part(Pos2::new(123.0, 100.0), &[(-10.0, 0.0)]);
        let nudge = snap_nudge(&[still, far, near], &[1, 2], SNAP_RADIUS).expect("a snap");
        assert!(
            (nudge.x + 3.0).abs() < 0.001,
            "expected the nearer pair's gap, got {nudge:?}"
        );
    }

    #[test]
    fn a_turned_part_snaps_on_the_pins_it_actually_draws() {
        let mut still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        still.rotation = 90.0;
        // A quarter turn puts the pin below the centre, not to its left, so a
        // part dragged in from the left must be out of reach.
        let beside = snapping_part(Pos2::new(92.0, 100.0), &[(-10.0, 0.0)]);
        assert!(
            snap_nudge(&[still.clone(), beside], &[1], SNAP_RADIUS).is_none(),
            "the pin turned away; nothing is within reach on that side"
        );
        let below = snapping_part(Pos2::new(110.0, 98.0), &[(-10.0, 0.0)]);
        let nudge = snap_nudge(&[still, below], &[1], SNAP_RADIUS).expect("a snap");
        assert!(
            (nudge.y + 8.0).abs() < 0.001,
            "expected the turned pin's gap on y, got {nudge:?}"
        );
    }

    #[test]
    fn the_app_starts_in_the_language_it_is_given() {
        // `Default` must stay English so no test depends on this machine's saved
        // config; starting_in is the one that honours a choice.
        let english = Spike::default();
        assert_eq!(english.texts.locale(), locales::Locale::English);
        assert_eq!(english.texts.t("menu.file"), "File");
        let turkish = Spike::starting_in(locales::Locale::Turkish);
        assert_eq!(turkish.texts.locale(), locales::Locale::Turkish);
        assert_eq!(turkish.texts.t("menu.file"), "Dosya");
        assert_eq!(
            turkish.texts.t("view.zoom_in"),
            "Yakınlaştır",
            "and the whole shell follows, not just the menu name"
        );
    }

    #[test]
    fn a_snap_lets_go_once_the_pointer_travels_past_it() {
        // The still part's pin sits at x = 90; the dragged part's pin is ten to
        // the left of its centre, so the centre that puts it on 90 is x = 100.
        let still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let moved = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let parts = vec![still, moved];
        let origin = vec![(1usize, Pos2::new(100.0, 100.0))];

        // Eight short of the pin: inside the radius, so the gap closes.
        let locked = drag_placement(&parts, &origin, Vec2::new(-8.0, 0.0), SNAP_RADIUS);
        assert_eq!(
            locked[0].1 + Vec2::new(-10.0, 0.0),
            Pos2::new(90.0, 100.0),
            "the pin should land exactly on the still one"
        );

        // Past the radius: the pointer wins and nothing pulls it back.
        let travelled = SNAP_RADIUS + 5.0;
        let free = drag_placement(&parts, &origin, Vec2::new(-travelled, 0.0), SNAP_RADIUS);
        assert_eq!(
            free[0].1,
            Pos2::new(100.0 - travelled, 100.0),
            "a part dragged clear should sit where the pointer put it"
        );
    }

    #[test]
    fn a_snapped_pick_is_not_pulled_back_every_frame() {
        // Two frames of one drag, both measured from the same origin. The first
        // snaps; the second has travelled past the radius and must be free.
        // When the nudge was baked into the part's centre, the second frame was
        // impossible and the part had to be ripped off the hole.
        let still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let moved = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let parts = vec![still, moved];
        let origin = vec![(1usize, Pos2::new(100.0, 100.0))];

        let first = drag_placement(&parts, &origin, Vec2::new(-6.0, 0.0), SNAP_RADIUS);
        assert_eq!(first[0].1, Pos2::new(100.0, 100.0), "snapped on frame one");

        let second = drag_placement(&parts, &origin, Vec2::new(-30.0, 0.0), SNAP_RADIUS);
        assert_eq!(
            second[0].1,
            Pos2::new(70.0, 100.0),
            "and let go on frame two, without the snap re-applying"
        );
    }

    #[test]
    fn a_pick_that_is_dragged_keeps_its_own_shape() {
        // Two parts dragged together hold their relative positions, snapped or
        // not: the nudge is one vector for the whole pick.
        let still = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let a = snapping_part(Pos2::new(100.0, 100.0), &[(-10.0, 0.0)]);
        let b = snapping_part(Pos2::new(140.0, 100.0), &[(-10.0, 0.0)]);
        let parts = vec![still, a, b];
        let origin = vec![(1usize, Pos2::new(100.0, 100.0)), (2usize, Pos2::new(140.0, 100.0))];
        let placed = drag_placement(&parts, &origin, Vec2::new(0.0, -40.0), SNAP_RADIUS);
        assert_eq!(
            placed[1].1.x - placed[0].1.x,
            40.0,
            "the pick's own spacing survives the drag"
        );
    }

    #[test]
    fn a_new_document_keeps_the_palette_art_and_drops_the_canvas_art() {
        // Generic over the value type so this needs no GPU context: the point is
        // which keys survive, not what a texture holds.
        let mut textures: HashMap<String, u32> = HashMap::new();
        textures.insert(format!("{PALETTE_PREFIX}led"), 1);
        textures.insert(format!("{PALETTE_PREFIX}resistor"), 2);
        textures.insert("led-instance-1".into(), 3);
        textures.insert("uno-instance-2".into(), 4);
        keep_palette(&mut textures);
        assert_eq!(textures.len(), 2, "only the palette survives");
        assert!(textures.contains_key("palette:led"));
        assert!(
            !textures.contains_key("led-instance-1"),
            "a part's canvas art belongs to a part that no longer exists"
        );
    }

    #[test]
    fn a_new_document_starts_from_the_sketch_template() {
        let mut spike = Spike::default();
        assert!(!spike.parts.is_empty(), "the preset loaded first");
        spike.new_document();
        assert!(spike.parts.is_empty(), "and the document is empty");
        assert!(
            spike.sketch.contains("void setup()"),
            "a new sketch is not a blank page: {:?}",
            spike.sketch
        );
        assert!(spike.sketch.contains("void loop()"));
        assert!(
            spike.sketch.contains("// write your setup code here"),
            "with the comments a learner is meant to read"
        );
    }

    #[test]
    fn the_steppers_hub_outline_stays_on_its_circle() {
        let rotor = scene::STEPPER_ROTOR;
        let (cx, cy) = rotor.centre_in_view();
        let radius = rotor.radius_in_view();
        for degrees in [0.0, 45.0, 180.0, 270.0] {
            for (x, y) in rotor.hub_outline(degrees) {
                let distance = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                assert!(
                    (distance - radius).abs() < 0.5,
                    "at {degrees}° a point sits {distance} from the centre, not {radius}"
                );
            }
        }
    }

    #[test]
    fn the_hubs_flat_sits_where_the_element_puts_it() {
        let rotor = scene::STEPPER_ROTOR;
        let (_, cy) = rotor.centre_in_view();
        let chord = rotor.chord_in_view();
        // At rest the cut runs across the top, so nothing rises above it.
        let highest = rotor
            .hub_outline(0.0)
            .into_iter()
            .map(|(_, y)| y)
            .fold(f32::INFINITY, f32::min);
        assert!(
            (highest - (cy + chord)).abs() < 0.5,
            "the cut should be at {}, not {highest}",
            cy + chord
        );
        // Half a turn puts it at the bottom, which is the point of drawing it live.
        let lowest = rotor
            .hub_outline(180.0)
            .into_iter()
            .map(|(_, y)| y)
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            (lowest - (cy - chord)).abs() < 0.5,
            "after half a turn the cut should be at {}, not {lowest}",
            cy - chord
        );
    }

    #[test]
    fn a_steppers_baked_art_carries_no_rotor() {
        // The rotor is drawn live now. If it were still in the texture, the
        // texture would still change with every angle — which was the lag.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/wokwi/stepper-motor.svg");
        let svg = std::fs::read_to_string(&path).expect("the stepper's art");
        let base = scene::stepper_base_svg(&svg);
        assert!(!base.contains("arrow-path"), "the rotor arrow is drawn live");
        assert!(!base.contains("shaft-path"), "so is the hub");
        assert!(base.contains("motor body"), "and the body is untouched");
        assert_eq!(
            base,
            scene::stepper_base_svg(&base),
            "stripping it twice changes nothing"
        );

        // The rotor's geometry is stated as constants, so they have to match this
        // file. If the art is ever replaced, this is what says the numbers moved.
        let rotor = scene::STEPPER_ROTOR;
        for (name, value) in [
            ("scale", rotor.scale),
            ("translate", rotor.translate),
            ("centre", rotor.centre),
            ("radius", rotor.radius),
            ("chord", rotor.chord),
        ] {
            assert!(
                svg.contains(&format!("{value}")),
                "the art no longer contains the rotor's {name} ({value}), so the constant is stale"
            );
        }
    }

    #[test]
    fn a_wire_over_a_part_is_what_the_pointer_finds() {
        let camera = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        // Two pins to hang a wire between, and a part sitting under its middle.
        let left = part_of_kind("led", Pos2::new(100.0, 100.0), &[(0.0, 0.0)]);
        let right = part_of_kind("led", Pos2::new(300.0, 100.0), &[(0.0, 0.0)]);
        let under = part_of_kind("resistor", Pos2::new(200.0, 100.0), &[]);
        let parts = vec![left, right, under];
        let wires = vec![Wire {
            a: (0, 0),
            b: (1, 0),
            waypoints: Vec::new(),
        }];
        let screen = camera.world_to_screen(rect, Pos2::new(200.0, 100.0));
        assert_eq!(
            pick_at(
                &parts,
                &wires,
                camera,
                rect,
                screen,
                PIN_HIT_RADIUS / camera.zoom
            ),
            Pick::Wire(0),
            "the wire is painted over the part, so it is what the pointer is on"
        );
    }

    #[test]
    fn a_pin_under_another_part_cannot_be_grabbed_through_it() {
        let camera = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        // The board's pin sits at its centre and a resistor is drawn over it.
        let board = part_of_kind("breadboard", Pos2::new(100.0, 100.0), &[(0.0, 0.0)]);
        let covering = part_of_kind("resistor", Pos2::new(100.0, 100.0), &[]);
        let parts = vec![board, covering];
        let screen = camera.world_to_screen(rect, Pos2::new(100.0, 100.0));
        assert_eq!(
            pick_at(
                &parts,
                &[],
                camera,
                rect,
                screen,
                PIN_HIT_RADIUS / camera.zoom
            ),
            Pick::Part(1),
            "the piece on top is what the pointer finds, not the pin beneath it"
        );
    }

    #[test]
    fn an_exposed_pin_is_the_first_thing_the_pointer_finds() {
        let camera = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let board = part_of_kind("breadboard", Pos2::new(100.0, 100.0), &[(0.0, 0.0)]);
        let parts = vec![board];
        let screen = camera.world_to_screen(rect, Pos2::new(100.0, 100.0));
        assert_eq!(
            pick_at(
                &parts,
                &[],
                camera,
                rect,
                screen,
                PIN_HIT_RADIUS / camera.zoom
            ),
            Pick::Pin((0, 0)),
            "a pin nothing covers is still how a wire gets started"
        );
    }

    #[test]
    fn a_wire_can_be_clicked_and_a_point_away_from_it_cannot() {
        let spike = Spike::default();
        let camera = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let path = wire_screen_path(&spike.parts, &spike.wires[0], camera, rect);
        let middle = path[0] + (path[1] - path[0]) * 0.5;

        assert_eq!(
            hit_wire(&spike.parts, &spike.wires, camera, rect, middle, WIRE_HIT_RADIUS),
            Some(0),
            "a click on the wire selects it"
        );
        assert_eq!(
            hit_wire(
                &spike.parts,
                &spike.wires,
                camera,
                rect,
                middle + Vec2::new(0.0, 200.0),
                WIRE_HIT_RADIUS
            ),
            None
        );
    }

    #[test]
    fn a_marquee_catches_a_wire_that_crosses_it() {
        let spike = Spike::default();
        let camera = Camera::default();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 800.0));
        let path = wire_screen_path(&spike.parts, &spike.wires[0], camera, rect);
        // A world box that holds the middle of the wire and neither end.
        let middle = (path[0] + (path[1] - path[0]) * 0.5 - rect.center()).to_pos2();
        let area = Rect::from_center_size(middle, Vec2::splat(4.0));
        assert!(segment_hits_rect(
            (path[0] - rect.center()).to_pos2(),
            (path[1] - rect.center()).to_pos2(),
            area
        ));
        assert!(!segment_hits_rect(
            Pos2::new(-500.0, -500.0),
            Pos2::new(-400.0, -500.0),
            area
        ));
    }

    #[test]
    fn the_keypad_mark_covers_the_key_the_tap_landed_on() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/keypad.json");
        let (parts, _wires, _sketch) = scene::load_diagram_file(&root);
        let pad = parts
            .iter()
            .find(|part| part.kind == "membrane-keypad")
            .expect("the preset has a keypad");
        for row in 0..4 {
            for column in 0..4 {
                let key = KeyPosition { row, column };
                let area = keypad_key_world(pad, key).expect("a key has a face");
                assert_eq!(
                    keypad_key(pad, area.center()),
                    Some(key),
                    "the mark and the hit-test disagree at {row},{column}"
                );
            }
        }
    }

    #[test]
    fn the_joysticks_invisible_hit_areas_are_not_in_its_palette_art() {
        let joystick = scene::part_from_kind(
            croquis_core::catalogue::PartKind::AnalogJoystick,
            Pos2::ZERO,
            "j".into(),
        )
        .expect("the catalogue has a joystick");
        let art = scene::resting_svg(&joystick);
        let regions = joystick.svg.matches("class=\"region\"").count();
        assert!(
            regions > 0,
            "the element's hit-areas are what rendered as grey blocks"
        );
        assert_eq!(
            art.matches("class=\"region\" fill=\"none\"").count(),
            regions,
            "every hit-area keeps its outline and loses its fill"
        );
        assert!(
            !art.contains("<circle id=\"knob\""),
            "the knob is drawn live, so it must not be baked into palette art"
        );
    }

    #[test]
    fn presets_load_through_the_generic_loader() {
        let presets = scene::presets();
        assert!(presets.len() >= 20, "shipped presets are discoverable");
        let (parts, wires, sketch) = scene::load_diagram_file(&presets[0].path);
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
    fn a_display_takes_a_new_pattern_unless_it_arrived_mid_write() {
        let shown = [1u8, 0, 0, 0, 0, 0, 0, 0];
        let values = [0u8, 1, 0, 0, 0, 0, 0, 0];
        assert!(
            !segment_settled(&shown, &shown, None),
            "an unchanged pattern is not a change"
        );
        assert!(
            segment_settled(&shown, &values, None),
            "the first pattern is always taken"
        );
        assert!(
            segment_settled(&shown, &values, Some(SEGMENT_SETTLE)),
            "a change a settle window later is a real digit"
        );
        assert!(
            !segment_settled(&shown, &values, Some(Duration::from_millis(2))),
            "a change 2 ms after the last one is half a write"
        );
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
    fn the_boards_led_and_a_pressed_button_are_both_in_the_art() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wokwi");

        let uno = std::fs::read_to_string(root.join("uno.svg")).expect("uno art");
        let dark = scene::uno_svg(&uno, false);
        let lit = scene::uno_svg(&uno, true);
        assert_eq!(dark, uno, "an unlit LED leaves the art alone");
        assert!(
            lit.contains("#ff8080") && lit.contains("url(#ledFilter)"),
            "pin 13 high puts the element's own red dot in the LED group"
        );
        assert_ne!(dark, lit);

        let button = std::fs::read_to_string(root.join("pushbutton.svg")).expect("button art");
        let up = scene::pushbutton_svg(&button, false);
        let down = scene::pushbutton_svg(&button, true);
        assert!(
            up.contains("style=\"display: none\""),
            "a resting cap hides the element's pressed circle"
        );
        assert!(
            !down.contains("style=\"display: none\""),
            "a held cap shows the pressed circle"
        );
        assert_ne!(up, down);
    }

    #[test]
    fn the_potentiometers_base_art_leaves_out_the_moving_part() {
        // The tick and the tip are drawn live, at about 40 ms a state cheaper
        // than rasterising them per value, so the base art must not carry them:
        // one of each, and both costing nothing on a drag.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wokwi");

        let pot = std::fs::read_to_string(root.join("potentiometer.svg")).expect("pot asset");
        let base = scene::potentiometer_svg(&pot);
        assert!(!base.contains("id=\"rotating\""), "the tick is drawn live");
        assert_ne!(base, pot, "and the art is otherwise untouched");

        let slide =
            std::fs::read_to_string(root.join("slide-potentiometer.svg")).expect("slide asset");
        let base = scene::slide_pot_svg(&slide);
        assert!(!base.contains("id=\"tip\""), "the tip is drawn live");
        assert_ne!(base, slide);
    }

    #[test]
    fn editing_a_property_resolves_the_circuit() {
        // The tilt switch is the case the owner could not use: its `tilted`
        // attribute is the whole switch, and nothing could change it. This is
        // the panel's write path, then the solve it triggers.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/digital-sensors.json");
        let (parts, wires, _) = scene::load_diagram_file(&path);
        let mut spike = Spike::default();
        let tilt = parts
            .iter()
            .position(|part| part.id == "tilt1")
            .expect("tilt switch");
        spike.parts = parts;
        spike.wires = wires;

        spike.evaluate_now();
        assert!(!spike.sensors["tilt1"].active, "upright to start with");

        use croquis_core::catalogue::{parse_kind, part_by_kind};
        let field = part_by_kind(parse_kind("tilt-switch").expect("kind"))
            .fields
            .iter()
            .find(|field| field.name == "tilted")
            .expect("tilted field")
            .clone();
        assert!(properties::field_commit(
            &mut spike.parts[tilt],
            &field,
            properties::EditorValue::Toggle(true)
        ));
        spike.evaluate_now();
        assert!(
            spike.sensors["tilt1"].active,
            "tilting it has to reach the netlist, which is what every preset              that reads a sensor depends on"
        );
    }

    #[test]
    fn the_button_preset_drives_its_pin_when_pressed() {
        // The real preset, through the real loader and the real netlist — no UI.
        // If this fails, nothing the app does afterwards can work.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/button.json");
        let (parts, wires, _sketch) = scene::load_diagram_file(&root);

        let ids: Vec<&str> = parts.iter().map(|part| part.id.as_str()).collect();
        println!("parts: {ids:?}");
        let button = parts
            .iter()
            .find(|part| part.kind == "pushbutton")
            .map(|part| part.id.clone())
            .expect("the preset has a pushbutton");

        let avr = HashMap::new();
        let idle = scene::evaluate_scene(
            &parts,
            &wires,
            &HashMap::new(),
            &avr,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        println!("idle drives: {:?}", idle.drives);

        let mut held = HashMap::new();
        held.insert(button.clone(), true);
        let pressed = scene::evaluate_scene(
            &parts,
            &wires,
            &held,
            &avr,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
        println!("pressed drives: {:?}", pressed.drives);
        assert!(
            pressed.drives.iter().any(|drive| drive.port == 'D' && drive.pin == 2),
            "a press must drive the button's pin; got {:?}",
            pressed.drives
        );
    }

    #[test]
    fn a_press_pushes_the_drive_to_the_simulator() {
        // The whole app link without a window: load the preset, press the
        // button, solve, and check what was actually handed to the simulator.
        // `last_drives` is the record of exactly that.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/button.json");
        let (parts, wires, sketch) = scene::load_diagram_file(&root);
        let button = parts
            .iter()
            .position(|part| part.kind == "pushbutton")
            .expect("the preset has a pushbutton");

        let workdir = std::env::temp_dir().join("croquis-egui-press-test");
        let _ = std::fs::create_dir_all(&workdir);
        let firmware = engine::compile(&sketch, &workdir).expect("the sketch compiles");
        let events: Arc<Mutex<Vec<engine::SimEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: engine::Emit = Arc::new(move |event| sink.lock().unwrap().push(event));
        let sim = engine::Simulator::start(Path::new(&firmware.elf), emit, false)
            .expect("the simulator starts");

        let mut spike = Spike::default();
        spike.parts = parts;
        spike.wires = wires;
        spike.sim = Some(sim);
        spike.selection.select_part(button);
        spike.evaluate_now();
        let idle = spike.last_drives.get(&('D', 2)).copied();

        let centre = spike.parts[button].bounds().center();
        spike.press_control(button, centre, Pos2::ZERO);
        spike.reevaluate = false;
        spike.evaluate_now();
        let pressed = spike.last_drives.get(&('D', 2)).copied();
        println!("D2: idle={idle:?} pressed={pressed:?}");

        assert_eq!(pressed, Some(false), "a press must push D2 low to the sim");
        if let Some(mut sim) = spike.sim.take() {
            sim.stop();
        }
    }

    #[test]
    fn a_diagnostic_jump_writes_the_caret_into_the_editor_state() {
        // The half of a jump that is not the pointer: given the editor's state,
        // the caret lands on the character the compiler named.
        let ctx = egui::Context::default();
        egui::text_edit::TextEditState::default().store(&ctx, sketch_editor_id());
        let sketch = "void setup() {\n  pinModee(13, OUTPUT);\n}\n";
        assert!(place_caret(&ctx, sketch, 2, 3));
        let state = egui::text_edit::TextEditState::load(&ctx, sketch_editor_id())
            .expect("the state is still there");
        assert_eq!(
            state.cursor.char_range(),
            Some(CCursorRange::one(CCursor::new(17))),
            "line 2, column 3 is the 'p' of pinModee"
        );
    }

    #[test]
    fn a_held_button_looks_different_from_a_resting_one() {
        // The element swaps the cap's gradient while the button is held. If the
        // two rasters differ by a handful of pixels the press cannot be seen,
        // which is what "the pressed state does not show" feels like.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/wokwi");
        let button = std::fs::read_to_string(root.join("pushbutton.svg")).expect("button art");
        let up = art::rasterize(&scene::pushbutton_svg(&button, false), 200, 135)
            .expect("the resting cap rasterises");
        let down = art::rasterize(&scene::pushbutton_svg(&button, true), 200, 135)
            .expect("the held cap rasterises");
        let different = up
            .pixels
            .iter()
            .zip(&down.pixels)
            .filter(|(a, b)| a != b)
            .count();
        assert!(
            different > 500,
            "a held cap has to look held: {different} pixels differ"
        );
    }

    #[test]
    fn a_broken_sketch_names_the_line_the_compiler_stopped_on() {
        // The real compiler through the real engine: the caret can only land on
        // the typo's line if the parser reads avr-gcc's output and the `#line`
        // directive puts the sketch's own name and numbering in it.
        let workdir = std::env::temp_dir().join("croquis-egui-diagnostics-test");
        let _ = std::fs::create_dir_all(&workdir);
        let sketch = "void setup() {\n  pinModee(13, OUTPUT);\n}\n\nvoid loop() {}\n";
        let error = engine::compile(sketch, &workdir).expect_err("the typo cannot compile");
        let diagnostics = editor::parse_diagnostics(&error);
        assert!(!diagnostics.is_empty(), "no diagnostics in: {error}");
        let first = &diagnostics[0];
        assert_eq!(first.line, 2, "wrong line in {diagnostics:?}");
        assert!(first.in_sketch(), "not the sketch's own file: {diagnostics:?}");
        assert_eq!(first.severity, editor::Severity::Error);
        assert!(
            first.message.contains("pinModee"),
            "the message has to name the typo: {}",
            first.message
        );
    }

    #[test]
    fn the_keypad_preset_reports_a_held_key() {
        // The whole loop with no window: real simulator, real sketch, a key
        // held, and the serial monitor checked for the key it should print.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/keypad.json");
        let (parts, wires, sketch) = scene::load_diagram_file(&root);
        let pad = parts
            .iter()
            .position(|part| part.kind == "membrane-keypad")
            .expect("the preset has a keypad");

        let workdir = std::env::temp_dir().join("croquis-egui-keypad-test");
        let _ = std::fs::create_dir_all(&workdir);
        let firmware = engine::compile(&sketch, &workdir).expect("the sketch compiles");

        let serial: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let pins: Arc<Mutex<Vec<(String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_serial = serial.clone();
        let sink_pins = pins.clone();
        let emit: engine::Emit = Arc::new(move |event| match event {
            engine::SimEvent::Serial(byte) => {
                sink_serial.lock().unwrap().push(byte as char);
            }
            engine::SimEvent::Pin { port, pin, high } => {
                let key = format!("{}{}", port as char, pin);
                sink_pins.lock().unwrap().push((key, high));
            }
            _ => {}
        });
        let sim = engine::Simulator::start(Path::new(&firmware.elf), emit, false)
            .expect("the simulator starts");

        let mut spike = Spike::default();
        spike.parts = parts;
        spike.wires = wires;
        spike.sim = Some(sim);
        spike.selection.select_part(pad);

        // The centre of the first cell.
        let unit = spike.parts[pad].bounds().width() / KEYPAD_VIEW.x;
        let world = spike.parts[pad].bounds().min
            + Vec2::new(
                KEYPAD_COLUMNS[0] + KEYPAD_KEY.x * 0.5,
                KEYPAD_ROWS[0] + KEYPAD_KEY.y * 0.5,
            ) * unit;

        let deadline = Instant::now() + Duration::from_secs(6);
        while Instant::now() < deadline {
            for (key, high) in pins.lock().unwrap().drain(..) {
                spike.avr_pins.insert(key, high);
            }
            // Held down, so the tap never lapses while the sketch scans.
            spike.press_control(pad, world, Pos2::ZERO);
            spike.evaluate_now();
            // Serial arrives a byte at a time, so wait for a whole line.
            {
                let seen = serial.lock().unwrap().clone();
                if let Some(rest) = seen.split("key ").nth(1) {
                    if rest.contains('\n') {
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(mut sim) = spike.sim.take() {
            sim.stop();
        }
        let text = serial.lock().unwrap().clone();
        println!("serial: {text:?}");
        assert!(text.contains("key "), "the keypad should print its key: {text:?}");
        // One key held means one letter printed: anything else is the sketch
        // reading rows that changed underneath it.
        let keys: Vec<char> = text
            .split("key ")
            .skip(1)
            .filter_map(|rest| rest.chars().next())
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        let distinct: std::collections::HashSet<char> = keys.iter().copied().collect();
        assert_eq!(
            distinct.len(),
            1,
            "one key held should print one letter, got {distinct:?} from {keys:?}"
        );
    }

    #[test]
    fn a_bend_is_pinned_while_routing_and_kept_by_the_wire() {
        let mut spike = Spike::default();
        spike.wires.clear();
        let (a, b) = ((0usize, 1usize), (2usize, 3usize));

        // A bend with no wire being drawn means nothing.
        spike.wire_bend(Pos2::new(10.0, 10.0));
        assert!(spike.wire_waypoints.is_empty());

        spike.wire_press(a);
        let start = spike.parts[a.0].pin_world(a.1);
        spike.wire_bend(Pos2::new(100.0, 40.0));
        let bend = spike.wire_waypoints[0];
        assert!(
            bend.x == start.x || bend.y == start.y,
            "a bend is pinned at the corner of the leg, not under the cursor: {bend:?} from {start:?}"
        );

        spike.wire_bend(Pos2::new(140.0, 40.0));
        assert_eq!(spike.wire_waypoints.len(), 2, "each click pins a bend");
        let pinned = spike.wire_waypoints.clone();

        spike.wire_release(Some(b));
        assert_eq!(spike.wires.len(), 1);
        assert_eq!(
            spike.wires[0].waypoints, pinned,
            "the finished wire keeps the route it was given"
        );
        assert!(spike.wire_waypoints.is_empty(), "and the routing state is cleared");
    }

    #[test]
    fn cancelling_gives_up_the_wire_and_its_bends() {
        let mut spike = Spike::default();
        spike.wire_press((0, 1));
        spike.wire_bend(Pos2::new(50.0, 50.0));
        spike.cancel_wire();
        assert_eq!(spike.wire_start, None);
        assert!(spike.wire_waypoints.is_empty());
    }

    #[test]
    fn a_wire_needs_two_different_pins() {
        let mut spike = Spike::default();
        // `default()` loads whatever preset is in the environment.
        spike.wires.clear();
        let (a, b) = ((0usize, 1usize), (2usize, 3usize));

        spike.wire_press(a);
        assert_eq!(spike.wire_start, Some(a), "the first pin arms the wire");

        // Letting go on the same pin is not a wire; it stays armed, which is
        // what makes click-then-click work.
        spike.wire_release(Some(a));
        assert!(spike.wires.is_empty(), "a pin cannot wire to itself");
        assert_eq!(spike.wire_start, Some(a));

        // A second press elsewhere keeps the first pin as the start.
        spike.wire_press(b);
        assert_eq!(spike.wire_start, Some(a));

        spike.wire_release(Some(b));
        assert_eq!(spike.wires.len(), 1);
        assert_eq!((spike.wires[0].a, spike.wires[0].b), (a, b));
        assert_eq!(spike.wire_start, None, "finishing a wire disarms it");
        assert!(spike.reevaluate, "and the circuit is solved again");
    }

    #[test]
    fn a_release_away_from_a_pin_leaves_the_wire_armed() {
        let mut spike = Spike::default();
        spike.wires.clear();
        spike.wire_press((0, 1));
        spike.wire_release(None);
        assert!(spike.wires.is_empty());
        assert_eq!(spike.wire_start, Some((0, 1)), "still waiting for a second pin");
    }

    #[test]
    fn a_press_takes_the_button_and_leaves_it_movable() {
        use croquis_core::catalogue::PartKind;

        // A pressed part is still a part. Buttons and keypads used to take the
        // press and never enter the drag state, so they could not be moved at
        // all — which is what the owner hit.
        let mut spike = Spike::default();
        spike.parts = vec![scene::part_from_kind(
            PartKind::Pushbutton,
            Pos2::new(100.0, 100.0),
            "btn".into(),
        )
        .expect("button")];
        let centre = spike.parts[0].bounds().center();
        spike.press_control(0, centre, Pos2::ZERO);
        assert_eq!(spike.pressed.get("btn"), Some(&true), "the button is down");
        assert_eq!(
            spike.press_drag.map(|(index, _)| index),
            Some(0),
            "and the grab is remembered, so travel can turn it into a drag"
        );
        assert_eq!(spike.dragging, None, "a press alone must not move it");

        let mut pad = Spike::default();
        pad.parts = vec![scene::part_from_kind(
            PartKind::MembraneKeypad,
            Pos2::new(100.0, 100.0),
            "pad".into(),
        )
        .expect("keypad")];
        let centre = pad.parts[0].bounds().center();
        pad.press_control(0, centre, Pos2::ZERO);
        assert_eq!(
            pad.press_drag.map(|(index, _)| index),
            Some(0),
            "a keypad can be moved too — once the pointer travels"
        );
    }

    #[test]
    fn a_press_reaches_the_circuit_and_a_release_lets_go() {
        use croquis_core::catalogue::PartKind;

        // 5V in through a button and a resistor to an LED: no sketch involved,
        // so what lights the LED is the press and nothing else.
        let parts = vec![
            scene::part_from_kind(PartKind::Uno, Pos2::new(0.0, 0.0), "uno".into()).expect("uno"),
            scene::part_from_kind(PartKind::Pushbutton, Pos2::new(300.0, 0.0), "btn".into())
                .expect("button"),
            scene::part_from_kind(PartKind::Resistor, Pos2::new(400.0, 0.0), "r".into())
                .expect("resistor"),
            scene::part_from_kind(PartKind::Led, Pos2::new(500.0, 0.0), "led".into())
                .expect("led"),
        ];
        let at = |id: &str, pin: &str| {
            let part = parts
                .iter()
                .position(|part| part.id == id)
                .unwrap_or_else(|| panic!("no part {id}"));
            let index = parts[part]
                .pins
                .iter()
                .position(|candidate| candidate.name == pin)
                .unwrap_or_else(|| panic!("no pin {id}:{pin}"));
            (part, index)
        };
        let wire = |a: (&str, &str), b: (&str, &str)| {
            let (from, to) = (at(a.0, a.1), at(b.0, b.1));
            scene::Wire {
                a: from,
                b: to,
                waypoints: Vec::new(),
            }
        };
        let wires = vec![
            wire(("uno", "5V"), ("btn", "1.l")),
            wire(("btn", "2.l"), ("r", "1")),
            wire(("r", "2"), ("led", "A")),
            wire(("led", "C"), ("uno", "GND.1")),
        ];

        let mut spike = Spike::default();
        spike.parts = parts;
        spike.wires = wires;
        let lights = |spike: &Spike| {
            matches!(
                spike.led_states.get("led"),
                Some(LedState::Lit) | Some(LedState::Dim) | Some(LedState::Burnt)
            )
        };

        spike.evaluate_now();
        assert!(!lights(&spike), "at rest the LED is dark");

        let centre = spike.parts[1].bounds().center();
        spike.press_control(1, centre, Pos2::ZERO);
        assert!(
            spike.reevaluate,
            "a press changes what the netlist is told, so it must ask for a solve"
        );
        spike.reevaluate = false;
        spike.evaluate_now();
        assert!(lights(&spike), "pressing has to light it");

        spike.release_input();
        assert!(spike.reevaluate, "and letting go must ask for one too");
        spike.reevaluate = false;
        spike.evaluate_now();
        assert!(!lights(&spike), "releasing puts it out");
    }

    #[test]
    fn a_breadboard_joins_only_the_holes_in_one_strip() {
        use croquis_core::catalogue::PartKind;

        // Pin 13 high, through the board, into the LED: lit only when the two
        // holes share a strip. This is the board's whole connectivity model.
        let led_lights = |upper: &str, lower: &str| -> bool {
            let parts = vec![
                scene::part_from_kind(PartKind::Uno, Pos2::new(0.0, 0.0), "uno".into())
                    .expect("uno"),
                scene::part_from_kind(PartKind::Breadboard, Pos2::new(400.0, 0.0), "bb".into())
                    .expect("breadboard"),
                scene::part_from_kind(PartKind::Led, Pos2::new(900.0, 0.0), "led".into())
                    .expect("led"),
            ];
            let at = |id: &str, pin: &str| {
                let part = parts
                    .iter()
                    .position(|part| part.id == id)
                    .unwrap_or_else(|| panic!("no part {id}"));
                let index = parts[part]
                    .pins
                    .iter()
                    .position(|candidate| candidate.name == pin)
                    .unwrap_or_else(|| panic!("no pin {id}:{pin}"));
                (part, index)
            };
            let wire = |a: (&str, &str), b: (&str, &str)| scene::Wire {
                a: at(a.0, a.1),
                b: at(b.0, b.1),
                waypoints: Vec::new(),
            };
            let wires = vec![
                wire(("uno", "13"), ("bb", upper)),
                wire(("bb", lower), ("led", "A")),
                wire(("led", "C"), ("uno", "GND.1")),
            ];
            let avr = HashMap::from([("B5".to_string(), true)]);
            let eval = scene::evaluate_scene(
                &parts,
                &wires,
                &HashMap::new(),
                &avr,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
            );
            // No resistor here, so a connected LED is over-driven rather than
            // merely lit. Both mean the strip carried the current.
            matches!(
                eval.leds.get("led"),
                Some(LedState::Lit) | Some(LedState::Dim) | Some(LedState::Burnt)
            )
        };

        assert!(led_lights("a5", "c5"), "two holes of one strip are one net");
        assert!(
            !led_lights("a5", "f5"),
            "the two banks across the trench are not joined"
        );
        assert!(
            !led_lights("a5", "a9"),
            "another column is a different strip"
        );
    }

    #[test]
    fn a_press_picks_up_the_control_under_it() {
        use croquis_core::catalogue::PartKind;
        let mut spike = Spike::default();
        let button =
            scene::part_from_kind(PartKind::Pushbutton, Pos2::ZERO, "b".into()).expect("button");
        let pad = scene::part_from_kind(PartKind::MembraneKeypad, Pos2::new(400.0, 0.0), "pad".into())
            .expect("pad");
        let stick = scene::part_from_kind(
            PartKind::AnalogJoystick,
            Pos2::new(900.0, 0.0),
            "s".into(),
        )
        .expect("stick");
        let led = scene::part_from_kind(PartKind::Led, Pos2::new(1200.0, 0.0), "led".into())
            .expect("led");
        spike.parts = vec![button, pad, stick, led];

        // A button is held by id, and a release lets go of it.
        spike.press_control(0, Pos2::ZERO, Pos2::ZERO);
        assert_eq!(spike.pressed.get("b"), Some(&true));
        spike.release_input();
        assert!(spike.pressed.is_empty());

        // A keypad taps the key under the pointer, with its minimum hold.
        let bounds = spike.parts[1].bounds();
        let unit = bounds.width() / KEYPAD_VIEW.x;
        let first_key = bounds.min + Vec2::new(12.0, 16.0) * unit;
        spike.press_control(1, first_key, Pos2::ZERO);
        assert_eq!(
            spike.keypad_keys.get("pad"),
            Some(&Some(KeyPosition { row: 0, column: 0 }))
        );
        assert!(spike.keypad_until.contains_key("pad"));

        // The stick is grabbed and its centre button held until it moves.
        let centre = spike.parts[2].bounds().center();
        spike.press_control(2, centre, Pos2::ZERO);
        assert_eq!(spike.joystick, Some(2));
        assert_eq!(spike.pressed.get("s"), Some(&true));

        // Everything else is dragged.
        let led_centre = spike.parts[3].bounds().center();
        spike.press_control(3, led_centre, Pos2::ZERO);
        assert_eq!(spike.dragging, Some(3));
    }

    #[test]
    fn a_held_button_pulls_its_pin_for_the_sketch() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets/button.json");
        let (parts, wires, _) = scene::load_diagram_file(&path);
        let held = HashMap::from([("b1".to_string(), true)]);
        let pressed_low = |pressed: &HashMap<String, bool>| {
            scene::evaluate_scene(
                &parts,
                &wires,
                pressed,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
            )
            .drives
            .iter()
            .any(|drive| drive.port == 'D' && drive.pin == 2 && !drive.high)
        };
        assert!(
            !pressed_low(&HashMap::new()),
            "nothing held: the pull-up stands, so the pin is not pulled low"
        );
        assert!(
            pressed_low(&held),
            "a press is a drive on pin 2, which is what reaches the simulator"
        );
    }

    #[test]
    fn a_keypad_key_is_found_where_the_elements_key_rect_is() {
        use croquis_core::catalogue::PartKind;
        let pad =
            scene::part_from_kind(PartKind::MembraneKeypad, Pos2::ZERO, "pad".into()).expect("pad");
        let bounds = pad.bounds();
        let unit = bounds.width() / KEYPAD_VIEW.x;
        let at = |x: f32, y: f32| bounds.min + Vec2::new(x, y) * unit;

        // The element puts the first key's corner at (7, 10.7) and its rect is
        // 11.2 x 11, so the middle of it is a press.
        assert_eq!(
            keypad_key(&pad, at(12.0, 16.0)),
            Some(KeyPosition { row: 0, column: 0 })
        );
        assert_eq!(
            keypad_key(&pad, at(52.0 + 5.0, 53.6 + 5.0)),
            Some(KeyPosition { row: 3, column: 3 })
        );
        // Column 1 starts at 22, so 20 is the gap between two keys.
        assert_eq!(keypad_key(&pad, at(20.0, 16.0)), None);
        // Outside the grid entirely.
        assert_eq!(keypad_key(&pad, at(2.0, 2.0)), None);
    }

    #[test]
    fn the_joystick_reads_centre_in_the_middle_and_clamps_at_the_edges() {
        use croquis_core::catalogue::PartKind;
        let stick = scene::part_from_kind(PartKind::AnalogJoystick, Pos2::ZERO, "stick".into())
            .expect("joystick");
        let bounds = stick.bounds();
        assert_eq!(joystick_axes(&stick, bounds.center()), (50, 50));
        assert_eq!(
            joystick_axes(&stick, bounds.min),
            (0, 100),
            "top-left is full left, full up"
        );
        assert_eq!(
            joystick_axes(&stick, bounds.max),
            (100, 0),
            "bottom-right is full right, full down"
        );
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
        let eval = scene::evaluate_scene(
            &parts,
            &wires,
            &HashMap::new(),
            &avr,
            &HashMap::new(),
            &pulses,
            &HashMap::new(),
            &HashMap::new(),
            &HashMap::new(),
        );
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
            let eval = scene::evaluate_scene(
                &spike.parts,
                &spike.wires,
                &HashMap::new(),
                &avr,
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
                &HashMap::new(),
            );
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
