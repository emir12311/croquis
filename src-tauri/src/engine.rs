//! The simulation engine: compile an Arduino sketch with `avr-gcc`, then run the
//! resulting firmware on **simavr**.
//!
//! Everything lives under a git-ignored `.toolchain/` folder (see the README's
//! "Development notes"): the Arduino AVR toolchain, the Arduino core, and simavr.

use std::ffi::CString;
use std::os::raw::c_void;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::timing::PinTiming;

#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code,
    clippy::all
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/simavr_bindings.rs"));
}

extern "C" {
    fn sim_shim_cycle(avr: *mut ffi::avr_t) -> u64;
    fn sim_shim_set_freq(avr: *mut ffi::avr_t, freq: u32);
    /// Set VCC/AVCC/AREF together, in millivolts (a 5V Uno).
    fn sim_shim_set_supply(avr: *mut ffi::avr_t, millivolts: u32);
}

/// Mirrors simavr's `AVR_IOCTL_DEF` macro (function-like, so bindgen skips it):
/// `((a << 24) | (b << 16) | (c << 8) | d)`.
const fn ioctl(a: u8, b: u8, c: u8, d: u32) -> u32 {
    ((a as u32) << 24) | ((b as u32) << 16) | ((c as u32) << 8) | (d & 0xff)
}
const fn ioport_getirq(name: u8) -> u32 {
    ioctl(b'i', b'o', b'g', name as u32)
}
const fn uart_getirq(name: u8) -> u32 {
    ioctl(b'u', b'a', b'r', name as u32)
}
/// The ADC module's irq table (`AVR_IOCTL_DEF('a','d','c','0')`). Its channels are
/// the header's `ADC_IRQ_ADC0..15`, so the IRQ index is the channel number.
const fn adc_getirq() -> u32 {
    ioctl(b'a', b'd', b'c', b'0' as u32)
}

/// The Uno's chip.
const MCU: &str = "atmega328p";
const F_CPU: u32 = 16_000_000;
/// The Uno's supply and analogue reference, in millivolts. What the ADC measures
/// against, and so the full-scale value `analogRead` divides by.
const SUPPLY_MV: u32 = 5000;

// ---- paths ----------------------------------------------------------------

fn workspace_root() -> PathBuf {
    // Baked at build time (the repo root, i.e. `src-tauri/..`). Development builds
    // resolve `.toolchain/` from here; an installed build is found by `toolchain()`
    // beside the binary or in `../lib/<product>/toolchain`.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri has a parent")
        .to_path_buf()
}
fn toolchain() -> PathBuf {
    // Shipped first, development second.
    //
    // An installed Croquis is not in a source tree, so the old `workspace_root()` lookup
    // found nothing and the app launched, wired, and then failed to compile anything. The
    // toolchain now ships as a bundle resource: beside the binary for the AppImage, and in
    // ../lib/<app>/ for the deb and rpm. Checking for avr-gcc rather than for the folder
    // means a half-copied tree is rejected instead of silently used.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Tauri names the resource directory after the product name, capital C and all.
            // Both cases are checked because a lookup that is one character off fails
            // silently, which is the exact bug this function exists to fix.
            let candidates = [
                dir.join("toolchain"),
                dir.join("../lib/Croquis/toolchain"),
                dir.join("../lib/croquis/toolchain"),
            ];
            for candidate in candidates {
                if candidate.join("avr/bin/avr-gcc").exists() {
                    return candidate;
                }
            }
        }
    }
    workspace_root().join(".toolchain")
}
fn avr_bin() -> PathBuf {
    toolchain().join("avr/bin")
}
fn core_dir() -> PathBuf {
    toolchain().join("ArduinoCore-avr-1.8.6")
}

// ---- compiling ------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize)]
pub struct Firmware {
    pub flash_bytes: u32,
    pub sram_bytes: u32,
    pub elf: String,
}

/// Compile `sketch` (Arduino source) and return the firmware. The `.elf` is
/// written inside `workdir`; a cache of the Arduino core objects lives beside
/// the toolchain so repeat compiles are quick.
pub fn compile(sketch: &str, workdir: &Path) -> Result<Firmware, String> {
    let bin = avr_bin();
    let core = core_dir();
    let coresrc = core.join("cores/arduino");
    let variant = core.join("variants/standard");
    let toolroot = toolchain();
    let cache = core_cache(&toolroot)?;

    let objdir = workdir.join("obj");
    std::fs::create_dir_all(&objdir).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;

    let base = format!(
        "-mmcu={mcu} -DF_CPU={f}L -DARDUINO=10819 -DARDUINO_AVR_UNO -DARDUINO_ARCH_AVR \
         -Os -ffunction-sections -fdata-sections -I{} -I{}",
        coresrc.display(),
        variant.display(),
        mcu = MCU,
        f = F_CPU,
    );

    let core_objs = compile_core(&bin, &coresrc, &base, &cache)?;

    // The sketch, prefixed the way the Arduino IDE does.
    let sketch_cpp = workdir.join("sketch.cpp");
    std::fs::write(
        &sketch_cpp,
        format!("#include <Arduino.h>\n#line 1 \"sketch.ino\"\n{sketch}"),
    )
    .map_err(|e| e.to_string())?;
    let sketch_obj = objdir.join("sketch.cpp.o");
    run_flags(
        &bin.join("avr-g++"),
        &base,
        &[
            "-c",
            sketch_cpp.to_str().unwrap(),
            "-o",
            sketch_obj.to_str().unwrap(),
        ],
    )?;

    // Link, then convert to Intel HEX.
    let elf = workdir.join("sketch.elf");
    let hex = workdir.join("sketch.hex");
    let mut link: Vec<String> = base.split_whitespace().map(String::from).collect();
    link.push("-Wl,--gc-sections".into());
    link.push("-o".into());
    link.push(elf.to_string_lossy().into());
    for o in &core_objs {
        link.push(o.to_string_lossy().into());
    }
    link.push(sketch_obj.to_string_lossy().into());
    link.push("-lm".into());
    let link_refs: Vec<&str> = link.iter().map(String::as_str).collect();
    run(&bin.join("avr-gcc"), &link_refs)?;
    run(
        &bin.join("avr-objcopy"),
        &[
            "-O",
            "ihex",
            "-R",
            ".eeprom",
            elf.to_str().unwrap(),
            hex.to_str().unwrap(),
        ],
    )?;

    let (flash_bytes, sram_bytes) = match Command::new(bin.join("avr-size"))
        .arg(elf.to_str().unwrap())
        .output()
    {
        Ok(out) => parse_size(&String::from_utf8_lossy(&out.stdout)),
        Err(_) => (0, 0),
    };

    Ok(Firmware {
        flash_bytes,
        sram_bytes,
        elf: elf.to_string_lossy().into(),
    })
}

/// Can this process create files in `dir`?
fn writable(dir: &Path) -> bool {
    let probe = dir.join(".write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Where the compiled core objects go.
///
/// A packaged Croquis keeps its toolchain read-only: an AppImage mount, or `/usr/lib` owned by
/// root. Writing a missing object in there fails with "Read-only file system" and takes the
/// whole compile with it — which is what happened the first time someone installed a package,
/// because the shipped cache had only the .cpp objects in it.
///
/// The shipped cache is used when it can be written to. When it cannot, the objects are copied
/// into a cache this process owns, so only whatever is genuinely missing gets compiled.
fn core_cache(toolroot: &Path) -> Result<PathBuf, String> {
    let shipped = toolroot.join("core-cache");
    if writable(&shipped) {
        return Ok(shipped);
    }
    let fallback = std::env::temp_dir().join("croquis-core-cache");
    std::fs::create_dir_all(&fallback).map_err(|e| e.to_string())?;
    if let Ok(entries) = std::fs::read_dir(&shipped) {
        for entry in entries.flatten() {
            let target = fallback.join(entry.file_name());
            if !target.exists() {
                // Copy beside the target and rename, for the same reason the core is
                // compiled that way: another process must not see a partial object.
                let tmp = fallback.join(format!(
                    ".{}.tmp-{}",
                    entry.file_name().to_string_lossy(),
                    std::process::id()
                ));
                if std::fs::copy(entry.path(), &tmp).is_ok() {
                    let _ = std::fs::rename(&tmp, &target);
                }
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }
    Ok(fallback)
}

fn compile_core(
    bin: &Path,
    coresrc: &Path,
    base: &str,
    cache: &Path,
) -> Result<Vec<PathBuf>, String> {
    let mut sources: Vec<PathBuf> = std::fs::read_dir(coresrc)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("c") | Some("cpp") | Some("S")
            )
        })
        // hooks.c aliases yield() to a `static` __empty, which this toolchain refuses to
        // compile at all. The shipped cache supplies yield.c.o instead; see
        // scripts/setup-toolchain.sh.
        .filter(|p| p.file_name().and_then(|n| n.to_str()) != Some("hooks.c"))
        .collect();
    sources.sort();

    let mut objs = Vec::new();
    for src in sources {
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        let obj = cache.join(format!("{name}.o"));
        if !obj.exists() {
            let cc = if name.ends_with(".c") {
                "avr-gcc"
            } else {
                "avr-g++"
            };
            // Compile to a private name and rename, so a second instance sharing the
            // cache can never link against a half-written object.
            let tmp = cache.join(format!(".{name}.o.tmp-{}", std::process::id()));
            run_flags(
                &bin.join(cc),
                base,
                &["-c", src.to_str().unwrap(), "-o", tmp.to_str().unwrap()],
            )?;
            if let Err(e) = std::fs::rename(&tmp, &obj) {
                let _ = std::fs::remove_file(&tmp);
                return Err(e.to_string());
            }
        }
        objs.push(obj);
    }

    // Every object in the cache, not just the ones derived from a core source: the shipped
    // cache also carries yield.o, which nothing in the core directory produces.
    if let Ok(entries) = std::fs::read_dir(cache) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("o") && !objs.contains(&path) {
                objs.push(path);
            }
        }
    }
    Ok(objs)
}

/// Run a program with a whitespace-split flag string plus extra args.
fn run_flags(program: &Path, base: &str, extra: &[&str]) -> Result<(), String> {
    let mut args: Vec<&str> = base.split_whitespace().collect();
    args.extend_from_slice(extra);
    run(program, &args)
}

fn run(program: &Path, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{}: {e}", program.display()))?;
    if out.status.success() {
        return Ok(());
    }
    let mut msg = String::from_utf8_lossy(&out.stderr).to_string();
    if msg.trim().is_empty() {
        msg = String::from_utf8_lossy(&out.stdout).to_string();
    }
    Err(msg.trim().to_string())
}

/// Parse `avr-size` output → (flash bytes, sram bytes).
fn parse_size(text: &str) -> (u32, u32) {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let _header = lines.next();
    let mut flash = 0;
    let mut sram = 0;
    if let Some(row) = lines.next() {
        let cols: Vec<&str> = row.split_whitespace().collect();
        if cols.len() >= 3 {
            let text: u32 = cols[0].parse().unwrap_or(0);
            let data: u32 = cols[1].parse().unwrap_or(0);
            let bss: u32 = cols[2].parse().unwrap_or(0);
            flash = text + data;
            sram = data + bss;
        }
    }
    (flash, sram)
}

// ---- simulating -----------------------------------------------------------

pub enum SimEvent {
    /// A change on an I/O pin (port byte, pin 0-7). Port `b'B'` pin 5 is the
    /// Uno's built-in LED, Arduino pin 13.
    Pin { port: u8, pin: u8, high: bool },
    /// A byte written to the UART — the serial monitor.
    Serial(u8),
    /// The latest PWM timing for a pin, emitted on a 20 ms tick.
    Timing {
        port: u8,
        pin: u8,
        duty_per_mille: u16,
        high_micros: u64,
        period_micros: u64,
    },
}

pub type Emit = Arc<dyn Fn(SimEvent) + Send + Sync>;

pub struct Simulator {
    input: Sender<(u8, u8, bool)>,
    /// ADC channel and the voltage to hold on it, in millivolts.
    analog: Sender<(u8, u16)>,
    /// Bytes to feed into the simulated UART's receive side.
    serial: Sender<u8>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Simulator {
    /// Load `elf` and start running it, calling `emit` for pin changes and serial.
    /// With `bounce`, a pressed switch chatters the way a real contact does; without
    /// it the level changes cleanly, the way firmware wishes switches behaved.
    pub fn start(elf: &Path, emit: Emit, bounce: bool) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (analog_tx, analog_rx) = mpsc::channel();
        let (serial_tx, serial_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let elf = elf.to_path_buf();
        let stop_thread = stop.clone();
        let handle = thread::spawn(move || {
            if let Err(e) = sim_main(
                &elf,
                rx,
                analog_rx,
                serial_rx,
                stop_thread,
                emit,
                &ready_tx,
                bounce,
            ) {
                let _ = ready_tx.send(Err(e));
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                input: tx,
                analog: analog_tx,
                serial: serial_tx,
                stop,
                handle: Some(handle),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("simulation thread exited early".into()),
        }
    }

    /// Drive an input pin — a pressed button, a held line, etc.
    pub fn set_pin(&self, port: u8, pin: u8, high: bool) {
        let _ = self.input.send((port, pin, high));
    }

    /// Hold a voltage on an ADC channel — a potentiometer's wiper, in millivolts.
    pub fn set_analog(&self, channel: u8, millivolts: u16) {
        let _ = self.analog.send((channel, millivolts));
    }

    /// Feed bytes into the simulated UART's receive side — serial monitor input.
    pub fn send_serial(&self, bytes: &[u8]) {
        for byte in bytes {
            let _ = self.serial.send(*byte);
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.stop();
    }
}

struct PinCtx {
    emit: Emit,
    port: u8,
    pin: u8,
    /// The simulator, as an address: the callback runs on the sim thread and only
    /// needs this to read the cycle counter.
    avr: usize,
    timing: Mutex<PinTiming>,
}

unsafe extern "C" fn pin_notify(_irq: *mut ffi::avr_irq_t, value: u32, param: *mut c_void) {
    let ctx = &*(param as *const PinCtx);
    let high = value != 0;
    let now = sim_shim_cycle(ctx.avr as *mut ffi::avr_t);
    if let Ok(mut timing) = ctx.timing.lock() {
        timing.edge(now, high);
    }
    (ctx.emit)(SimEvent::Pin {
        port: ctx.port,
        pin: ctx.pin,
        high,
    });
}

unsafe extern "C" fn uart_notify(_irq: *mut ffi::avr_irq_t, value: u32, param: *mut c_void) {
    let emit = &*(param as *const Emit);
    if value <= 0xff {
        emit(SimEvent::Serial(value as u8));
    }
}

fn sim_main(
    elf: &Path,
    input: Receiver<(u8, u8, bool)>,
    analog: Receiver<(u8, u16)>,
    serial: Receiver<u8>,
    stop: Arc<AtomicBool>,
    emit: Emit,
    ready: &Sender<Result<(), String>>,
    bounce: bool,
) -> Result<(), String> {
    unsafe {
        let name = CString::new(MCU).unwrap();
        let avr = ffi::avr_make_mcu_by_name(name.as_ptr());
        if avr.is_null() {
            return Err(format!("unknown MCU `{MCU}`"));
        }

        let mut fw: Box<ffi::elf_firmware_t> = Box::new(std::mem::zeroed());
        let path = CString::new(elf.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        if ffi::elf_read_firmware(path.as_ptr(), &mut *fw) != 0 {
            return Err(format!("could not read firmware {}", elf.display()));
        }
        ffi::avr_init(avr);
        ffi::avr_load_firmware(avr, &mut *fw);
        // simavr defaults the clock to 1 MHz; our firmware is built for 16 MHz.
        sim_shim_set_freq(avr, F_CPU);
        // The Uno runs at 5V, and analogRead defaults to measuring against AVCC.
        // simavr starts with no supply set, so it silently measures against 3.3V.
        sim_shim_set_supply(avr, SUPPLY_MV);
        ffi::avr_reset(avr);

        // Watch every pin of ports B/C/D (covers the built-in LED and the common
        // digital pins), plus the UART output. The contexts are leaked on purpose:
        // simavr keeps the pointers for the lifetime of the simulation.
        let mut meters: Vec<*mut PinCtx> = Vec::new();
        for port in [b'B', b'C', b'D'] {
            for pin in 0u8..8 {
                let irq = ffi::avr_io_getirq(avr, ioport_getirq(port), pin as i32);
                if !irq.is_null() {
                    let ctx = Box::into_raw(Box::new(PinCtx {
                        emit: emit.clone(),
                        port,
                        pin,
                        avr: avr as usize,
                        timing: Mutex::new(PinTiming::default()),
                    }));
                    meters.push(ctx);
                    ffi::avr_irq_register_notify(irq, Some(pin_notify), ctx as *mut c_void);
                }
            }
        }
        let uart = ffi::avr_io_getirq(avr, uart_getirq(b'0'), ffi::UART_IRQ_OUTPUT as i32);
        if !uart.is_null() {
            let ctx = Box::into_raw(Box::new(emit.clone()));
            ffi::avr_irq_register_notify(uart, Some(uart_notify), ctx as *mut c_void);
        }
        // The receive side: raising this IRQ with a byte is simavr's UART input.
        let uart_in = ffi::avr_io_getirq(avr, uart_getirq(b'0'), ffi::UART_IRQ_INPUT as i32);

        let freq = F_CPU as f64;
        let start_cycles = sim_shim_cycle(avr);
        let start = Instant::now();
        let _ = ready.send(Ok(()));

        /// Push a level onto a pin.
        unsafe fn raise_pin(avr: *mut ffi::avr_t, port: u8, pin: u8, high: bool) {
            let irq = ffi::avr_io_getirq(avr, ioport_getirq(port), pin as i32);
            if !irq.is_null() {
                ffi::avr_raise_irq(irq, high as u32);
            }
        }

        /// Hold a voltage on an ADC channel. simavr records the value and reads it
        /// at the next conversion, so it can be set at any time and persists.
        unsafe fn raise_adc(avr: *mut ffi::avr_t, channel: u8, millivolts: u16) {
            let irq = ffi::avr_io_getirq(
                avr,
                adc_getirq(),
                ffi::ADC_IRQ_ADC0 as i32 + channel as i32,
            );
            if !irq.is_null() {
                ffi::avr_raise_irq(irq, millivolts as u32);
            }
        }

        // Contact bounce. A real switch doesn't change level cleanly: it chatters
        // for a millisecond or so, and firmware debounces it away. Expressed in
        // *simulated* cycles, so it's identical however fast the machine is, and a
        // sketch that doesn't debounce will really see it.
        const BOUNCE_CHATTER_CYCLES: u64 = F_CPU as u64 / 5500; // ~180us between make/breaks
        const BOUNCE_FLIPS: u64 = 6; // ~1ms of chatter in all
        // One timing event per pin per 20 ms: enough for fades and servo pulses,
        // far below the edge rate of a 490 Hz PWM.
        const TIMING_TICK: u64 = F_CPU as u64 / 50;
        // A pin that stops toggling is no longer PWM; stop repeating its timing.
        const TIMING_STALE: u64 = F_CPU as u64 / 10;
        struct Bounce {
            port: u8,
            pin: u8,
            target: bool,
            flips: u64,
            next: u64,
        }
        let mut bounces: Vec<Bounce> = Vec::new();
        let mut next_timing = sim_shim_cycle(avr) + TIMING_TICK;

        loop {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            while let Ok((port, pin, high)) = input.try_recv() {
                // Keep only the newest request per pin.
                bounces.retain(|b| !(b.port == port && b.pin == pin));
                if bounce {
                    // Let the contact chatter its way to the new level.
                    bounces.push(Bounce {
                        port,
                        pin,
                        target: high,
                        flips: BOUNCE_FLIPS,
                        next: sim_shim_cycle(avr),
                    });
                } else {
                    // A clean change, straight to the level asked for.
                    raise_pin(avr, port, pin, high);
                }
            }
            // Analogue inputs are not momentary: whatever the circuit last said is
            // the voltage that stays on the channel until it says something else.
            while let Ok((channel, millivolts)) = analog.try_recv() {
                raise_adc(avr, channel, millivolts);
            }
            // Serial input is a byte stream, like the monitor's Send button: each
            // byte is pushed into the UART's receive side.
            while let Ok(byte) = serial.try_recv() {
                if !uart_in.is_null() {
                    ffi::avr_raise_irq(uart_in, byte as u32);
                }
            }
            let state = ffi::avr_run(avr);
            // Settle the contacts that are still chattering.
            if !bounces.is_empty() {
                let now = sim_shim_cycle(avr);
                bounces.retain_mut(|b| {
                    while now >= b.next && b.flips > 0 {
                        b.flips -= 1;
                        // Lands on `target` first, then opens, then closes again…
                        let value = if b.flips % 2 == 0 { !b.target } else { b.target };
                        raise_pin(avr, b.port, b.pin, value);
                        b.next += BOUNCE_CHATTER_CYCLES;
                    }
                    if b.flips == 0 {
                        raise_pin(avr, b.port, b.pin, b.target);
                        return false;
                    }
                    true
                });
            }
            // A 20 ms cadence for PWM timing. Each meter is updated by the pin
            // callback; this is where it becomes one UI event instead of an edge
            // flood.
            let now = sim_shim_cycle(avr);
            if now >= next_timing {
                for &ctx in &meters {
                    let timing = match (*ctx).timing.lock() {
                        Ok(timing) => {
                            let recent = timing
                                .last_edge()
                                .map(|edge| now.saturating_sub(edge) <= TIMING_STALE)
                                .unwrap_or(false);
                            if recent {
                                timing.timing()
                            } else {
                                None
                            }
                        }
                        Err(_) => None,
                    };
                    if let Some(timing) = timing {
                        (emit)(SimEvent::Timing {
                            port: (*ctx).port,
                            pin: (*ctx).pin,
                            duty_per_mille: timing.duty_per_mille,
                            high_micros: timing.high_micros(F_CPU as u64),
                            period_micros: timing.period_micros(F_CPU as u64),
                        });
                    }
                }
                next_timing = now + TIMING_TICK;
            }
            if state == ffi::cpu_Done as i32 || state == ffi::cpu_Crashed as i32 {
                break;
            }
            // Pace to real time so `delay(500)` really is half a second.
            let sim_secs = (sim_shim_cycle(avr) - start_cycles) as f64 / freq;
            let real = start.elapsed().as_secs_f64();
            if sim_secs > real {
                let d = (sim_secs - real).min(0.05);
                if d > 0.0005 {
                    thread::sleep(Duration::from_secs_f64(d));
                }
            }
        }
        ffi::avr_terminate(avr);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A real switch chatters when it changes. A sketch with no debounce sees it;
    /// one with a debounce doesn't. The two sketches differ only in that delay.
    #[test]
    fn a_button_bounces_unless_the_sketch_debounces() {
        let dir = std::env::temp_dir().join("croquis-bounce-test");
        let _ = std::fs::remove_dir_all(&dir);

        let sketch = |debounce_ms: u32| {
            format!(
                r#"
const int PIN = 2;
int last;
unsigned long lastChange;
int changes;

void setup() {{
  pinMode(PIN, INPUT_PULLUP);
  Serial.begin(9600);
  last = digitalRead(PIN);
}}

void loop() {{
  int now = digitalRead(PIN);
  if (now != last && (millis() - lastChange) >= {debounce_ms}) {{
    changes++;
    lastChange = millis();
    last = now;
    Serial.print("change ");
    Serial.println(changes);
  }}
}}
"#
            )
        };

        // Press, hold, release — and count how many changes the sketch reported.
        let press_and_release = |fw: &Firmware, bounce: bool| -> usize {
            let events: Arc<Mutex<Vec<SimEvent>>> = Arc::new(Mutex::new(Vec::new()));
            let sink = events.clone();
            let emit: Emit = Arc::new(move |e| sink.lock().unwrap().push(e));
            let mut sim =
                Simulator::start(Path::new(&fw.elf), emit, bounce).expect("sim should start");
            thread::sleep(Duration::from_millis(200)); // let setup() finish
            sim.set_pin(b'D', 2, false); // press: INPUT_PULLUP, so low is held
            thread::sleep(Duration::from_millis(150));
            sim.set_pin(b'D', 2, true); // release
            thread::sleep(Duration::from_millis(150));
            sim.stop();
            let ev = events.lock().unwrap();
            let text: String = ev
                .iter()
                .filter_map(|e| match e {
                    SimEvent::Serial(b) => Some(*b as char),
                    _ => None,
                })
                .collect();
            text.matches("change").count()
        };

        let raw_dir = dir.join("raw");
        let deb_dir = dir.join("debounced");
        let raw = compile(&sketch(0), &raw_dir).expect("sketch should compile");
        let debounced = compile(&sketch(5), &deb_dir).expect("sketch should compile");

        let bounced = press_and_release(&raw, true);
        let clean = press_and_release(&debounced, true);
        let switched_off = press_and_release(&raw, false);
        assert!(
            bounced >= 3,
            "an undebounced sketch should see the chatter (saw {bounced} changes)"
        );
        assert_eq!(
            clean, 2,
            "a debounced sketch should see one press and one release (saw {clean})"
        );
        assert_eq!(
            switched_off, 2,
            "with bounce switched off even a raw sketch should see one press and one release \
             (saw {switched_off}) — that is what the Run menu's toggle turns off"
        );
    }

    /// Every preset we ship has to build. A preset that does not compile makes
    /// "press Run and see it work" a lie, which is the whole point of shipping it.
    #[test]
    fn every_shipped_preset_compiles() {
        let dir = std::env::temp_dir().join("croquis-preset-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let presets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../presets");
        let mut checked = 0;
        let mut names: Vec<String> = std::fs::read_dir(&presets)
            .expect("presets/ should sit next to src-tauri/")
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert!(!names.is_empty(), "no presets found in {}", presets.display());

        for name in &names {
            let path = Path::new(name);
            let text = std::fs::read_to_string(path).unwrap();
            let doc: serde_json::Value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()));
            let sketch = doc["sketch"]
                .as_str()
                .unwrap_or_else(|| panic!("{} carries no sketch", path.display()));
            // The sketch must reference the parts, or the file is a drawing rather
            // than a circuit: every connection names a part id that has to exist.
            let ids: Vec<&str> = doc["parts"]
                .as_array()
                .unwrap_or_else(|| panic!("{} has no parts", path.display()))
                .iter()
                .filter_map(|p| p["id"].as_str())
                .collect();
            for connection in doc["connections"].as_array().unwrap_or(&Vec::new()) {
                for end in connection.as_array().unwrap_or(&Vec::new()).iter().take(2) {
                    let reference = end.as_str().unwrap_or_default();
                    let id = reference.split(':').next().unwrap_or_default();
                    assert!(
                        ids.contains(&id),
                        "{} wires {reference}, but no part has that id",
                        path.display()
                    );
                }
            }
            let out = dir.join(path.file_stem().unwrap().to_str().unwrap());
            let fw = compile(sketch, &out)
                .unwrap_or_else(|e| panic!("{}: {e}", path.file_name().unwrap().to_string_lossy()));
            assert!(fw.flash_bytes > 500, "{} built nothing", path.display());
            checked += 1;
        }
        assert!(checked >= 3, "expected three or more presets, built {checked}");
    }

    #[test]
    fn compiles_and_runs_a_blinking_sketch() {
        let dir = std::env::temp_dir().join("croquis-engine-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let sketch = r#"
void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
  Serial.println("hello");
}
void loop() {
  digitalWrite(LED_BUILTIN, HIGH); delay(100);
  digitalWrite(LED_BUILTIN, LOW);  delay(100);
}
"#;
        let fw = compile(sketch, &dir).expect("sketch should compile");
        assert!(fw.flash_bytes > 500, "flash was {}", fw.flash_bytes);

        let events: Arc<Mutex<Vec<SimEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: Emit = Arc::new(move |e| sink.lock().unwrap().push(e));

        let mut sim = Simulator::start(Path::new(&fw.elf), emit, false).expect("sim should start");
        thread::sleep(Duration::from_millis(900));
        sim.stop();

        let ev = events.lock().unwrap();
        let highs = ev
            .iter()
            .filter(|e| matches!(e, SimEvent::Pin { port: b'B', pin: 5, high: true }))
            .count();
        let lows = ev
            .iter()
            .filter(|e| matches!(e, SimEvent::Pin { port: b'B', pin: 5, high: false }))
            .count();
        let serial: String = ev
            .iter()
            .filter_map(|e| match e {
                SimEvent::Serial(b) => Some(*b as char),
                _ => None,
            })
            .collect();

        assert!(highs >= 2, "pin 13 never went high ({highs})");
        assert!(lows >= 2, "pin 13 never went low ({lows})");
        assert!(serial.contains("hello"), "serial was {serial:?}");
    }

    /// The analogue path end to end: a sketch that reports `analogRead(A0)` should
    /// print the value a fed voltage scales to, against the Uno's 5V reference.
    #[test]
    fn an_adc_voltage_reaches_analog_read() {
        let dir = std::env::temp_dir().join("croquis-analog-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let sketch = r#"
void setup() {
  Serial.begin(9600);
}
void loop() {
  Serial.println(analogRead(A0));
  delay(200);
}
"#;
        let fw = compile(sketch, &dir).expect("sketch should compile");

        let events: Arc<Mutex<Vec<SimEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: Emit = Arc::new(move |e| sink.lock().unwrap().push(e));
        let mut sim = Simulator::start(Path::new(&fw.elf), emit, false).expect("sim should start");
        // Half of the 5 V supply on A0. Against a 5 V reference that is 511/512;
        // against simavr's un-set 3.3 V default it would read ~776, so this also
        // proves the supply was configured.
        sim.set_analog(0, 2502);
        thread::sleep(Duration::from_millis(700));
        sim.stop();

        let ev = events.lock().unwrap();
        let serial: String = ev
            .iter()
            .filter_map(|e| match e {
                SimEvent::Serial(b) => Some(*b as char),
                _ => None,
            })
            .collect();
        let readings: Vec<i32> = serial.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        let last = *readings.last().unwrap_or_else(|| panic!("no analogRead output: {serial:?}"));
        assert!(
            (500..=525).contains(&last),
            "2502 mV should read ~512 against a 5 V reference, got {last} (all: {readings:?})"
        );
    }

    /// The receive side of the serial monitor: bytes fed into the UART must reach
    /// `Serial.read()`, not just appear in the output pane.
    #[test]
    fn serial_input_reaches_serial_read() {
        let dir = std::env::temp_dir().join("croquis-uart-input-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let sketch = r#"
void setup() {
  Serial.begin(9600);
}
void loop() {
  if (Serial.available()) {
    Serial.write(Serial.read());
  }
}
"#;
        let fw = compile(sketch, &dir).expect("sketch should compile");

        let events: Arc<Mutex<Vec<SimEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: Emit = Arc::new(move |e| sink.lock().unwrap().push(e));
        let mut sim = Simulator::start(Path::new(&fw.elf), emit, false).expect("sim should start");
        thread::sleep(Duration::from_millis(200)); // let setup() finish
        sim.send_serial(b"hi");
        thread::sleep(Duration::from_millis(600));
        sim.stop();

        let ev = events.lock().unwrap();
        let serial: String = ev
            .iter()
            .filter_map(|e| match e {
                SimEvent::Serial(b) => Some(*b as char),
                _ => None,
            })
            .collect();
        assert!(
            serial.contains("hi"),
            "serial input never reached Serial.read(): {serial:?}"
        );
    }

    /// The PWM path end to end: `analogWrite(9, 64)` is 25% duty, and the UI's
    /// timing event must carry that rather than a flood of edges.
    #[test]
    fn pwm_duty_reaches_the_ui() {
        let dir = std::env::temp_dir().join("croquis-pwm-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let sketch = r#"
void setup() {
  pinMode(6, OUTPUT);
}
void loop() {
  analogWrite(6, 64);
}
"#;
        let fw = compile(sketch, &dir).expect("sketch should compile");

        let events: Arc<Mutex<Vec<SimEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let emit: Emit = Arc::new(move |e| sink.lock().unwrap().push(e));
        let mut sim = Simulator::start(Path::new(&fw.elf), emit, false).expect("sim should start");
        thread::sleep(Duration::from_millis(300));
        sim.stop();

        let ev = events.lock().unwrap();
        let duty = ev
            .iter()
            .find_map(|e| match e {
                SimEvent::Timing {
                    port: b'D',
                    pin: 6,
                    duty_per_mille,
                    ..
                } => Some(*duty_per_mille),
                _ => None,
            })
            .expect("no PWM timing reached the UI for pin 6 (D6)");
        assert!(
            (200..=300).contains(&duty),
            "analogWrite(6, 64) should be about 25% duty, got {duty} per mille"
        );
    }
}

#[cfg(test)]
mod core_cache_tests {
    use super::core_cache;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_read_only_toolchain_falls_back_instead_of_failing() {
        // The bug that reached a release: the shipped cache is read-only (an AppImage mount, or
        // /usr/lib owned by root) and was missing the .c objects, so the first compile tried to
        // write into it and died. The cache must move somewhere writable, and take what is there.
        let root = std::env::temp_dir().join("croquis-core-cache-test");
        let _ = std::fs::remove_dir_all(&root);
        let shipped = root.join("core-cache");
        std::fs::create_dir_all(&shipped).unwrap();
        std::fs::write(shipped.join("kept.o"), b"x").unwrap();
        std::fs::set_permissions(&shipped, std::fs::Permissions::from_mode(0o555)).unwrap();

        let resolved = core_cache(&root).expect("should not fail on a read-only toolchain");
        assert_ne!(resolved, shipped, "should not hand back a directory it cannot write");
        assert!(resolved.join("kept.o").exists(), "should have copied what shipped");

        // Leave the machine as we found it.
        std::fs::set_permissions(&shipped, std::fs::Permissions::from_mode(0o755)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&resolved);
    }

    #[test]
    fn a_writable_toolchain_is_used_as_it_is() {
        let root = std::env::temp_dir().join("croquis-core-cache-test-rw");
        let _ = std::fs::remove_dir_all(&root);
        let shipped = root.join("core-cache");
        std::fs::create_dir_all(&shipped).unwrap();
        assert_eq!(core_cache(&root).unwrap(), shipped);
        let _ = std::fs::remove_dir_all(&root);
    }
}
