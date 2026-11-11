# Development notes

Things that cost time to learn, so they are not learned twice. Some of these are about this
machine or this driver; all of them are about *this* project.

## Locked stack, and why

| Layer | Choice | Why (short) |
|---|---|---|
| App shell | **egui** (eframe, glow) | immediate mode; UI and engine in one language, no webview |
| Fake chip | **simavr** (C), bridged into Rust | mature & feature-complete; Rust has no ready-made AVR core |
| Compiler | **avr-gcc** | bundled, offline, no external installs |
| Code editor | **egui_code_editor** | completion, auto-indent and wrapping out of the box |
| Part drawings | **@wokwi/elements** SVG, rasterised with **resvg** | the Wokwi look, ready-made (MIT) |
| Save file | **diagram.json** | Wokwi's documented format; portable |
| Language | **Rust** | the engine and the shell share one language |
| Translations | **flat JSON** in `assets/locales/` | a translator needs a text editor, not a build |
| License | **AGPL-3.0** | GPL leaves a hosted-service loophole; AGPL section 13 closes it. |

## Gotchas we hit

- **Parts are rasterised once, not re-parsed every frame.** Each Wokwi `<svg>` goes through resvg into an
  egui texture, and the palette tiles are the same art at a smaller size, so the two cannot drift apart.
  The stepper is the case that proved the rule: its rotor used to be re-rasterised for every angle, and a
  660×706 SVG per frame of a spinning motor stalled the UI. The rotor's arrow and hub are now drawn live
  as polygons — `scene::STEPPER_ROTOR` carries the numbers, and a test pins them to the asset.
- **Exact float equality on geometry invents corners.** Pin positions are `f32` and route arithmetic is
  `f64`: `89.0 + 116.6` is not quite `f32`'s `205.6`, so a decoded wire sat ~6e-6 off the axis its
  neighbour was on. The router's `==` axis test then inserted an elbow, and the next save wrote that
  elbow out as a real bend — a wire that grew a corner every time it was saved. The round-trip test over
  all 24 presets is what found it. `scene::snap_to_axes` now nudges anything within 1e-3 onto its
  neighbour's axis before the route is expanded.
- **`egui_wants_keyboard_input` is the wrong question.** It is true whenever *any* widget holds focus, so
  the editor's own shortcuts — Ctrl+Z among them — were being swallowed by the app. Ask
  `ctx.text_edit_focused()` when the question is "does the code editor have the keyboard".
- **eframe turns Ctrl+C/X/V into events, and needs the `clipboard` feature.** With
  `default-features = false` the clipboard is a stub: copying fills nothing, no `Paste` event is ever
  emitted, and the shortcut looks simply dead. The feature is on, and the app listens for
  `Event::Copy` / `Cut` / `Paste` rather than for key combinations.
- **Nested menus close themselves.** In egui, opening a submenu closes its parent, so the Presets menu
  changes state instead of relying on the menu staying open across a click.
- **A guard that asks can cancel its own answer.** The unsaved-changes question cancels a window close so it
  can ask. When the reader then chose Discard, the close was sent again — and the guard, still seeing a dirty
  document, cancelled *that* close and re-opened the prompt, so the button looked dead. Save only appeared to
  work because saving makes the document clean. The guard has to be disarmed by the answer, not by the outcome:
  Discard records the current state as the baseline before carrying the close out.
- **The palette is a process-wide switch, deliberately.** The shell's six colours became a `Palette` with a dark
  and a light flavour, read through `palette()`. Immediate-mode drawing reads colours from free functions and
  closures all over this file; threading a palette parameter through every one of them would have touched far
  more than the constants it replaced. The trade is that the theme is global state — only `apply_theme` writes
  it, and the saved choice is read once at startup, before the style is built. A colour that does not come from
  the palette is a theme bug waiting to be found by eye: the canvas, the grid, the palette tiles and the Run
  button were all caught that way.
- **The breadboard is generated, not borrowed.** Wokwi's element library has no breadboard, and the
  detailed third-party artwork (Fritzing) is **CC-BY-SA**, which cannot be baked into an AGPL-3.0 binary.
  So it is built from a spec instead — on a breadboard the drawing and the pin map are the *same* regular
  grid, so one spec gives both and they cannot drift apart (which would put a wire half a hole off the
  copper it should touch).
- **The toolchain is downloaded, not committed.** `.toolchain/` (Arduino AVR `avr-gcc`, the Arduino core,
  and a from-source `simavr`) is git-ignored, and `croquis-sim`'s build script links `libsimavr`.
  `scripts/build-toolchain-dist.sh` trims 290 MB to 160 MB for shipping and will not finish unless its
  own `avr-g++` compiles a sketch for the atmega328p — a toolchain that looks complete and fails every
  compile is the failure mode worth guarding.
- **Packaging is one payload, four formats.** `scripts/package.sh` stages `usr/bin/croquis` plus
  `usr/lib/croquis/{toolchain,libraries}` and wraps that one tree as a .deb, .rpm, AppImage and Arch
  package, so the four cannot disagree about what is inside. Two things bite off an rpm distro:
  `brp-strip` chokes on the bundled AVR `.a` archives (so `__os_install_post` is disabled — the app
  binary is stripped once, in `stage`), and `/var/lib/rpm` does not exist, so `_dbpath` points at a
  scratch database. Do not pre-create that database with `rpm --initdb`: it segfaults under rpm 6.0.1
  while `rpmbuild` initialises it happily.
- **Test the netlist the way the app calls it.** The button model asked "is the net the button
  shorts to high?" — but pressing a button makes its two sides *one* net, so that net held the
  MCU pin as well as the rail, and the pin's own driven level answered for itself. It latched:
  every press computed "drive high", and any button wired pin-to-GND had done nothing since the
  netlist first landed. The unit test passed because it handed the netlist a cleaner world than
  the app does — a pin that had never been driven. Anything that reads back a level the app
  itself sets has to be tested *with* that level present. The regression test now does.
- **When a feature misbehaves, measure inside the app before theorising.** Three readings of that
  button bug were wrong — the ground pin, a reactivity spread, the pin-arming radius. What settled
  it was having the app record what it decided (the pressed state, the drives it computed, whether
  the command resolved) to a file and reading that. It was a temporary patch, stripped out again;
  the fix it produced is the commit that kept its promise.

## Anti-patterns, banned

- Purple / indigo gradients · Tailwind's default slate palette · random un-systemed icons · glassmorphism
- Marketing-speak copy ("Seamlessly", "Powerful", "Effortlessly")
- Rounded corners on everything · drop shadows everywhere
- Decorative / fake loading spinners · controls that don't explain themselves · anything essential hidden behind stacked menus · popup windows
