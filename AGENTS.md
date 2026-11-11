# Working on Croquis

Instructions for an AI agent working in this repository — a contributor's or the maintainer's.
Humans will get on better with [the README](README.md); the reasoning behind everything here is in
[`docs/dev-notes.md`](docs/dev-notes.md).

## What this is

A desktop app for building and running Arduino circuits without an Arduino. Your sketch is compiled
locally with `avr-gcc` and executed on `simavr`, and the circuit around it is solved from the wires —
it is not a mock-up, and the LED lights to its real current.

Three constraints shape most design decisions, and none of them is negotiable:

- **Fully offline at runtime.** No server, no accounts, no telemetry, no network calls. The fonts are
  bundled rather than fetched for exactly this reason. Anything that would add a network dependency
  is wrong here even if it would be convenient.
- **Lightweight.** It should start instantly and stay small.
- **Desktop-first**, Linux only so far — Rust and egui, with no webview in the loop.

## Commands

| | |
| --- | --- |
| `./scripts/setup-toolchain.sh` | once — simavr, avr-gcc, the Arduino core |
| `cargo run --release --manifest-path app/Cargo.toml` | the app |
| `cargo test --manifest-path app/Cargo.toml` | the app's suite — presets, netlist, UI, editor |
| `cargo test --manifest-path crates/croquis-core/Cargo.toml` | the engine's suite |
| `cargo test --manifest-path crates/croquis-sim/Cargo.toml` | the simulator's suite |
| `cargo check --all-targets --manifest-path app/Cargo.toml` | warnings are errors in this crate |
| `./scripts/package.sh all` | the four packages, from one staged payload |
| `node scripts/check-versions.mjs [tag]` | the app's version declarations agree |
| `tools/ui-shot.sh` | screenshot the app under a headless sway |

**The toolchain is not in the repository.** `.toolchain/` is git-ignored and holds `avr-gcc`, the
Arduino core and a from-source `simavr`; `crates/croquis-sim/build.rs` links `libsimavr` and panics
without it. `./scripts/build-toolchain-dist.sh` trims it to what ships, and refuses to finish unless
its own `avr-g++` compiles a sketch for the atmega328p. `scripts/package.sh` runs it.

## Where things live

| | |
| --- | --- |
| `app/src/main.rs` | the shell: `ui()`, the panels, menus, the context menu, undo/redo, the unsaved guard |
| `app/src/scene.rs` | parts and wires on the canvas, the art for kinds Croquis draws itself, presets, `snap_to_axes` |
| `app/src/properties.rs` | the properties strip's rows |
| `app/src/editor.rs` | the editor's syntax and theme, and the compiler's diagnostics |
| `app/src/locales.rs` | `Locale`, the JSON catalogue, `Texts`, the saved choice |
| `app/src/art.rs` | rasterising the Wokwi SVGs through resvg into textures |
| `app/src/{lcd,knobs,buzzer,sensors,stepper,remote}.rs` | the parts that need behaviour or geometry of their own |
| `crates/croquis-core/src/netlist.rs` | the circuit model: nets, LEDs, sensors, relays and outputs |
| `crates/croquis-core/src/{diagram,document,route,catalogue}.rs` | the file format, the document, orthogonal routing, the part catalogue |
| `crates/croquis-sim/src/engine.rs` | compiling sketches, and the simavr bridge |
| `assets/locales/en.json`, `tr.json` | the two languages, flat key → text |
| `presets/*.json` | the shipped circuits, in the same shape Save writes |
| `libraries/` | the bundled Arduino libraries compiled offline with a sketch; provenance in `NOTICE` |
| `packaging/org.croquis.app.desktop` | the desktop entry every package installs |
| `scripts/package.sh` | one staged payload → .deb, .rpm, AppImage and Arch package |

**Adding a component is a catalogue entry plus its art, not a branch everywhere.** `PartDef` in
`catalogue.rs` carries the tag, the pin grouping and the editable fields (as translation keys), and
the palette, properties strip and renderer all work from that. Behaviour that is genuinely per-kind —
what the simulation drives, which state changes the drawing — stays explicit.

## Conventions

- **No drop shadows anywhere.** Menus and overlays float on a border instead.
- **No popups, no stacked menus.** Panels dock, or they do not exist.
- **Every colour the shell paints comes from `palette()`**, in one of its two flavours. A literal
  colour outside it is a theme bug: it will look right in the theme you are using and wrong in the
  other. Part art and wire colours are the exception — those are the circuit, not the chrome.
- The banned list in [`docs/dev-notes.md`](docs/dev-notes.md#anti-patterns-banned) is meant
  literally: no purple gradients, no marketing-speak, no fake loading spinners.
- **One concern per commit**, and a slice should be runnable rather than described.

## Traps

- **A wire's geometry is `f32` in one place and `f64` in another.** Pin positions are `f32`, route
  arithmetic is `f64`, and a corner six millionths off an axis makes the router insert an elbow — a
  wire that grows a bend every time it is saved. `scene::snap_to_axes` exists for this; the
  round-trip test over every preset is what caught it.
- **`egui_wants_keyboard_input` answers the wrong question.** It is true whenever *any* widget holds
  focus, so it swallowed the editor's own shortcuts. Ask `ctx.text_edit_focused()`.
- **The `clipboard` feature is load-bearing.** eframe turns Ctrl+C/X/V into events; with
  `default-features = false` the clipboard is a stub, copying fills nothing, no `Paste` arrives, and
  the shortcut looks simply dead.
- **Textures are rasterised once, at startup or on load.** Re-rasterising per frame is how the
  stepper used to stall; its rotor is drawn live as polygons instead.
- **`.toolchain/core-cache`** holds precompiled core objects and is what keeps compiles fast.
  `scripts/setup-toolchain.sh` builds it, including `yield.c.o` — which stands in for `hooks.c`, a
  source this toolchain refuses — and `wiring_pulse.S`, which `delay()` needs.
  `scripts/build-toolchain-dist.sh` ships it read-only, so an object missing there is one the app
  cannot compile at run time.
- **Packaging is one payload, four formats.** `scripts/package.sh` stages
  `usr/bin/croquis` plus `usr/lib/croquis/{toolchain,libraries}` and wraps that one tree. Off an rpm
  distro the rpm needs `_dbpath` and `__os_install_post` disabled; the Arch package is built without
  `makepkg` on CI. The details are in [`docs/dev-notes.md`](docs/dev-notes.md).

## Before you say it works

- `cargo test` on all three crates, and `cargo check --all-targets` clean — warnings are errors in
  the app crate, which catches the mistakes that build anyway.
- If you changed behaviour, **run the app and look at it**. Several of this project's real bugs were
  invisible in the source and obvious on screen — a dark canvas under a light theme, a button that
  did nothing. `tools/ui-shot.sh` will photograph it for you.
- If you are writing a claim into a doc, prove it or say it is unproven. One claim sat in the README
  being read as evidence while being false from the commit that introduced it.
- **Prove a claim when you write it, not in a sweep at the end.** Every bug this project found late
  was findable on the day its slice landed: a netlist that latched a pin high, a paste that drifted
  six pixels, a wire that grew a bend per save. An audit at the end of a project is worth doing and
  is not a substitute for checking things as you go.
- **Test the artefact, not just the development build.** A `.deb` was once built, its metadata
  inspected, its dependencies confirmed — and never run. It installed cleanly and could not compile a
  sketch. `./scripts/package.sh all`, then install or unpack one, and use *that*.
- **Every slice ends with something the owner can run.** They are the person who actually uses it;
  the test suite is not a substitute for that, however many checks it runs. A bug that surfaces later
  and was findable at the time is a process failure rather than bad luck.

## Review convention

Whoever owns the project decides product questions: what to build, what it should feel like, what is
in scope. Technical choices come with plain-language reasons and honest trade-offs rather than a menu
of options with no opinion. And when something turns out to be broken, the useful report is what you
did, what you expected, and what you saw.
