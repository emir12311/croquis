/**
 * The component catalogue — the registry the rest of the app reads from.
 *
 * An entry names the element that draws the part, its default attributes, its
 * palette fallback icon, and how its pins group electrically (`pinNode`). That's
 * enough for adding a component to be a catalogue entry plus an element: the
 * renderer, the palette and the netlist work off this rather than switching on
 * kinds. (The palette shows the element's *real* drawing, rasterised — see
 * `lib/part-art.ts`; `icon` is only what it falls back to before that's ready.)
 *
 * Deliberately still per-kind, in `App.svelte`: what the simulation drives on an
 * element, and which state changes its drawing. Those are behaviour, not
 * registration.
 */

import { BREADBOARDS, breadboardPinNode } from './breadboard'

export type PartKind =
  | 'uno'
  | 'led'
  | 'rgb-led'
  | 'resistor'
  | 'pushbutton'
  | 'potentiometer'
  | 'slide-potentiometer'
  | 'photoresistor-sensor'
  | 'ntc-temperature-sensor'
  | 'analog-joystick'
  | 'tilt-switch'
  | 'pir-motion-sensor'
  | 'flame-sensor'
  | 'sound-sensor'
  | 'buzzer'
  | 'relay'
  | 'seven-segment'
  | 'seven-segment-4'
  | 'led-bar-graph'
  | 'breadboard'
  | 'breadboard-full'

/** Both breadboard sizes behave identically — one generator, two specs. */
export function isBreadboard(kind: PartKind): boolean {
  return kind === 'breadboard' || kind === 'breadboard-full'
}

export interface FieldOption {
  value: string
  label: string
  /** A colour to show beside the label. */
  swatch?: string
}

/** How a field is edited. Not every property is a list of choices. */
export interface ResistanceUnit {
  label: string
  factor: number
}

/**
 * The SI ladder for resistance, smallest first. A picklist for a breadboard only
 * needs Ω, kΩ and MΩ, but the ladder is conventional, so it is written out in full
 * rather than stopped at a line someone would have to explain. Both ends are real
 * work: a current-sense shunt is mΩ, a leakage path is GΩ.
 */
export const RESISTANCE_UNITS: ResistanceUnit[] = [
  { label: 'pΩ', factor: 1e-12 },
  { label: 'nΩ', factor: 1e-9 },
  { label: 'µΩ', factor: 1e-6 },
  { label: 'mΩ', factor: 1e-3 },
  { label: 'Ω', factor: 1 },
  { label: 'kΩ', factor: 1000 },
  { label: 'MΩ', factor: 1e6 },
  { label: 'GΩ', factor: 1e9 },
  { label: 'TΩ', factor: 1e12 },
]

/**
 * The unit that keeps the number readable: 100 → "100" Ω, 4700 → "4.7" kΩ,
 * 100000 → "100" kΩ. The largest unit the value still reaches, so the number
 * never comes out as 0.0001.
 */
export function formatResistance(ohms: number): { text: string; factor: number } {
  const unit = [...RESISTANCE_UNITS].reverse().find((u) => ohms >= u.factor) ?? RESISTANCE_UNITS[0]
  return { text: String(Math.round((ohms / unit.factor) * 1000) / 1000), factor: unit.factor }
}

/**
 * What the user typed, in ohms — or null if it isn't a number at all, which the
 * caller turns into "put back what is stored" rather than a guess. Clamped, so a
 * typed 500 kΩ lands on the range the part supports and the box shows that.
 */
export function parseResistance(text: string, factor: number, min: number, max: number): number | null {
  const typed = Number(text.trim().replace(',', '.'))
  if (!Number.isFinite(typed) || typed <= 0) return null
  // To twelve significant figures, not to whole ohms: rounding 0.47 mΩ to an integer
  // gives 0, which would erase every value below an ohm and make the bottom of the
  // ladder a set of dead choices.
  const ohms = Number((typed * factor).toPrecision(12))
  return Math.min(max, Math.max(min, ohms))
}

export type FieldControl =
  | { kind: 'select'; options: FieldOption[] }
  /**
   * A resistance: a number you type, and a unit. Stored as ohms, which is what the
   * netlist wants, so the unit is a way of reading and writing it.
   */
  | { kind: 'resistance'; units: ResistanceUnit[]; min: number; max: number }
  /**
   * A value you slide rather than type — a potentiometer's wiper. Unlike the
   * others this stays live while a sketch runs, because a knob you cannot turn
   * during a run is a knob with nothing to do.
   */
  | { kind: 'range'; min: number; max: number; step: number }
  /** A sensor's on/off state, shown as a checkbox. */
  | { kind: 'toggle' }

export interface PartField {
  name: string
  label: string
  control: FieldControl
}

export interface PartDef {
  kind: PartKind
  /** Human label, shown in the palette. */
  label: string
  /** The registered custom-element tag that draws this on the canvas. */
  tag: string
  /**
   * The `type` to write into a diagram when it is not the element tag. The
   * 4-digit display uses the same Wokwi element as the 1-digit one, but needs a
   * distinct type so loading it back picks the right catalogue entry.
   */
  diagramType?: string
  /** Default attributes applied to the element (e.g. an LED's colour). */
  attrs: Record<string, string>
  /** Hand-drawn 24px line icon — the palette's fallback, not its normal look. */
  icon: string
  /**
   * Don't put this part's pins in the DOM. A breadboard has hundreds of holes;
   * hit-testing reads `pinsById` (data) anyway, so they'd be dead weight.
   */
  skipDomPins?: boolean
  /**
   * Which of this part's pins are electrically one point — a pushbutton's four
   * legs are two, a breadboard's five-hole strip is one. Anything not named here
   * is its own point. This is what lets a new component bring its own electrical
   * shape rather than the netlist growing another branch.
   */
  pinNode?: (pin: string) => string
  /**
   * The properties a user can change on a placed part. Owning this here means
   * the properties strip renders any component's controls without knowing what
   * they mean — a new component brings its own.
   */
  fields?: PartField[]
  /**
   * The part's drawing depends on a value that changes often — a slider dragged
   * through hundreds of positions, or simulation state pushed onto the element —
   * so its sprite cache needs the live trim (see `canvas.svelte.ts`), not the
   * cached-state alternation an LED or button wants.
   */
  live?: boolean
}

export const PARTS: PartDef[] = [
  {
    kind: 'uno',
    label: 'Arduino Uno',
    tag: 'wokwi-arduino-uno',
    attrs: {},
    icon: '<rect x="2.5" y="7" width="19" height="10" rx="1"/><rect x="9" y="10" width="6" height="4" rx="0.5"/><circle cx="5.5" cy="9" r="0.7"/><circle cx="7.5" cy="9" r="0.7"/><circle cx="16.5" cy="15" r="0.7"/><circle cx="18.5" cy="15" r="0.7"/>',
  },
  {
    kind: 'led',
    label: 'LED',
    tag: 'wokwi-led',
    attrs: { color: 'red' },
    // Exactly the colours the element knows how to light (`lightColors` in
    // wokwi-elements), and the swatch is its CSS colour so they read as colours.
    fields: [
      {
        name: 'color',
        label: 'Colour',
        control: {
          kind: 'select',
          options: ['red', 'green', 'blue', 'yellow', 'orange', 'white', 'purple'].map((c) => ({
            value: c,
            label: c[0].toUpperCase() + c.slice(1),
            swatch: c,
          })),
        },
      },
    ],
    icon: '<circle cx="12" cy="10" r="4"/><path d="M10 13.8V18M14 13.8V18"/><path d="M16.6 5.4l2.2-2.2M18.6 8.4l2.6-.6"/>',
  },
  {
    kind: 'rgb-led',
    label: 'RGB LED',
    tag: 'wokwi-rgb-led',
    attrs: {},
    // Three LEDs in one package. The element takes ledRed/ledGreen/ledBlue as
    // 0..1 brightness, which is exactly what the netlist computes per channel.
    live: true,
    icon: '<circle cx="12" cy="9" r="4.5"/><path d="M9 13.5V18M12 13.5V18M15 13.5V18"/><path d="M17 5l2-2M19 8l2.5-.5"/>',
  },
  {
    kind: 'resistor',
    label: 'Resistor',
    tag: 'wokwi-resistor',
    attrs: { value: '220' },
    // Plain ohms: the element derives its bands from the magnitude, so it reads
    // `value` as a number — '1k' would break it. The labels are the friendly part.
    fields: [
      {
        name: 'value',
        label: 'Resistance',
        control: { kind: 'resistance', units: RESISTANCE_UNITS, min: 1e-12, max: 1e12 },
      },
    ],
    icon: '<path d="M2 12h3.4l1.2-3 2.4 6 2.4-6 2.4 6 1.2-3H22"/>',
  },
  {
    kind: 'pushbutton',
    label: 'Pushbutton',
    tag: 'wokwi-pushbutton',
    attrs: {},
    // Four legs, two electrical nodes: the two on each side are one switch.
    pinNode: (pin) => (pin.startsWith('1') ? '1' : '2'),
    icon: '<circle cx="12" cy="13" r="4.5"/><path d="M9 9.2V6.5h6v2.7"/><path d="M4.5 13H2M19.5 13H22"/>',
  },
  {
    kind: 'potentiometer',
    label: 'Potentiometer',
    tag: 'wokwi-potentiometer',
    // Wokwi's own attribute name and scale: 0 is the GND end of the track, 1023 the
    // VCC end, so the SIG voltage is value/1023 of the supply and `analogRead` reads
    // it back unchanged at full scale.
    attrs: { value: '0' },
    fields: [
      {
        name: 'value',
        label: 'Position',
        control: { kind: 'range', min: 0, max: 1023, step: 1 },
      },
    ],
    live: true,
    icon: '<circle cx="12" cy="9" r="5.5"/><path d="M12 9l3.4-3.4"/><path d="M7 15v4M12 15.5V20M17 15v4"/>',
  },
  {
    kind: 'slide-potentiometer',
    label: 'Slide potentiometer',
    tag: 'wokwi-slide-potentiometer',
    // The element's own scale and defaults (min 0, max 100): the slider's tip sits
    // value/100 of the way along the track, so SIG is value/100 of the supply —
    // the same divider as the rotary pot, read on a matching 0–100 scale.
    attrs: { value: '0' },
    fields: [
      {
        name: 'value',
        label: 'Position',
        control: { kind: 'range', min: 0, max: 100, step: 1 },
      },
    ],
    live: true,
    icon: '<rect x="3" y="11" width="18" height="4" rx="1"/><path d="M9 11v4" stroke-linecap="round"/><path d="M6 18v2M12 18v2M18 18v2"/>',
  },
  {
    kind: 'photoresistor-sensor',
    label: 'Photoresistor',
    tag: 'wokwi-photoresistor-sensor',
    // The module is an LDR divider plus a comparator. `light` sets where its AO
    // sits between the rails (more light, higher voltage); `threshold` is where its
    // matching DO output flips. The element's `ledPower`/`ledDO` indicator lamps are
    // driven from the simulation in `App.svelte`, not stored here.
    attrs: { light: '50', threshold: '50' },
    fields: [
      {
        name: 'light',
        label: 'Light',
        control: { kind: 'range', min: 0, max: 100, step: 1 },
      },
      {
        name: 'threshold',
        label: 'DO threshold',
        control: { kind: 'range', min: 0, max: 100, step: 1 },
      },
    ],
    live: true,
    icon: '<circle cx="8" cy="9" r="2.5"/><path d="M11.5 6.5l5-2-1.8 4.7z" fill="currentColor" stroke="none" opacity=".001"/><path d="M13 6.5a6 6 0 0 1 0 5.2"/><path d="M15.5 5a9 9 0 0 1 .1 8.6"/><path d="M7 13.5v6"/>',
  },
  {
    kind: 'ntc-temperature-sensor',
    label: 'NTC temperature sensor',
    tag: 'wokwi-ntc-temperature-sensor',
    // `temperature` is degrees Celsius and OUT rises with it, across the −15…135 °C
    // range a bench would let you show: −15 °C reads 0 V, 135 °C the full supply,
    // room temperature around 27% of it. The map is deliberately plain — the teaching
    // point is "hotter reads higher", not the Steinhart–Hart polynomial.
    attrs: { temperature: '25' },
    fields: [
      {
        name: 'temperature',
        label: 'Temperature',
        control: { kind: 'range', min: -15, max: 135, step: 1 },
      },
    ],
    live: true,
    icon: '<circle cx="9" cy="11" r="3.5"/><path d="M9 14.5V19"/><path d="M15 6h4M15 9h3M15 12h4"/>',
  },
  {
    kind: 'analog-joystick',
    label: 'Analog joystick',
    tag: 'wokwi-analog-joystick',
    // The element's xValue/yValue are −1…1; the catalogue keeps a friendlier
    // 0…100 pair and App maps it onto the element. SEL is the integrated button.
    attrs: { x: '50', y: '50' },
    fields: [
      { name: 'x', label: 'Horizontal', control: { kind: 'range', min: 0, max: 100, step: 1 } },
      { name: 'y', label: 'Vertical', control: { kind: 'range', min: 0, max: 100, step: 1 } },
    ],
    live: true,
    icon: '<circle cx="12" cy="12" r="6"/><circle cx="12" cy="12" r="2"/><path d="M12 4v3M12 17v3M4 12h3M17 12h3"/>',
  },
  {
    kind: 'tilt-switch',
    label: 'Tilt switch',
    tag: 'wokwi-tilt-switch',
    // A ball in a can: the module's comparator holds OUT high when tilted.
    attrs: { tilted: '0' },
    fields: [{ name: 'tilted', label: 'Tilted', control: { kind: 'toggle' } }],
    icon: '<rect x="3" y="8" width="15" height="8" rx="1"/><circle cx="10" cy="12" r="1.5"/><path d="M18 10h3M18 14h3"/>',
  },
  {
    kind: 'pir-motion-sensor',
    label: 'PIR motion sensor',
    tag: 'wokwi-pir-motion-sensor',
    // HC-SR501-style module: OUT goes high while motion is detected.
    attrs: { motion: '0' },
    fields: [{ name: 'motion', label: 'Motion', control: { kind: 'toggle' } }],
    icon: '<circle cx="10" cy="11" r="6"/><path d="M10 5v6l4 2"/><path d="M3 7l-1-1M3 15l-1 1M17 7l1-1M17 15l1 1"/>',
  },
  {
    kind: 'flame-sensor',
    label: 'Flame sensor',
    tag: 'wokwi-flame-sensor',
    // AOUT rises with the flame reading; DOUT crosses at half scale.
    attrs: { flame: '0' },
    fields: [{ name: 'flame', label: 'Flame', control: { kind: 'range', min: 0, max: 100, step: 1 } }],
    live: true,
    icon: '<path d="M12 3c3 4-1 5 1 8 1-1 2-2 2-4 2 3 2 8-3 10-4-2-5-5-3-8 1 1 1 2 2 3 0-3 2-5 1-9z"/>',
  },
  {
    kind: 'sound-sensor',
    label: 'Sound sensor',
    tag: 'wokwi-small-sound-sensor',
    // AOUT rises with the noise reading; DOUT crosses at half scale.
    attrs: { noise: '0' },
    fields: [{ name: 'noise', label: 'Noise', control: { kind: 'range', min: 0, max: 100, step: 1 } }],
    live: true,
    icon: '<path d="M4 10v4h3l4 4V6l-4 4H4z"/><path d="M15 9c1 1 1 5 0 6M18 7c2 2 2 8 0 10"/>',
  },
  {
    kind: 'buzzer',
    label: 'Buzzer',
    tag: 'wokwi-buzzer',
    attrs: {},
    live: true,
    icon: '<path d="M8 9H5v6h3l5 4V5L8 9z"/><path d="M16 9c1.5 1.5 1.5 4.5 0 6M19 6c3 3 3 9 0 12"/>',
  },
  {
    kind: 'relay',
    label: 'Relay',
    tag: 'wokwi-ks2e-m-dc5',
    attrs: {},
    icon: '<rect x="3" y="7" width="18" height="10" rx="1"/><path d="M7 7V5M17 7V5M7 17v2M17 17v2M9 12h6"/>',
  },
  {
    kind: 'seven-segment',
    label: '7-segment display',
    tag: 'wokwi-7segment',
    attrs: {},
    live: true,
    icon: '<rect x="4" y="4" width="16" height="16" rx="1"/><path d="M8 8h8M8 12h8M8 16h8M8 8v4M16 8v4M8 12v4M16 12v4"/>',
  },
  {
    kind: 'seven-segment-4',
    label: '4-digit display',
    tag: 'wokwi-7segment-4',
    attrs: {},
    live: true,
    icon: '<rect x="2" y="5" width="20" height="14" rx="1"/><path d="M4 9h3M9 9h3M14 9h3M19 9h1M4 13h3M9 13h3M14 13h3M19 13h1M4 17h3M9 17h3M14 17h3M19 17h1"/>',
  },
  {
    kind: 'led-bar-graph',
    label: 'LED bar graph',
    tag: 'wokwi-led-bar-graph',
    attrs: { color: 'red' },
    fields: [
      {
        name: 'color',
        label: 'Colour',
        control: {
          kind: 'select',
          options: ['red', 'green', 'yellow', 'blue', 'white'].map((c) => ({
            value: c,
            label: c[0].toUpperCase() + c.slice(1),
            swatch: c,
          })),
        },
      },
    ],
    live: true,
    icon: '<path d="M5 5v14M9 8v10M13 8v10M17 5v14"/>',
  },
  {
    kind: 'breadboard',
    label: 'Breadboard',
    tag: BREADBOARDS.half.tag,
    attrs: {},
    pinNode: breadboardPinNode,
    icon: '<rect x="2" y="6" width="20" height="12" rx="1"/><path d="M2 12h20"/><circle cx="6" cy="9" r="0.7"/><circle cx="9" cy="9" r="0.7"/><circle cx="6" cy="15" r="0.7"/><circle cx="9" cy="15" r="0.7"/><circle cx="15" cy="9" r="0.7"/><circle cx="18" cy="9" r="0.7"/><circle cx="15" cy="15" r="0.7"/><circle cx="18" cy="15" r="0.7"/>',
    skipDomPins: true,
  },
  {
    kind: 'breadboard-full',
    label: 'Breadboard · full',
    tag: BREADBOARDS.full.tag,
    attrs: {},
    pinNode: breadboardPinNode,
    icon: '<rect x="1.5" y="6" width="21" height="12" rx="1"/><path d="M1.5 12h21"/><circle cx="5" cy="9" r="0.7"/><circle cx="8" cy="9" r="0.7"/><circle cx="11" cy="9" r="0.7"/><circle cx="16" cy="9" r="0.7"/><circle cx="19" cy="9" r="0.7"/><circle cx="5" cy="15" r="0.7"/><circle cx="8" cy="15" r="0.7"/><circle cx="11" cy="15" r="0.7"/><circle cx="16" cy="15" r="0.7"/><circle cx="19" cy="15" r="0.7"/>',
    skipDomPins: true,
  },
]

const BY_KIND = new Map<PartKind, PartDef>(PARTS.map((p) => [p.kind, p]))
const BY_TAG = new Map<string, PartDef>()
for (const part of PARTS) {
  // The 4-digit display shares the element tag with the 1-digit one; the plain
  // tag must keep resolving to the 1-digit kind, with the alias doing the rest.
  if (!BY_TAG.has(part.tag)) BY_TAG.set(part.tag, part)
  if (part.diagramType) BY_TAG.set(part.diagramType, part)
}

export function partByKind(kind: PartKind): PartDef {
  const def = BY_KIND.get(kind)
  if (!def) throw new Error(`unknown part kind: ${kind}`)
  return def
}

/** Reverse lookup: the part kind for a `wokwi-*` element tag, if we support it. */
export function kindForTag(tag: string): PartKind | undefined {
  return BY_TAG.get(tag)?.kind
}
