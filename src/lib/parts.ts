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
  | 'resistor'
  | 'pushbutton'
  | 'potentiometer'
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
    icon: '<circle cx="12" cy="9" r="5.5"/><path d="M12 9l3.4-3.4"/><path d="M7 15v4M12 15.5V20M17 15v4"/>',
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
const BY_TAG = new Map<string, PartDef>(PARTS.map((p) => [p.tag, p]))

export function partByKind(kind: PartKind): PartDef {
  const def = BY_KIND.get(kind)
  if (!def) throw new Error(`unknown part kind: ${kind}`)
  return def
}

/** Reverse lookup: the part kind for a `wokwi-*` element tag, if we support it. */
export function kindForTag(tag: string): PartKind | undefined {
  return BY_TAG.get(tag)?.kind
}
