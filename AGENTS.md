# Working on Croquis

Instructions for an AI agent working in this repository — a contributor's or the maintainer's.
Humans will get on better with [the README](README.md); the reasoning behind everything here is in
[`docs/dev-notes.md`](docs/dev-notes.md).

## What this is

A desktop app for building and running Arduino circuits without an Arduino. Your sketch is compiled
locally with `avr-gcc` and executed on `simavr`, and the circuit around it is solved from the wires —
it is not a mock-up, and the LED lights to its real current.

Three constraints shape most design decisions, and none of them is negotiable:

- **Fully offline at runtime.** No server, no accounts, no telemetry, no network calls. The fonts
  are bundled rather than fetched for exactly this reason, and the app is granted only
  `core:default` and `dialog:default`, under a CSP that allows no network origin. Anything that
  would add a network dependency is wrong here even if it would be convenient.
- **Lightweight.** It should start instantly and stay small.
- **Desktop-first** (Tauri), Linux only so far.

## Commands

| | |
| --- | --- |
| `npm install` | once |
| `./scripts/setup-toolchain.sh` | once — simavr, avr-gcc, the Arduino core |
| `./scripts/build-toolchain-dist.sh` | **required before `tauri build`** — `tauri.conf.json` bundles `../.toolchain-dist`, which only this creates |
| `npm run tauri dev` | the real app, with HMR |
| `npm run tauri build` | a release build |
| `npm test` | frontend tests, vitest |
| `npm run check:versions` | every manifest agrees with `package.json` — or pass a tag: `-- v0.1.2` |
| `npm run check` | svelte-check and TypeScript — must be clean |
| `cargo test --manifest-path src-tauri/Cargo.toml` | engine tests — compiles and runs real sketches |

**The toolchain is not in the repository.** `.toolchain/` is git-ignored and holds `avr-gcc`, the
Arduino core and a from-source `simavr`. If it is missing, `src-tauri/build.rs` panics with the
simavr steps; the rest are in [the README](README.md#build-from-source).

## Where things live

| | |
| --- | --- |
| `src/App.svelte` | the composition root: markup, panel layout, menus, properties strip |
| `src/lib/project.svelte.ts` | the circuit document: parts, wires, undo, clipboard, dirty |
| `src/lib/document.ts` | document types and pure helpers (snapshots, attributes, copies) |
| `src/lib/file-session.svelte.ts` | save/save-as/open, the home path and its errors |
| `src/lib/canvas.svelte.ts` | pin geometry from the mounted elements, sprites, palette art, drawing |
| `src/lib/input.svelte.ts` | selection and the gestures: wiring, dragging, marquee, pan |
| `src/lib/sim.svelte.ts` | run/stop, chip pins, serial, netlist solving |
| `src/lib/view.svelte.ts` | zoom/pan/fit and screen ↔ canvas coordinates |
| `src/lib/sketch.svelte.ts` | the sketch text and its saved/dirty pair |
| `src/lib/netlist.ts` | the circuit model: nets, LEDs/RGB, sensors, relays and outputs |
| `src/lib/diagram.ts` | the `diagram.json` format, read and written |
| `src/lib/route.ts` | orthogonal wire routing. **Nothing here may produce a diagonal** |
| `src/lib/parts.ts` | the component catalogue — the registry everything else works from |
| `src/lib/breadboard.ts` | the breadboard, generated from a grid spec |
| `src/lib/snap.ts` | dropping and moving parts onto holes |
| `src/lib/circuit-render.ts` | the canvas renderer |
| `src/lib/part-art.ts` | rasterising the `@wokwi/elements` parts into sprites |
| `src/lib/buzzer-audio.ts` | the WebAudio voice for driven buzzers |
| `src/lib/serial-scroll.ts` | whether the serial monitor should follow new output |
| `src/lib/wokwi.ts` | the `@wokwi/elements` we import, and element fixes such as the slide pot's missing styles |
| `src/lib/hc595-element.ts` | the hand-drawn 74HC595 DIP-16 — `@wokwi/elements` has no logic parts |
| `src/lib/shift-register.ts` | the 74HC595's pure shift/latch state machine |
| `src/lib/arduino-api.ts` | the editor's Arduino completions |
| `src/lib/presets.ts` + `presets/*.json` | the shipped circuits |
| `libraries/` | the bundled Arduino libraries compiled offline with a sketch; provenance in `NOTICE.md` |
| `src-tauri/src/engine.rs` | compiling sketches, and the simavr bridge |
| `src-tauri/src/lib.rs` | the Tauri commands |
| `src/lib/*.test.ts` | a test file per module, next to it |

**Adding a component is a catalogue entry plus an element, not a branch everywhere.** `PartDef` in
`parts.ts` carries the tag, the pin grouping (`pinNode`) and the editable fields, and the renderer,
palette and properties strip all work from that. Behaviour that is genuinely per-kind — what the
simulation drives, which state changes a drawing — stays explicit rather than forced into the
catalogue.

## Conventions

- **No drop shadows anywhere**, in any stylesheet. Menus and overlays float on a border instead.
- **No popups, no stacked menus.** Panels dock, or they do not exist.
- Everything is themed through CSS variables — a light theme and a dark one, and no third.
- The banned list in [`docs/dev-notes.md`](docs/dev-notes.md#anti-patterns-banned) is meant
  literally: no purple gradients, no marketing-speak, no fake loading spinners.
- **One concern per commit**, and a slice should be runnable rather than described.

## Traps

- **The dev port is pinned to 5173 with `strictPort: true`**, because Tauri's `devUrl` must match
  it. A second dev server cannot bind, and `npm run tauri dev` then fails to start **while the stale
  page stays up** — so you can be looking at old code and not know it. Run anything else with
  `npm run dev -- --port 5199`.
- **The circuit is drawn on a `<canvas>`.** Each part's DOM is invisible and exists only for
  hit-testing; the visible pixels are rasterised sprites. So reading the DOM tells you about
  *interaction*, not about what a part looks like.
- **WebKitGTK on the NVIDIA proprietary driver needs help** — `src-tauri/src/lib.rs` applies the fix
  automatically. A running CSS *animation* makes WebKitGTK drop to low-resolution tiles globally and
  blur everything, which is why the caret blink is disabled and overlays do not animate.
- **`.toolchain/core-cache`** holds precompiled core objects and is what keeps compiles fast.
  `scripts/setup-toolchain.sh` builds it (step 4), including `yield.c.o` — which stands in for
  `hooks.c`, a source this toolchain refuses — and `wiring_pulse.S`, which `delay()` needs.
  `scripts/build-toolchain-dist.sh` ships it read-only, so an object missing here is one the app
  cannot compile at run time. Bundled-library objects live in its `lib-objects/` subdirectory,
  because the cache sweep in `compile_core()` links everything at the top level.

## Before you say it works

- `npm run check` clean, and both test commands passing. `npm run check` catches the mistakes that
  build anyway.
- If you changed behaviour, **drive the app** rather than reasoning about the code. Several of this
  project's real bugs were invisible in the source and obvious within a minute of driving it.
  **`npm run drive`** runs the checks that need a live app: it starts a dev server on 5199 and
  headless chromium on 9222, drives the real UI, and cleans up after itself. `import { connect }`
  from `scripts/drive.mjs` to write a one-off check against whatever you are working on.
- If you are writing a claim into a doc, prove it or say it is unproven. One claim sat in the README
  being read as evidence while being false from the commit that introduced it.
- **Prove a claim when you write it, not in a sweep at the end.** Every bug this project found late
  was findable on the day its slice landed: a netlist that latched a pin high, a paste that drifted
  six pixels, a view whose origin sat in a corner. An audit at the end of a project is worth doing
  and is not a substitute for checking things as you go.
- **Test the artefact, not just the development build.** A `.deb` was built, its metadata inspected,
  its dependencies confirmed — and never run. It installed cleanly and could not compile a sketch,
  because the compiler was looked up in the source tree. Build the package, install it, and use the
  installed thing before thinking about a release.
- **Every slice ends with something the owner can run.** They are the person who actually uses it;
  the harness is not a substitute for that, however many checks it runs. A bug that surfaces later
  and was findable at the time is a process failure rather than bad luck.

## Review convention

Whoever owns the project decides product questions: what to build, what it should feel like, what is
in scope. Technical choices come with plain-language reasons and honest trade-offs rather than a menu
of options with no opinion. And when something turns out to be broken, the useful report is what you
did, what you expected, and what you saw.
