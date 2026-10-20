import { isBreadboard, partByKind, type PartKind } from './parts'

/**
 * A tiny electrical model for the circuit.
 *
 * We don't simulate analogue electronics — just connectivity. Pins joined by a
 * wire (or through a resistor, or through a *pressed* button) form a "net". A
 * net is HIGH if something drives it high (an AVR pin that's high, or 5V) and
 * LOW if something pulls it down (GND, or an AVR pin that's low). That's enough
 * to light an LED and to read a button — which is all v0.1 needs.
 */

import { BREADBOARD_PITCH } from './breadboard'

export type PinRef = { instanceId: string; pin: string }

/**
 * How close a leg has to be to a hole to count as inserted. Slightly under half
 * a hole's pitch, so a leg can only join the hole it's actually standing in.
 */
const HOLE_REACH = BREADBOARD_PITCH * 0.45

/** Uno pin name → AVR port/pin. */
const UNO_DIGITAL: Record<string, [string, number]> = {
  '0': ['D', 0],
  '1': ['D', 1],
  '2': ['D', 2],
  '3': ['D', 3],
  '4': ['D', 4],
  '5': ['D', 5],
  '6': ['D', 6],
  '7': ['D', 7],
  '8': ['B', 0],
  '9': ['B', 1],
  '10': ['B', 2],
  '11': ['B', 3],
  '12': ['B', 4],
  '13': ['B', 5],
  A0: ['C', 0],
  A1: ['C', 1],
  A2: ['C', 2],
  A3: ['C', 3],
  A4: ['C', 4],
  A5: ['C', 5],
}

/**
 * The analogue inputs and the ADC channel each maps to. Arduino's `analogRead`
 * numbers A0…A5 as 14…19 and subtracts 14, so the channel is the pin's own
 * index — the same order the port map above gives (A0 → C0 … A5 → C5).
 */
const UNO_ANALOG_CHANNELS: [string, number][] = [
  ['A0', 0],
  ['A1', 1],
  ['A2', 2],
  ['A3', 3],
  ['A4', 4],
  ['A5', 5],
]

export type UnoPin =
  | { kind: 'mcu'; port: string; pin: number }
  | { kind: 'rail'; rail: 'GND' | 'VCC' }
  | null

export function unoPin(pin: string): UnoPin {
  const digital = UNO_DIGITAL[pin]
  if (digital) return { kind: 'mcu', port: digital[0], pin: digital[1] }
  if (pin.startsWith('GND')) return { kind: 'rail', rail: 'GND' }
  if (pin === '5V' || pin === '3.3V' || pin === 'VIN') return { kind: 'rail', rail: 'VCC' }
  return null
}

/**
 * Pins that are electrically one and the same point on a part. The part's own
 * catalogue entry says how — so a new component brings its own electrical shape
 * and this file doesn't grow a branch for it.
 */
function nodeName(kind: PartKind, pin: string): string {
  return partByKind(kind).pinNode?.(pin) ?? pin
}

export interface EvalPart {
  id: string
  kind: PartKind
  /**
   * The part's pins in canvas space, when the caller knows them. Needed so a leg
   * resting in a breadboard hole can be seen to be in it.
   */
  pins?: { name: string; x: number; y: number }[]
  /** The part's own attributes (a resistor's value, an LED's colour). */
  attrs?: Record<string, string>
}

/** How hard an LED is being driven. `burnt` means over-driven *right now*. */
export type LedState = 'off' | 'dim' | 'lit' | 'burnt'

/**
 * The number of brightness steps the renderer caches. The simulated brightness is
 * continuous; a sprite per exact duty would be unbounded, so both the element and
 * its sprite key use the same quantised value.
 */
export const LED_STEPS = 31

export function ledStep(brightness: number): number {
  return Math.round(Math.min(1, Math.max(0, brightness)) * LED_STEPS) / LED_STEPS
}

export interface EvalWire {
  a: PinRef
  b: PinRef
}

/**
 * What a two-output sensor module reports. `powered` means its rails are wired
 * the right way round; `active` means its digital output (if it has one) is high.
 */
export interface SensorState {
  powered: boolean
  active: boolean
}

/** Brightness of an RGB LED's three channels, 0..1. */
export interface RgbState {
  r: number
  g: number
  b: number
}

export interface CircuitEval {
  /** LED instance id → how hard it's driven. */
  leds: Record<string, LedState>
  /** LED instance id → brightness 0..1, so a PWM LED fades instead of flickering. */
  ledBrightness: Record<string, number>
  /** RGB LED instance id → per-channel brightness 0..1. */
  rgbLeds: Record<string, RgbState>
  /** Buttons pulled this pin to a level (because a button connects it to a rail). */
  drives: { port: string; pin: number; high: boolean }[]
  /** What's wrong with a part — for the properties strip and the canvas badge. */
  warnings: Record<string, string>
  /**
   * Voltages to hold on the chip's ADC inputs, in millivolts (the unit simavr
   * wants). One entry per analogue pin a part's output pin reaches.
   */
  analogs: { channel: number; millivolts: number }[]
  /** Sensor modules that have an on/off output, by part id. */
  sensors: Record<string, SensorState>
}

// A small, honest model rather than a real circuit solver: enough that a
// resistor *does* something (it's in series with the LED, so it sets the
// current) and that a missing one shows up the way it does on a bench.
/** The Uno runs at 5V, and an AVR output pin has a few tens of ohms of its own. */
const SUPPLY_V = 5
const PIN_OHMS = 25
/** Typical forward voltages, so an LED's colour means something electrically. */
const LED_FORWARD_V: Record<string, number> = {
  red: 1.8,
  orange: 2,
  yellow: 2.1,
  green: 2.2,
  blue: 3,
  white: 3,
  purple: 3,
}
/** Over-driven (a 5mm LED is rated around 20–25mA), comfortably lit, barely on. */
const LED_BURN_MA = 25
const LED_LIT_MA = 3
const LED_DIM_MA = 0.2

/**
 * A voltage: clamped to 0..1, or 0 when the attribute never parsed. An empty or
 * untyped slider is the bottom of its range rather than a guess.
 */
function fractionOf(raw: unknown, top: number): number {
  const n = Number(raw)
  if (!Number.isFinite(n)) return 0
  return Math.min(1, Math.max(0, n / top))
}

/**
 * The parts whose output pin presents a voltage between two of their own rails —
 * one table entry per part, and a new analogue part is a row, not a branch.
 *
 * A symmetric source is a bare divider (a pot can sit either way round, and then
 * its track reads the other way, exactly as it would on a bench). An asymmetric
 * one is a powered module, where reversing VCC and GND is a wiring mistake, not
 * a convention: it does nothing until the rails land the right way.
 */
const ANALOG_SOURCES: {
  kind: PartKind
  /** The pin that carries the divided voltage. */
  out: string
  symmetric: boolean
  /** Position along the part's own track, 0 at its GND end and 1 at its VCC end. */
  fractionOf(attrs?: Record<string, string>): number
  /** Optional on/off output driven by this part's own reading. */
  onOff?: {
    pin: string
    /** Whether the comparator holds this output high. */
    active(attrs?: Record<string, string>): boolean
  }
}[] = [
  {
    kind: 'potentiometer',
    out: 'SIG',
    symmetric: true,
    fractionOf: (attrs) => fractionOf(attrs?.value, 1023),
  },
  {
    kind: 'slide-potentiometer',
    out: 'SIG',
    symmetric: true,
    // The element's own scale: min 0, max 100, the tip value/100 along the track.
    fractionOf: (attrs) => fractionOf(attrs?.value, 100),
  },
  {
    kind: 'photoresistor-sensor',
    out: 'AO',
    symmetric: false,
    fractionOf: (attrs) => fractionOf(attrs?.light, 100),
    // More light than the threshold, and the comparator holds DO high.
    onOff: {
      pin: 'DO',
      active: (attrs) => fractionOf(attrs?.light, 100) >= fractionOf(attrs?.threshold, 100),
    },
  },
  {
    kind: 'ntc-temperature-sensor',
    out: 'OUT',
    symmetric: false,
    // Hotter reads higher, across the −15…135 °C range the catalogue documents.
    fractionOf: (attrs) => fractionOf((Number(attrs?.temperature) ?? 25) + 15, 150),
  },
]

/** Union-find over pin keys. */
function makeUnionFind() {
  const parent = new Map<string, string>()
  const find = (k: string): string => {
    let root = k
    while (parent.has(root) && parent.get(root) !== root) root = parent.get(root) as string
    // path-compress
    let cur = k
    while (cur !== root) {
      const next = parent.get(cur) as string
      parent.set(cur, root)
      cur = next
    }
    return root
  }
  return {
    find,
    union(a: string, b: string) {
      if (!parent.has(a)) parent.set(a, a)
      if (!parent.has(b)) parent.set(b, b)
      const ra = find(a)
      const rb = find(b)
      if (ra !== rb) parent.set(ra, rb)
    },
  }
}

export function evaluateCircuit(
  parts: EvalPart[],
  wires: EvalWire[],
  pressed: Record<string, boolean>,
  avr: Record<string, boolean>,
  pinDuty: Record<string, number> = {},
): CircuitEval {
  const kindById = new Map(parts.map((p) => [p.id, p.kind]))
  const key = (ref: PinRef) => {
    const kind = kindById.get(ref.instanceId) ?? 'led'
    return `${ref.instanceId}:${nodeName(kind, ref.pin)}`
  }

  const uf = makeUnionFind()

  // Current can flow along a wire, or through a closed button. A resistor is NOT
  // a wire: its two legs stay separate nets and it becomes a resistive edge
  // below. That single change is what gives it a job.
  for (const w of wires) uf.union(key(w.a), key(w.b))
  for (const part of parts) {
    if (part.kind === 'pushbutton' && pressed[part.id]) {
      uf.union(`${part.id}:1`, `${part.id}:2`)
    }
  }

  // A leg standing in a breadboard hole is connected — that's the whole point of
  // a breadboard, and it needs no wire. Any pin within a hair of a hole joins
  // that hole's net (which already carries the rest of its strip).
  const boards = parts.filter((p) => isBreadboard(p.kind) && p.pins?.length)
  if (boards.length) {
    const reach = HOLE_REACH
    const reachSq = reach * reach
    for (const part of parts) {
      if (isBreadboard(part.kind) || !part.pins?.length) continue
      for (const pin of part.pins) {
        let hit: { key: string; d: number } | null = null
        for (const board of boards) {
          for (const hole of board.pins ?? []) {
            // Reject on one axis first, then compare squared distances. This runs
            // per pin per hole on every frame of a drag (830 holes × every pin),
            // and Math.hypot here measured at ~2ms a frame.
            const dx = hole.x - pin.x
            if (dx < -reach || dx > reach) continue
            const dy = hole.y - pin.y
            if (dy < -reach || dy > reach) continue
            const d = dx * dx + dy * dy
            if (d <= reachSq && (!hit || d < hit.d)) {
              hit = { key: `${board.id}:${nodeName('breadboard', hole.name)}`, d }
            }
          }
        }
        if (hit) uf.union(`${part.id}:${nodeName(part.kind, pin.name)}`, hit.key)
      }
    }
  }

  // A resistor is an edge between two nets with a resistance, not a wire. Walking
  // these edges is how we find how much sits in series with an LED.
  const resistorEdges = new Map<string, { to: string; ohms: number }[]>()
  const addEdge = (from: string, to: string, ohms: number) => {
    const list = resistorEdges.get(from) ?? []
    list.push({ to, ohms })
    resistorEdges.set(from, list)
  }
  for (const part of parts) {
    if (part.kind !== 'resistor') continue
    const ohms = Number(part.attrs?.value ?? 220)
    if (!Number.isFinite(ohms) || ohms <= 0) continue
    const a = uf.find(`${part.id}:1`)
    const b = uf.find(`${part.id}:2`)
    addEdge(a, b, ohms)
    addEdge(b, a, ohms)
  }

  /**
   * The path with the least resistance between one net and any of `targets`,
   * following resistor edges only — an LED is a load, not a conductor. Fewest hops
   * wins, which is exact for the series chains people actually build. Null: no path.
   */
  function reachTo(from: string, targets: Set<string>): { ohms: number; net: string } | null {
    const seen = new Map<string, { prev: string; ohms: number } | null>([[from, null]])
    const queue: string[] = [from]
    for (let i = 0; i < queue.length; i++) {
      const net = queue[i]!
      if (targets.has(net)) {
        let ohms = 0
        let step = seen.get(net)
        while (step) {
          ohms += step.ohms
          step = seen.get(step.prev)
        }
        return { ohms, net }
      }
      for (const edge of resistorEdges.get(net) ?? []) {
        if (seen.has(edge.to)) continue
        seen.set(edge.to, { prev: net, ohms: edge.ohms })
        queue.push(edge.to)
      }
    }
    return null
  }

  function resistanceTo(from: string, targets: Set<string>): number | null {
    return reachTo(from, targets)?.ohms ?? null
  }

  // What does each net carry?
  const vcc = new Set<string>()
  const gnd = new Set<string>()
  const mcuHigh = new Set<string>()
  const mcuLow = new Set<string>()
  const mcuPinOf = new Map<string, { port: string; pin: number }>()
  const dutyByRoot = new Map<string, number>()

  for (const part of parts) {
    if (part.kind !== 'uno') continue
    for (const pin of Object.keys(UNO_DIGITAL)) {
      const info = unoPin(pin)
      if (!info || info.kind !== 'mcu') continue
      const root = uf.find(`${part.id}:${nodeName('uno', pin)}`)
      const high = avr[`${info.port}${info.pin}`] === true
      ;(high ? mcuHigh : mcuLow).add(root)
      if (high) {
        // A PWM pin is still a source for connectivity; the duty says how hard.
        const key = `${info.port}${info.pin}`
        const duty = Math.min(1, Math.max(0, pinDuty[key] ?? 1))
        dutyByRoot.set(root, Math.max(dutyByRoot.get(root) ?? 0, duty))
      }
      mcuPinOf.set(root + `/${info.port}${info.pin}`, { port: info.port, pin: info.pin })
      if (pin.startsWith('A') || pin === '5V') continue
    }
  }
  // Power rails: any Uno pin named GND / 5V that's actually wired into a net.
  // (We join them as they appear in wires.)
  for (const w of wires) {
    for (const ref of [w.a, w.b]) {
      if (kindById.get(ref.instanceId) !== 'uno') continue
      const info = unoPin(ref.pin)
      if (!info || info.kind !== 'rail') continue
      const root = uf.find(key(ref))
      if (info.rail === 'VCC') vcc.add(root)
      else gnd.add(root)
    }
  }

  const netHigh = (root: string) => vcc.has(root) || mcuHigh.has(root)
  const netLow = (root: string) => gnd.has(root) || mcuLow.has(root)

  const highNets = new Set([...vcc, ...mcuHigh])
  const lowNets = new Set([...gnd, ...mcuLow])

  // LEDs: how much current actually flows. A resistor in series sets it, so this
  // is where a resistor stops being decoration — and where a missing one shows.
  const leds: Record<string, LedState> = {}
  const ledBrightness: Record<string, number> = {}
  const rgbLeds: Record<string, RgbState> = {}
  const warnings: Record<string, string> = {}
  for (const part of parts) {
    if (part.kind !== 'led') continue
    const a = uf.find(`${part.id}:A`)
    const c = uf.find(`${part.id}:C`)
    const supplyPath = reachTo(a, highNets)
    const backPath = reachTo(c, lowNets)
    const supply = supplyPath?.ohms ?? null
    const back = backPath?.ohms ?? null
    if (supply === null || back === null) {
      leds[part.id] = 'off'
      ledBrightness[part.id] = 0
      // No path either way — but the *reverse* works, so it's in backwards.
      if (
        supply === null &&
        back === null &&
        resistanceTo(a, lowNets) !== null &&
        resistanceTo(c, highNets) !== null
      ) {
        warnings[part.id] = 'Wired the wrong way round — swap its legs.'
      }
      continue
    }
    const ohms = PIN_OHMS + supply + back
    const forwardV = LED_FORWARD_V[part.attrs?.color ?? ''] ?? 2
    const mA = ((SUPPLY_V - forwardV) / ohms) * 1000
    // A PWM pin is high for only part of each period. The average current is
    // what heats the LED and what the eye sees; the pulse still carries the
    // peak, but the teaching model is about the average.
    const duty = supplyPath ? (dutyByRoot.get(supplyPath.net) ?? 1) : 0
    const averageMa = mA * duty
    if (averageMa >= LED_BURN_MA) {
      leds[part.id] = 'burnt'
      ledBrightness[part.id] = 0
      warnings[part.id] = `Over-driven — ${Math.round(averageMa)} mA with nothing to limit it. Put a resistor in series.`
    } else {
      const state = averageMa >= LED_LIT_MA ? 'lit' : averageMa >= LED_DIM_MA ? 'dim' : 'off'
      leds[part.id] = state
      const base = mA >= LED_LIT_MA ? 1 : mA / LED_LIT_MA
      ledBrightness[part.id] = state === 'off' ? 0 : Math.min(1, duty * base)
    }
  }

  // RGB LEDs are three separate loads sharing one common leg. Each colour has
  // its own resistor and PWM duty, so the channel brightness is independent.
  const RGB_CHANNELS = [
    { pin: 'R', key: 'r', forward: 1.8 },
    { pin: 'G', key: 'g', forward: 2.2 },
    { pin: 'B', key: 'b', forward: 3 },
  ] as const
  for (const part of parts) {
    if (part.kind !== 'rgb-led') continue
    const com = uf.find(`${part.id}:COM`)
    const backPath = reachTo(com, lowNets)
    const back = backPath?.ohms ?? null
    const out: RgbState = { r: 0, g: 0, b: 0 }
    if (back === null) {
      rgbLeds[part.id] = out
      warnings[part.id] = 'Wire the common leg to GND.'
      continue
    }
    let overDriven = false
    for (const channel of RGB_CHANNELS) {
      const supplyPath = reachTo(uf.find(`${part.id}:${channel.pin}`), highNets)
      if (!supplyPath) continue
      const ohms = PIN_OHMS + supplyPath.ohms + back
      const mA = ((SUPPLY_V - channel.forward) / ohms) * 1000
      const duty = dutyByRoot.get(supplyPath.net) ?? 1
      const averageMa = mA * duty
      if (averageMa >= LED_BURN_MA) overDriven = true
      const base = mA >= LED_LIT_MA ? 1 : mA / LED_LIT_MA
      out[channel.key] = averageMa < LED_DIM_MA ? 0 : Math.min(1, duty * base)
    }
    rgbLeds[part.id] = out
    if (overDriven) {
      warnings[part.id] = 'Over-driven — put a resistor on each colour.'
    }
  }

  // Analogue in. Every entry in ANALOG_SOURCES is a divider between the two rails
  // it reaches, and its output pin sets the voltage on whatever analogue pin
  // shares its net. That voltage is what the ADC reads.
  const analogs: CircuitEval['analogs'] = []
  const sensors: CircuitEval['sensors'] = {}
  const analogChannels = new Map<string, number[]>()
  for (const part of parts) {
    if (part.kind !== 'uno') continue
    for (const [pin, channel] of UNO_ANALOG_CHANNELS) {
      const root = uf.find(`${part.id}:${nodeName('uno', pin)}`)
      const list = analogChannels.get(root) ?? []
      list.push(channel)
      analogChannels.set(root, list)
    }
  }
  /** Does this pin's net reach an MCU pin at all (so a complaint is worth making)? */
  const reachesPin = (part: EvalPart, pin: string): boolean => {
    const root = uf.find(`${part.id}:${pin}`)
    for (const label of mcuPinOf.keys()) if (label.startsWith(root + '/')) return true
    return false
  }
  for (const def of ANALOG_SOURCES) {
    for (const part of parts) {
      if (part.kind !== def.kind) continue
      const highSide = uf.find(`${part.id}:VCC`)
      const lowSide = uf.find(`${part.id}:GND`)
      // A bare divider can sit either way round; a module's rails cannot.
      const forward = netHigh(highSide) && netLow(lowSide)
      const reverse = def.symmetric && netHigh(lowSide) && netLow(highSide)
      const powered = forward || reverse
      if (!powered) {
        // A quiet hint the moment its output reaches a pin that would read it, and
        // nothing at all while it isn't wired up — an unpowered module on the
        // shelf is not an error.
        if (reachesPin(part, def.out) || (def.onOff && reachesPin(part, def.onOff.pin))) {
          warnings[part.id] = def.symmetric
            ? 'Put one end on 5V and the other on GND.'
            : 'Its VCC pin needs 5V and its GND pin needs GND.'
        }
        continue
      }
      if (def.onOff) {
        sensors[part.id] = { powered: true, active: def.onOff.active(part.attrs) }
      }
      const raw = def.fractionOf(part.attrs)
      // Swapped ends read the other way round, exactly as they would on a bench.
      const position = forward ? raw : 1 - raw
      const millivolts = Math.round(SUPPLY_V * 1000 * position)
      const channels = analogChannels.get(uf.find(`${part.id}:${def.out}`))
      for (const channel of channels ?? []) analogs.push({ channel, millivolts })
    }
  }

  // The joystick is two independent dividers on VERT/HORZ plus a button on SEL.
  // Its rails are a module, so a reversed supply is a warning, not a mirrored axis.
  const JOYSTICK_AXES = [
    { pin: 'VERT', attr: 'y' },
    { pin: 'HORZ', attr: 'x' },
  ] as const
  for (const part of parts) {
    if (part.kind !== 'analog-joystick') continue
    const highSide = uf.find(`${part.id}:VCC`)
    const lowSide = uf.find(`${part.id}:GND`)
    const powered = netHigh(highSide) && netLow(lowSide)
    if (!powered) {
      if (
        reachesPin(part, 'VERT') ||
        reachesPin(part, 'HORZ') ||
        reachesPin(part, 'SEL')
      ) {
        warnings[part.id] = 'Its VCC pin needs 5V and its GND pin needs GND.'
      }
      continue
    }
    for (const axis of JOYSTICK_AXES) {
      const fraction = fractionOf(part.attrs?.[axis.attr], 100)
      const channels = analogChannels.get(uf.find(`${part.id}:${axis.pin}`))
      for (const channel of channels ?? []) {
        analogs.push({ channel, millivolts: Math.round(SUPPLY_V * 1000 * fraction) })
      }
    }
  }

  // Buttons: whichever side reaches an MCU pin drives it. Pressed, the switch is a
  // short to whichever rail it reaches — GND pulls the pin low, +5V pulls it high.
  // Released, it's an open circuit, so the pin's level is whatever else holds it:
  // a pull-down resistor means low, otherwise the sketch's pull-up means high.
  // (simavr has no "release", so we drive the level it would settle at.)
  const drives: CircuitEval['drives'] = []
  for (const part of parts) {
    if (part.kind !== 'pushbutton') continue
    const roots: Record<string, string> = { '1': uf.find(`${part.id}:1`), '2': uf.find(`${part.id}:2`) }
    // When pressed, both sides are one net — don't drive the same pin twice.
    const sides = roots['1'] === roots['2'] ? (['1'] as const) : (['1', '2'] as const)
    for (const side of sides) {
      const other = side === '1' ? '2' : '1'
      for (const [label, pins] of mcuPinOf) {
        if (!label.startsWith(roots[side] + '/')) continue
        // Pressed, the switch is a short to whichever rail the other side reaches:
        // GND pulls the pin low, +5V pulls it high, and a side reaching no rail at all
        // leaves the pin to its pull-up, exactly as when released.
        //
        // Deliberately a question about the *rail*, not netHigh(roots[other]). Pressing
        // makes the button's two sides one net, so that net contains the MCU pin as
        // well as the rail — and netHigh() would let the pin's own driven level answer
        // for itself. It latched high: a pressed button computed "drive high" forever
        // and never pulled anything low.
        const high = pressed[part.id]
          ? !gnd.has(roots[other])
          : resistanceTo(roots[side], gnd) === null
        drives.push({ port: pins.port, pin: pins.pin, high })
      }
    }
  }

  // Sensor comparator outputs, unlike a pressed button, *drive* their level both
  // ways: DO held low pulls the pin low, held high drives it high. An unpowered
  // module drives nothing, so its pin settles at the sketch's pull-up as always.
  for (const def of ANALOG_SOURCES) {
    if (!def.onOff) continue
    for (const part of parts) {
      if (part.kind !== def.kind) continue
      const state = sensors[part.id]
      if (!state?.powered) continue
      const root = uf.find(`${part.id}:${def.onOff.pin}`)
      for (const [label, pins] of mcuPinOf) {
        if (!label.startsWith(root + '/')) continue
        drives.push({ port: pins.port, pin: pins.pin, high: state.active })
      }
    }
  }

  // The joystick's SEL switch has a pull-up on the module: released it drives the
  // pin high, pressed it pulls it low. It is a powered module, so an unpowered one
  // drives nothing at all.
  for (const part of parts) {
    if (part.kind !== 'analog-joystick') continue
    const powered =
      netHigh(uf.find(`${part.id}:VCC`)) && netLow(uf.find(`${part.id}:GND`))
    if (!powered) continue
    const root = uf.find(`${part.id}:SEL`)
    for (const [label, pins] of mcuPinOf) {
      if (!label.startsWith(root + '/')) continue
      drives.push({ port: pins.port, pin: pins.pin, high: !pressed[part.id] })
    }
  }

  return { leds, ledBrightness, rgbLeds, drives, warnings, analogs, sensors }
}
