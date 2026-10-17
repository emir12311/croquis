//! Application entry point: the Tauri window, the file commands, and the
//! simulation commands that drive `engine`.

pub mod engine;

use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

/// Holds the running simulator, if any.
#[derive(Default)]
struct SimState(Mutex<Option<engine::Simulator>>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    prepare_linux_webview();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(SimState::default())
        .invoke_handler(tauri::generate_handler![
            read_text_file,
            write_text_file,
            compile_sketch,
            start_sim,
            stop_sim,
            set_input
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Read a UTF-8 text file — used to open a saved `diagram.json`.
#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    std::fs::read_to_string(&path).map_err(|e| e.to_string())
}

/// Write a UTF-8 text file — used to save a `diagram.json`.
#[tauri::command]
fn write_text_file(path: String, contents: String) -> Result<(), String> {
    std::fs::write(&path, contents).map_err(|e| e.to_string())
}

/// Compile an Arduino sketch and return the firmware location + size.
#[tauri::command]
fn compile_sketch(sketch: String) -> Result<engine::Firmware, String> {
    let dir = std::env::temp_dir().join("croquis-build");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    engine::compile(&sketch, &dir)
}

/// Start running compiled firmware, streaming pin changes and serial to the UI.
#[tauri::command]
fn start_sim(
    elf: String,
    bounce: bool,
    app: tauri::AppHandle,
    state: State<SimState>,
) -> Result<(), String> {
    let mut slot = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(mut previous) = slot.take() {
        previous.stop();
    }
    let emitter: engine::Emit = Arc::new(move |event: engine::SimEvent| match event {
        engine::SimEvent::Pin { port, pin, high } => {
            let _ = app.emit(
                "sim:pin",
                serde_json::json!({ "port": port, "pin": pin, "high": high }),
            );
        }
        engine::SimEvent::Serial(byte) => {
            let _ = app.emit("sim:serial", byte);
        }
    });
    *slot = Some(engine::Simulator::start(Path::new(&elf), emitter, bounce)?);
    Ok(())
}

/// Stop the running simulation, if any.
#[tauri::command]
fn stop_sim(state: State<SimState>) {
    if let Ok(mut slot) = state.0.lock() {
        if let Some(mut sim) = slot.take() {
            sim.stop();
        }
    }
}

/// Drive an input pin — a pressed button, a held line, etc.
#[tauri::command]
fn set_input(port: u8, pin: u8, high: bool, state: State<SimState>) {
    if let Ok(slot) = state.0.lock() {
        if let Some(sim) = slot.as_ref() {
            sim.set_pin(port, pin, high);
        }
    }
}

/// NVIDIA proprietary driver workaround.
///
/// WebKitGTK's GPU path is broken on the NVIDIA proprietary driver: blank
/// windows, `Failed to create GBM buffer`, or `Error 71 (Protocol error)
/// dispatching to Wayland display`. The recommended fixes differ by display
/// server:
///
///   - Wayland: disable NVIDIA explicit sync. This keeps WebKit's accelerated
///     path, so rendering stays smooth. (We also force the DMA-BUF renderer
///     back on, since some distros ship a patch that disables it for NVIDIA.)
///   - X11: fall back to the non-DMA-BUF renderer (slower, but it draws).
///
/// Every value is only set when the user hasn't set it themselves, so they can
/// always override.
///
/// See: <https://v2.tauri.app/develop/debug/linux-graphics/>
#[cfg(target_os = "linux")]
/// Which environment WebKit needs on this machine, as a decision that can be tested.
///
/// The DMA-BUF renderer is the part that crashes. On the NVIDIA proprietary driver it is
/// the known-bad path; on a plain machine with no vendor driver it segfaults too — a live
/// USB was the first place that showed up, because there is no /sys/module/nvidia there, no
/// workaround was applied at all, and the window booted, drew once and died.
///
/// So DMA-BUF is off everywhere, and a user whose machine handles it can set
/// WEBKIT_DISABLE_DMABUF_RENDERER=0 to get the faster path back.
fn webview_env(nvidia: bool, wayland: bool) -> Vec<(&'static str, &'static str)> {
    let mut env = vec![("WEBKIT_DISABLE_DMABUF_RENDERER", "1")];
    if nvidia && wayland {
        // Keeps GPU acceleration on the proprietary driver, where it works once explicit
        // sync is out of the way.
        env.push(("__NV_DISABLE_EXPLICIT_SYNC", "1"));
    }
    env
}

fn prepare_linux_webview() {
    let nvidia = std::path::Path::new("/sys/module/nvidia").exists();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    for (key, value) in webview_env(nvidia, wayland) {
        set_env_default(key, value);
    }
}

#[cfg(target_os = "linux")]
fn set_env_default(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        std::env::set_var(key, value);
    }
}

#[cfg(not(target_os = "linux"))]
fn prepare_linux_webview() {}

#[cfg(test)]
mod webview_env_tests {
    use super::webview_env;

    fn keys<'a>(env: &'a [(&'a str, &'a str)]) -> Vec<&'a str> {
        env.iter().map(|(key, _)| *key).collect()
    }

    #[test]
    fn dma_buf_is_disabled_on_every_machine() {
        // The crash on a live USB was WebKitGTK's DMA-BUF renderer, on a machine with no
        // vendor driver at all — so this cannot be conditional on finding one.
        for (nvidia, wayland) in [(false, false), (false, true), (true, false), (true, true)] {
            assert!(
                webview_env(nvidia, wayland).contains(&("WEBKIT_DISABLE_DMABUF_RENDERER", "1")),
                "DMA-BUF not disabled for nvidia={nvidia} wayland={wayland}"
            );
        }
    }

    #[test]
    fn nvidia_on_wayland_is_the_only_case_with_an_extra() {
        assert_eq!(
            keys(&webview_env(true, true)),
            ["WEBKIT_DISABLE_DMABUF_RENDERER", "__NV_DISABLE_EXPLICIT_SYNC"]
        );
        assert_eq!(keys(&webview_env(true, false)), ["WEBKIT_DISABLE_DMABUF_RENDERER"]);
        assert_eq!(keys(&webview_env(false, true)), ["WEBKIT_DISABLE_DMABUF_RENDERER"]);
    }
}
