//! The engine behind Croquis: compile an Arduino sketch with `avr-gcc`, then run
//! the firmware on **simavr**.
//!
//! This crate deliberately knows nothing about Tauri, egui, the catalogue or the
//! diagram format. It owns the toolchain lookup, the firmware build cache, the
//! simulator thread and the pin/serial/timing events.

pub mod engine;
pub mod timing;
