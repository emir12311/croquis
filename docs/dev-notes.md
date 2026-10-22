# Development notes

Things that cost time to learn, so they are not learned twice. Some of these are about this
machine or this driver; all of them are about *this* project.

## Locked stack, and why

| Layer | Choice | Why (short) |
|---|---|---|
| App shell | **Tauri** | tiny, light on memory |
| Fake chip | **simavr** (C), bridged into Rust | mature & feature-complete; Rust has no ready-made AVR core |
| Compiler | **avr-gcc** | bundled, offline, no external installs |
| UI toolkit | **Svelte 5** | small, fast output |
| Code editor | **CodeMirror 6** | light, touch-friendly, auto-brackets / auto-indent |
| Part drawings | **@wokwi/elements** | the Wokwi look, ready-made (MIT) |
| Save file | **diagram.json** | Wokwi's documented format; portable |
| Engine language | **Rust** | the engine and the shell share one language |
| UI language | **TypeScript** | safety net; fewer bugs reach you |
| License | **AGPL-3.0** | GPL leaves a hosted-service loophole; AGPL section 13 closes it.  |

## Gotchas we hit

- **The circuit is drawn on a `<canvas>`, not as DOM/SVG.** WebKitGTK (on the NVIDIA driver, at least) rasterises *painted* DOM at reduced resolution, blurring the parts, grid and wires. So the visible circuit is drawn onto a canvas at device resolution; the DOM stays `opacity: 0` and handles hit-testing and interaction. Parts are rasterised from their shadow-root `<svg>` — serialised as **XML** (HTML `outerHTML` emits invalid entities like `&nbsp;`), with styles read from **both** `<style>` and `adoptedStyleSheets` — and are sized/rotated from the rasterised drawing's natural size. That rasterising lives in `src/lib/part-art.ts`, shared with the palette thumbnails so the two can't drift apart.
- **NVIDIA + WebKitGTK (Linux).** WebKitGTK's GPU path is broken on the NVIDIA proprietary driver — blank windows, `Error 71 (Protocol error)` on Wayland, or `Failed to create GBM buffer`. `src-tauri/src/lib.rs` disables the DMA-BUF renderer on NVIDIA and on machines where no GPU driver can be identified, and additionally disables NVIDIA explicit sync on Wayland. Intel and AMD keep DMA-BUF: the fallback is much slower, and forcing it on an i915 laptop made panning and simulation visibly stutter. Every choice is overridable through `WEBKIT_DISABLE_DMABUF_RENDERER` and `__NV_DISABLE_EXPLICIT_SYNC`. See [Tauri's Linux graphics notes](https://v2.tauri.app/develop/debug/linux-graphics/).
- **Don't fight WebKit's animations.** CodeMirror blinks the caret with a CSS animation (`cm-blink`); a running animation makes WebKitGTK drop to low-resolution tiles *globally*, blurring everything. The caret blink is disabled for this reason.
- **The dev port is pinned to 5173 with `strictPort: true`**, because Tauri's `devUrl` has to match exactly. That has a sharp edge: a *second* dev server — a test harness, another checkout — can't bind, and `npm run tauri dev` then fails to start **while the stale page stays up**, so you can be looking at old code and not know it. Run anything else on another port: `npm run dev -- --port 5199`.
- **File I/O** is done by Rust commands (`read_text_file` / `write_text_file`) with `tauri-plugin-dialog` for the picker. Both accept only an absolute `.json` path and cap a circuit at 8 MiB, so the commands cannot be turned into a general file API. The webview runs under a CSP (`csp`, and `devCsp` for the Vite server) that permits no network origin — the bundled fonts and `blob:` sprites are the only resources it loads.
- **The breadboard is generated, not borrowed.** Wokwi's element library has no breadboard, and the detailed third-party artwork (Fritzing) is **CC-BY-SA**, which can't be baked into an AGPL-3.0 binary. So `src/lib/breadboard.ts` builds it from a spec instead — on a breadboard the drawing and the pin map are the *same* regular grid, so one spec gives both and they can't drift apart (which would put a wire half a hole off the copper it should touch). The pitch is `67/7`px (~9.5714), chosen so the worst-fitting part lands exactly rather than on a round number — `src/lib/breadboard.ts` carries the numbers and why they were picked.
- **The toolchain is downloaded, not committed.** `.toolchain/` (Arduino AVR `avr-gcc`, the Arduino core, and a from-source `simavr`) is git-ignored; `src-tauri/build.rs` links `libsimavr` and generates the FFI bindings with `bindgen` + a small C shim.
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
