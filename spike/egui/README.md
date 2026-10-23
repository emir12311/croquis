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

The Wokwi asset/licensing decision is still open. The vector placeholders are
intentionally not the final catalogue.

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
