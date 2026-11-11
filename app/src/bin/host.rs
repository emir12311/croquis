//! Spike C: host shell around the real engine.
//!
//! Proves the non-webview host path: native open/save dialogs, a background
//! compile/run worker built on the existing Tauri-free engine module, and a
//! native audio tone. The engine is still a path dependency on the old crate;
//! extracting it into its own crate is the Phase 1 step.
//!
//! Run with:
//!
//! ```text
//! cargo run --manifest-path app/Cargo.toml --bin host
//! ```

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use croquis_sim::engine::{self, SimEvent};
use eframe::egui;
use rodio::Source;

const SKETCH: &str = r#"
void setup() {
  Serial.begin(9600);
  pinMode(13, OUTPUT);
}

void loop() {
  digitalWrite(13, HIGH);
  Serial.println("host spike");
  delay(200);
  digitalWrite(13, LOW);
  delay(200);
}
"#;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("Croquis — egui host spike"),
        ..Default::default()
    };
    eframe::run_native(
        "croquis-egui-host-spike",
        options,
        Box::new(|_cc| Ok(Box::new(HostSpike::default()))),
    )
}

struct HostSpike {
    document: String,
    path: Option<PathBuf>,
    log: Vec<String>,
    events: Option<Receiver<String>>,
    busy: bool,
}

impl Default for HostSpike {
    fn default() -> Self {
        Self {
            document: "{\n  \"version\": 1,\n  \"parts\": [],\n  \"connections\": []\n}\n".into(),
            path: None,
            log: Vec::new(),
            events: None,
            busy: false,
        }
    }
}

impl eframe::App for HostSpike {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();

        if let Some(receiver) = &self.events {
            let mut done = false;
            while let Ok(line) = receiver.try_recv() {
                if line == "__done__" {
                    done = true;
                } else {
                    self.log.push(line);
                }
            }
            if self.log.len() > 300 {
                let excess = self.log.len() - 300;
                self.log.drain(0..excess);
            }
            if done {
                self.events = None;
                self.busy = false;
            }
        }
        if self.busy {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        egui::Panel::right("host-spike-controls")
            .exact_size(290.0)
            .show(root, |ui| {
                ui.heading("egui host spike");
                ui.label("Native dialogs, engine worker, audio.");
                ui.separator();

                if ui.button("Open circuit…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Croquis circuit", &["json"])
                        .pick_file()
                    {
                        match std::fs::read_to_string(&path) {
                            Ok(text) => {
                                self.document = text;
                                self.path = Some(path.clone());
                                self.log.push(format!("opened {}", path.display()));
                            }
                            Err(error) => self.log.push(format!("open failed: {error}")),
                        }
                    }
                }
                if ui.button("Save circuit…").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Croquis circuit", &["json"])
                        .set_file_name("diagram.json")
                        .save_file()
                    {
                        match std::fs::write(&path, &self.document) {
                            Ok(()) => {
                                self.path = Some(path.clone());
                                self.log.push(format!("saved {}", path.display()));
                            }
                            Err(error) => self.log.push(format!("save failed: {error}")),
                        }
                    }
                }
                if let Some(path) = &self.path {
                    ui.label(format!("Path: {}", path.display()));
                }

                ui.separator();
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Compile & run engine"))
                    .clicked()
                {
                    self.start_engine();
                }
                if ui.button("Beep 440 Hz").clicked() {
                    beep();
                }
                if ui.button("Clear log").clicked() {
                    self.log.clear();
                }
                ui.separator();
                ui.label(if self.busy {
                    "Engine: running…"
                } else {
                    "Engine: idle"
                });
                ui.label("The worker uses the real avr-gcc/simavr path.");
            });

        egui::CentralPanel::default().show(root, |ui| {
            ui.label("Document");
            egui::ScrollArea::vertical()
                .id_salt("host-document")
                .max_height(260.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.document)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .desired_rows(10),
                    );
                });
            ui.separator();
            ui.label("Engine log");
            egui::ScrollArea::vertical()
                .id_salt("host-log")
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.log {
                        ui.monospace(line);
                    }
                });
        });
    }
}

impl HostSpike {
    fn start_engine(&mut self) {
        let (sender, receiver) = mpsc::channel();
        self.events = Some(receiver);
        self.busy = true;
        self.log.push("starting engine worker".into());
        thread::spawn(move || {
            run_engine(sender);
        });
    }
}

fn run_engine(sender: mpsc::Sender<String>) {
    let workdir = std::env::temp_dir().join(format!("croquis-host-spike-{}", std::process::id()));
    if let Err(error) = std::fs::create_dir_all(&workdir) {
        let _ = sender.send(format!("workdir failed: {error}"));
        let _ = sender.send("__done__".into());
        return;
    }

    let _ = sender.send("compiling sketch…".into());
    let firmware = match engine::compile(SKETCH, &workdir) {
        Ok(firmware) => firmware,
        Err(error) => {
            let _ = sender.send(format!("compile failed: {error}"));
            let _ = sender.send("__done__".into());
            return;
        }
    };
    let _ = sender.send(format!(
        "compiled: {} flash bytes, {} SRAM bytes",
        firmware.flash_bytes, firmware.sram_bytes
    ));

    let event_sender = sender.clone();
    let emit: engine::Emit = Arc::new(move |event| {
        let _ = event_sender.send(format_event(&event));
    });
    let mut simulator = match engine::Simulator::start(Path::new(&firmware.elf), emit, false) {
        Ok(simulator) => simulator,
        Err(error) => {
            let _ = sender.send(format!("simulation failed: {error}"));
            let _ = sender.send("__done__".into());
            return;
        }
    };
    let _ = sender.send("simulation started (1.2 s)".into());
    thread::sleep(Duration::from_millis(1200));
    simulator.stop();
    let _ = sender.send("simulation stopped".into());
    let _ = sender.send("__done__".into());
}

fn format_event(event: &SimEvent) -> String {
    match event {
        SimEvent::Pin { port, pin, high } => {
            format!(
                "pin {}{} -> {}",
                *port as char,
                pin,
                if *high { "HIGH" } else { "LOW" }
            )
        }
        SimEvent::Serial(byte) => format!("serial: {}", *byte as char),
        SimEvent::Timing {
            port,
            pin,
            duty_per_mille,
            high_micros,
            period_micros,
        } => format!(
            "timing {}{} duty={}‰ high={}µs period={}µs",
            *port as char, pin, duty_per_mille, high_micros, period_micros
        ),
    }
}

fn beep() {
    thread::spawn(|| {
        let sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(sink) => sink,
            Err(error) => {
                eprintln!("audio unavailable: {error}");
                return;
            }
        };
        let player = rodio::Player::connect_new(sink.mixer());
        player.append(
            rodio::source::SineWave::new(440.0)
                .take_duration(Duration::from_millis(180))
                .amplify(0.04),
        );
        player.sleep_until_end();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_pin_and_serial_events() {
        let pin = SimEvent::Pin {
            port: b'B',
            pin: 5,
            high: true,
        };
        assert_eq!(format_event(&pin), "pin B5 -> HIGH");
        assert_eq!(format_event(&SimEvent::Serial(b'x')), "serial: x");
    }

    #[test]
    #[ignore = "needs the AVR toolchain at the repository root"]
    fn compiles_a_sketch_through_the_host_api() {
        let workdir =
            std::env::temp_dir().join(format!("croquis-host-sketch-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let firmware = engine::compile(SKETCH, &workdir).expect("the host must compile a sketch");
        assert!(firmware.flash_bytes > 0);
        let _ = std::fs::remove_dir_all(&workdir);
    }
}
