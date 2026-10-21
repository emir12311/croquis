/**
 * The `@wokwi/elements` we draw parts with, in one place.
 *
 * Importing the whole library would pull in every sensor and display, which
 * works against the "stay lightweight" rule — so these are the elements we
 * use, imported one by one. Anything that needs a correction to make an
 * element *usable* lives here rather than scattered over the app.
 */

import '@wokwi/elements/dist/esm/arduino-uno-element'
import '@wokwi/elements/dist/esm/led-element'
import '@wokwi/elements/dist/esm/rgb-led-element'
import '@wokwi/elements/dist/esm/resistor-element'
import '@wokwi/elements/dist/esm/pushbutton-element'
import '@wokwi/elements/dist/esm/potentiometer-element'
import '@wokwi/elements/dist/esm/slide-potentiometer-element'
import '@wokwi/elements/dist/esm/photoresistor-sensor-element'
import '@wokwi/elements/dist/esm/ntc-temperature-sensor-element'
import '@wokwi/elements/dist/esm/analog-joystick-element'
import '@wokwi/elements/dist/esm/tilt-switch-element'
import '@wokwi/elements/dist/esm/pir-motion-sensor-element'
import '@wokwi/elements/dist/esm/flame-sensor-element'
import '@wokwi/elements/dist/esm/small-sound-sensor-element'
import '@wokwi/elements/dist/esm/buzzer-element'
import '@wokwi/elements/dist/esm/ks2e-m-dc5-element'
import '@wokwi/elements/dist/esm/7segment-element'
import '@wokwi/elements/dist/esm/led-bar-graph-element'

/**
 * Feed a stylesheet into an element's own shadow root, once, after it connects.
 *
 * Some elements in the package ship without styles — most recently the slide
 * potentiometer, unlike the rotary pot which hides its own range input with
 * `static styles`. Unstyled, the element renders inline and invisible: its
 * host collapses to a sliver at the baseline while the drawing floats above it,
 * so nothing that measures the host box (the renderer, the palette, the drag
 * hit tests) sees the right shape. The style is ours to give — the element is
 * ours to size.
 */
function ensureShadowStyles(tag: string, css: string) {
  customElements.whenDefined(tag).then(() => {
    const proto = customElements.get(tag)!.prototype as {
      firstUpdated?: (props: unknown) => void
    }
    // Lit builds the shadow root during the element's first update, after
    // `connectedCallback` — so the injection lands in `firstUpdated`, the
    // library passes on the class's own hook if it ever grows one.
    const firstUpdated = proto.firstUpdated
    proto.firstUpdated = function (props: unknown) {
      firstUpdated?.call(this, props)
      const root = (this as HTMLElement).shadowRoot
      if (root && !root.querySelector('style[data-croquis-shim]')) {
        const style = document.createElement('style')
        style.dataset.croquisShim = ''
        style.textContent = css
        root.append(style)
      }
    }
  })
}

// The slide pot draws the same hidden range input the rotary pot renders —
// and then forgets to hide it. The drawing is block, the input absolute and
// clipped, so the host box is exactly the drawing, all the way down.
ensureShadowStyles(
  'wokwi-slide-potentiometer',
  '.hide-input { position: absolute; width: 1px; height: 1px; clip-path: inset(50%); overflow: hidden; } svg { display: block; }',
)
