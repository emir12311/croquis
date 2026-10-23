# egui Spike A

Phase 0 canvas spike for the v0.3 rewrite. It is isolated from the shipped app.

```bash
cargo run --manifest-path spike/egui/Cargo.toml
cargo test --manifest-path spike/egui/Cargo.toml
```

What it proves:

- screen ↔ world transforms with pan and pointer-centred zoom;
- part and pin hit-testing in data, not DOM;
- dragging a part, wiring two pins, cancelling with Esc;
- a deterministic geometry test that runs without a display.

This is **not** the new app and does not include real part art yet. The next
Spike A step is to replace the placeholder vector parts with a chosen art
pipeline and measure texture memory and rasterisation time.
