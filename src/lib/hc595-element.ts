/**
 * A 74HC595 in a DIP-16 package.
 *
 * `@wokwi/elements` has no logic parts, so this one is drawn here. It only has
 * to do what the rasteriser expects: a shadow root holding an `<svg>`, a
 * laid-out size, and a `pinInfo` of `{ name, x, y }` in the host's pixels.
 *
 * Pins are numbered the way a DIP is: pin 1 bottom-left, pin 16 top-left.
 */

export const HC595_WIDTH = 76
export const HC595_HEIGHT = 38

const PIN_X = [4.8, 14.4, 24, 33.6, 43.2, 52.8, 62.4, 72]

const BOTTOM = ['Q1', 'Q2', 'Q3', 'Q4', 'Q5', 'Q6', 'Q7', 'GND']
const TOP = ['VCC', 'Q0', 'DS', 'OE', 'STCP', 'SHCP', 'MR', 'Q7S']

const pinInfo = [
  ...BOTTOM.map((name, i) => ({ name, x: PIN_X[i]!, y: HC595_HEIGHT, number: i + 1 })),
  ...TOP.map((name, i) => ({ name, x: PIN_X[i]!, y: 0, number: 16 - i })),
]

const pins = PIN_X.map(
  (x) =>
    `<rect x="${(x - 2.75).toFixed(1)}" y="0" width="5.5" height="8" fill="#c8c8c8" stroke="#8f8f8f" stroke-width="0.6"/>` +
    `<rect x="${(x - 2.75).toFixed(1)}" y="30" width="5.5" height="8" fill="#c8c8c8" stroke="#8f8f8f" stroke-width="0.6"/>`,
).join('')

const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${HC595_WIDTH}" height="${HC595_HEIGHT}" viewBox="0 0 ${HC595_WIDTH} ${HC595_HEIGHT}">
  ${pins}
  <rect x="4" y="6" width="68" height="26" rx="1.5" fill="#1c1c1c" stroke="#000" stroke-width="0.8"/>
  <circle cx="8.6" cy="29.4" r="1.2" fill="#8a8a8a"/>
  <text x="38" y="17.4" text-anchor="middle" font-family="monospace" font-size="9.5" font-weight="700" fill="#d8cf3f">74HC</text>
  <text x="38" y="27" text-anchor="middle" font-family="monospace" font-size="9.5" font-weight="700" fill="#d8cf3f">595</text>
</svg>`

if (typeof customElements !== 'undefined' && !customElements.get('wokwi-74hc595')) {
  customElements.define(
    'wokwi-74hc595',
    class extends HTMLElement {
      readonly pinInfo = pinInfo
      connectedCallback() {
        if (this.shadowRoot) return
        const root = this.attachShadow({ mode: 'open' })
        root.innerHTML = `<style>:host{display:block;width:${HC595_WIDTH}px;height:${HC595_HEIGHT}px}svg{display:block}</style>${svg}`
      }
    },
  )
}
