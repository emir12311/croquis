# egui Phase 0 spikes

Isolated Phase 0 experiments for the v0.3 rewrite. None of this is part of the
shipped app.

## Spike A — canvas and SVG art

```bash
cargo run --manifest-path spike/egui/Cargo.toml
cargo test --manifest-path spike/egui/Cargo.toml
```

Proves:

- screen ↔ world transforms with pan and pointer-centred zoom;
- part and pin hit-testing in data, not DOM;
- dragging a part, wiring two pins, cancelling with Esc;
- `resvg` rasterises an embedded SVG into an egui texture;
- deterministic geometry, hit-test and raster tests without a display.

The spike now loads the Blink preset through `croquis-core` and renders the
extracted Wokwi SVGs/pins from `assets/wokwi`. Dynamic state overlays (LED lit,
RGB channels, displays) are still pending.

## Spike B — editor and diagnostics

```bash
cargo run --manifest-path spike/egui/Cargo.toml --bin editor
cargo test --manifest-path spike/egui/Cargo.toml
```

Proves:

- multiline TextEdit with a syntax-colouring layouter;
- completion filtering and insertion for Arduino-style names;
- a diagnostic line/column jump that moves and focuses the caret;
- pure tests for line/column mapping, UTF-8 byte conversion, word prefixes,
  completion filtering and highlighter text preservation.

Not yet proven: IME/compose behaviour, long-line performance, a maintained
editor-crate comparison, and completion at a wrapped caret on a real display.
Those are measured before choosing the production editor.

## Spike C — host shell

```bash
cargo run --manifest-path spike/egui/Cargo.toml --bin host
cargo test --manifest-path spike/egui/Cargo.toml --bin host -- --ignored
```

Proves:

- native Open/Save through `rfd`;
- a background compile/run worker calling the existing Tauri-free engine API;
- live pin, serial and timing events drained into the egui UI;
- a native generated-tone audio path through `rodio`;
- a release binary can be built without a webview.

The engine now comes from the standalone `croquis-sim` crate; the host no longer
depends on `src-tauri`. A display run, an installed deb/rpm/AppImage and
audio-device behaviour still need measurement. This spike does not yet load or
render a real diagram — it exercises the host boundary around the engine.

Measured release build on the development machine after the extraction:

- `spike/egui/target/release/host` is 14 MB;
- the release build took 1m45s;
- `ldd` contains no WebKit, GTK or Tauri libraries; the remaining system link is
  ALSA for audio. The lightweight claim can now be measured against this base.
