import { describe, expect, it } from 'vitest'
import { BREADBOARDS } from './breadboard'
import { evaluateCircuit, ledStep, type EvalPart, type EvalWire } from './netlist'

const uno: EvalPart = { id: 'uno', kind: 'uno' }
const led = (color = 'red'): EvalPart => ({ id: 'led', kind: 'led', attrs: { color } })
const rgb = (): EvalPart => ({ id: 'rgb', kind: 'rgb-led' })
const resistor = (value: string): EvalPart => ({ id: 'r', kind: 'resistor', attrs: { value } })
const button = (): EvalPart => ({ id: 'b', kind: 'pushbutton' })
const joystick = (x = '50', y = '50'): EvalPart => ({
  id: 'joy',
  kind: 'analog-joystick',
  attrs: { x, y },
})
const tilt = (tilted = '0'): EvalPart => ({
  id: 'tilt',
  kind: 'tilt-switch',
  attrs: { tilted },
})
const pir = (motion = '0'): EvalPart => ({
  id: 'pir',
  kind: 'pir-motion-sensor',
  attrs: { motion },
})
const flame = (value = '0'): EvalPart => ({
  id: 'flame',
  kind: 'flame-sensor',
  attrs: { flame: value },
})
const sound = (value = '0'): EvalPart => ({
  id: 'sound',
  kind: 'sound-sensor',
  attrs: { noise: value },
})
const buzzer = (): EvalPart => ({ id: 'buzzer', kind: 'buzzer' })
const relay = (): EvalPart => ({ id: 'relay', kind: 'relay' })
const sevenSegment = (): EvalPart => ({ id: 'seg', kind: 'seven-segment' })
const ledBar = (): EvalPart => ({ id: 'bar', kind: 'led-bar-graph' })

/**
 * Pin references are built one end at a time on purpose. The obvious
 * `wire(a, pinA, b, pinB)` interleaves two pairs of strings, and writing one in
 * the wrong order silently produces a circuit that doesn't connect — which cost
 * an hour today. Two `at()` calls can't be interleaved.
 */
const at = (instanceId: string, pin: string) => ({ instanceId, pin })
const wire = (a: ReturnType<typeof at>, b: ReturnType<typeof at>): EvalWire => ({ a, b })

/** A circuit with the Uno's pin 13 (port B, bit 5) driven high. */
const run = (parts: EvalPart[], wires: EvalWire[], pressed: Record<string, boolean> = {}) =>
  evaluateCircuit([uno, ...parts], wires, pressed, { B5: true })

/** The same circuit, with the pin carrying a PWM duty instead of a steady high. */
const runAtDuty = (
  parts: EvalPart[],
  wires: EvalWire[],
  duty: number,
  pressed: Record<string, boolean> = {},
) => evaluateCircuit([uno, ...parts], wires, pressed, { B5: true }, { B5: duty })

describe('ledStep', () => {
  it('quantises to the sprite steps and clamps', () => {
    expect(ledStep(-1)).toBe(0)
    expect(ledStep(2)).toBe(1)
    expect(ledStep(0)).toBe(0)
    expect(ledStep(1)).toBe(1)
    expect(ledStep(0.25)).toBeCloseTo(8 / 31)
  })
})

describe('an LED draws whatever is in series with it', () => {
  // 13 -> [resistor] -> LED -> GND
  const chain = (value?: string, colour = 'red') =>
    value
      ? run(
          [resistor(value), led(colour)],
          [
            wire(at('uno', '13'), at('r', '1')),
            wire(at('r', '2'), at('led', 'A')),
            wire(at('led', 'C'), at('uno', 'GND.2')),
          ],
        )
      : run(
          [led(colour)],
          [wire(at('uno', '13'), at('led', 'A')), wire(at('led', 'C'), at('uno', 'GND.2'))],
        )

  it('lights through a resistor in the usual range', () => {
    expect(chain('220').leds.led).toBe('lit')
    expect(chain('1000').leds.led).toBe('lit')
    expect(chain('220').ledBrightness.led).toBe(1)
  })

  it('follows PWM duty instead of flickering on and off', () => {
    const wires = [
      wire(at('uno', '13'), at('r', '1')),
      wire(at('r', '2'), at('led', 'A')),
      wire(at('led', 'C'), at('uno', 'GND.2')),
    ]
    const quarter = runAtDuty([resistor('220'), led()], wires, 0.25)
    expect(quarter.leds.led).toBe('lit')
    expect(quarter.ledBrightness.led).toBeCloseTo(0.25)

    const nearlyOff = runAtDuty([resistor('220'), led()], wires, 0.01)
    expect(nearlyOff.leds.led).toBe('off')
    expect(nearlyOff.ledBrightness.led).toBe(0)
  })

  it('dims as the resistance climbs, and goes out when it is far too high', () => {
    expect(chain('10000').leds.led).toBe('dim')
    expect(chain('100000').leds.led).toBe('off')
  })

  it('is over-driven with no resistor at all, and says how much', () => {
    const out = chain()
    expect(out.leds.led).toBe('burnt')
    expect(out.warnings.led).toMatch(/over-driven/i)
    expect(out.warnings.led).toMatch(/mA/) // a real number, not a scolding
  })

  it('is over-driven by a resistor that is too small', () => {
    expect(chain('100').leds.led).toBe('burnt')
  })

  it('draws less when the forward voltage is higher', () => {
    // The same 100R over-drives a red LED (1.8V) but not a blue one (3V).
    expect(chain('100', 'red').leds.led).toBe('burnt')
    expect(chain('100', 'blue').leds.led).toBe('lit')
  })

  it('notices one that is in backwards', () => {
    const out = run(
      [led()],
      [wire(at('uno', '13'), at('led', 'C')), wire(at('led', 'A'), at('uno', 'GND.2'))],
    )
    expect(out.leds.led).toBe('off')
    expect(out.warnings.led).toMatch(/wrong way round/i)
  })
})

describe('an RGB LED mixes three independent channels', () => {
  const channelWire = (channel: string, value = '220') => [
    resistor(value),
    rgb(),
    [
      wire(at('uno', '13'), at('r', '1')),
      wire(at('r', '2'), at('rgb', channel)),
      wire(at('rgb', 'COM'), at('uno', 'GND.2')),
    ],
  ] as const

  it('lights the channel that has a path, and leaves the others dark', () => {
    const [r, part, wires] = channelWire('R')
    const out = run([r, part], [...wires])
    expect(out.rgbLeds.rgb).toEqual({ r: 1, g: 0, b: 0 })
  })

  it('maps the green and blue pins separately', () => {
    const [r, part, wires] = channelWire('G')
    expect(run([r, part], [...wires]).rgbLeds.rgb).toEqual({ r: 0, g: 1, b: 0 })
  })

  it('uses PWM duty per channel', () => {
    const [r, part, wires] = channelWire('R')
    const out = runAtDuty([r, part], [...wires], 0.25)
    expect(out.rgbLeds.rgb.r).toBeCloseTo(0.25)
  })

  it('warns when the common leg is not grounded', () => {
    const out = run([rgb()], [wire(at('uno', '13'), at('rgb', 'R'))])
    expect(out.rgbLeds.rgb).toEqual({ r: 0, g: 0, b: 0 })
    expect(out.warnings.rgb).toMatch(/common leg/i)
  })

  it('warns when a channel has no resistor', () => {
    const out = run(
      [rgb()],
      [
        wire(at('uno', '13'), at('rgb', 'R')),
        wire(at('rgb', 'COM'), at('uno', 'GND.2')),
      ],
    )
    expect(out.warnings.rgb).toMatch(/over-driven/i)
  })
})

describe('the analog joystick', () => {
  const wires = [
    wire(at('uno', '5V'), at('joy', 'VCC')),
    wire(at('joy', 'GND'), at('uno', 'GND.2')),
    wire(at('joy', 'VERT'), at('uno', 'A0')),
    wire(at('joy', 'HORZ'), at('uno', 'A1')),
    wire(at('joy', 'SEL'), at('uno', '2')),
  ]
  const read = (x: string, y: string, pressed: Record<string, boolean> = {}) =>
    evaluateCircuit([uno, joystick(x, y)], wires, pressed, {})

  it('puts the two axes on their own ADC channels', () => {
    const out = read('0', '100')
    expect(out.analogs.find((a) => a.channel === 0)?.millivolts).toBe(5000)
    expect(out.analogs.find((a) => a.channel === 1)?.millivolts).toBe(0)
  })

  it('drives SEL high released and low while pressed', () => {
    expect(read('50', '50').drives).toEqual([{ port: 'D', pin: 2, high: true }])
    expect(read('50', '50', { joy: true }).drives).toEqual([
      { port: 'D', pin: 2, high: false },
    ])
  })

  it('warns when the module is wired but not powered', () => {
    const out = evaluateCircuit(
      [uno, joystick()],
      [wire(at('joy', 'VERT'), at('uno', 'A0')), wire(at('joy', 'SEL'), at('uno', '2'))],
      {},
      {},
    )
    expect(out.warnings.joy).toMatch(/VCC.*5V.*GND/i)
  })
})

describe('digital sensor modules', () => {
  const powered = (part: EvalPart, out: string) => [
    wire(at(part.id, 'VCC'), at('uno', '5V')),
    wire(at(part.id, 'GND'), at('uno', 'GND.2')),
    wire(at(part.id, out), at('uno', '2')),
  ]

  it('turns a tilt switch into a driven OUT level', () => {
    const level = (tilted: string) =>
      evaluateCircuit([uno, tilt(tilted)], powered(tilt(), 'OUT'), {}, {})
    expect(level('0').drives).toEqual([{ port: 'D', pin: 2, high: false }])
    expect(level('1').drives).toEqual([{ port: 'D', pin: 2, high: true }])
    expect(level('1').sensors.tilt).toEqual({ powered: true, active: true })
  })

  it('turns PIR motion into a driven OUT level', () => {
    const out = evaluateCircuit([uno, pir('1')], powered(pir(), 'OUT'), {}, {})
    expect(out.drives).toEqual([{ port: 'D', pin: 2, high: true }])
    expect(out.sensors.pir).toEqual({ powered: true, active: true })
  })

  it('splits the flame sensor into an analogue reading and a thresholded DOUT', () => {
    const wires = [
      wire(at('flame', 'VCC'), at('uno', '5V')),
      wire(at('flame', 'GND'), at('uno', 'GND.2')),
      wire(at('flame', 'AOUT'), at('uno', 'A0')),
      wire(at('flame', 'DOUT'), at('uno', '3')),
    ]
    const strong = evaluateCircuit([uno, flame('80')], wires, {}, {})
    expect(strong.analogs.find((a) => a.channel === 0)?.millivolts).toBe(4000)
    expect(strong.drives).toEqual([{ port: 'D', pin: 3, high: true }])
    const weak = evaluateCircuit([uno, flame('20')], wires, {}, {})
    expect(weak.analogs.find((a) => a.channel === 0)?.millivolts).toBe(1000)
    expect(weak.drives).toEqual([{ port: 'D', pin: 3, high: false }])
  })

  it('does the same for the sound sensor', () => {
    const wires = [
      wire(at('sound', 'VCC'), at('uno', '5V')),
      wire(at('sound', 'GND'), at('uno', 'GND.2')),
      wire(at('sound', 'AOUT'), at('uno', 'A1')),
      wire(at('sound', 'DOUT'), at('uno', '4')),
    ]
    const out = evaluateCircuit([uno, sound('60')], wires, {}, {})
    expect(out.analogs.find((a) => a.channel === 1)?.millivolts).toBe(3000)
    expect(out.drives).toEqual([{ port: 'D', pin: 4, high: true }])
  })

  it('warns when a digital sensor is wired but not powered', () => {
    const out = evaluateCircuit(
      [uno, tilt()],
      [wire(at('tilt', 'OUT'), at('uno', '2'))],
      {},
      {},
    )
    expect(out.warnings.tilt).toMatch(/VCC.*5V.*GND/i)
  })
})

describe('output parts', () => {
  it('turns a buzzer on across its two terminals', () => {
    const wires = [
      wire(at('uno', '13'), at('buzzer', '1')),
      wire(at('buzzer', '2'), at('uno', 'GND.2')),
    ]
    expect(evaluateCircuit([uno, buzzer()], wires, {}, { B5: true }).outputs.buzzer.active).toBe(true)
    expect(evaluateCircuit([uno, buzzer()], wires, {}, {}).outputs.buzzer.active).toBe(false)
  })

  it('energizes a relay coil the same way', () => {
    const wires = [
      wire(at('uno', '13'), at('relay', 'COIL1')),
      wire(at('relay', 'COIL2'), at('uno', 'GND.2')),
    ]
    expect(evaluateCircuit([uno, relay()], wires, {}, { B5: true }).outputs.relay.active).toBe(true)
  })

  it('lights seven-segment segments only when their common is low', () => {
    const wires = [
      wire(at('uno', '2'), at('seg', 'A')),
      wire(at('uno', '3'), at('seg', 'B')),
      wire(at('seg', 'COM.1'), at('uno', 'GND.2')),
    ]
    const out = evaluateCircuit([uno, sevenSegment()], wires, {}, { D2: true, D3: true })
    expect(out.outputs.seg.values).toEqual([1, 1, 0, 0, 0, 0, 0, 0])
    const floating = evaluateCircuit([uno, sevenSegment()], wires, {}, { D2: true, D3: true })
    expect(floating.outputs.seg.active).toBe(true)
  })

  it('maps each LED bar pair independently', () => {
    const wires = [
      wire(at('uno', '2'), at('bar', 'A1')),
      wire(at('bar', 'C1'), at('uno', 'GND.2')),
      wire(at('uno', '3'), at('bar', 'A2')),
      wire(at('bar', 'C2'), at('uno', 'GND.2')),
    ]
    const out = evaluateCircuit([uno, ledBar()], wires, {}, { D2: true })
    expect(out.outputs.bar.values[0]).toBe(1)
    expect(out.outputs.bar.values[1]).toBe(0)
  })
})

describe('a leg standing in a breadboard hole, with no wire of its own', () => {
  const half = BREADBOARDS.half
  const hole = (name: string) => {
    const pin = half.pins.find((p) => p.name === name)
    if (!pin) throw new Error(`no hole named ${name}`)
    return pin
  }
  const board: EvalPart = {
    id: 'bb',
    kind: 'breadboard',
    pins: half.pins.map((p) => ({ name: p.name, x: p.x, y: p.y })),
  }
  /** An LED resting its legs in two holes (its legs are 10px apart, as drawn). */
  const ledIn = (anodeHole: string): EvalPart => {
    const spot = hole(anodeHole)
    return {
      id: 'led',
      kind: 'led',
      attrs: { color: 'red' },
      pins: [
        { name: 'A', x: spot.x, y: spot.y },
        { name: 'C', x: spot.x - 10, y: spot.y },
      ],
    }
  }
  // Pin 13 feeds strip 5 of the upper bank; GND holds strip 4 low.
  const circuit = (part: EvalPart, withResistor = true) =>
    run(
      withResistor ? [board, part, resistor('220')] : [board, part],
      withResistor
        ? [
            wire(at('uno', '13'), at('r', '1')),
            wire(at('r', '2'), at('bb', 'a5')),
            wire(at('uno', 'GND.2'), at('bb', 'c4')),
          ]
        : [wire(at('uno', '13'), at('bb', 'a5')), wire(at('uno', 'GND.2'), at('bb', 'c4'))],
    )

  it('joins the strip it stands in', () => {
    expect(circuit(ledIn('c5')).leds.led).toBe('lit')
  })

  it('is genuinely connected, not just lit by a wire — with no resistor it is over-driven', () => {
    // The LED has no wire of its own: its legs are in holes c5 and c4 while the
    // wires go to *other* holes in those strips. Over-driven is only reachable if
    // the legs really are joined to the strips.
    expect(circuit(ledIn('c5'), false).leds.led).toBe('burnt')
  })

  it('does not reach the other bank, because the trench is a break', () => {
    expect(circuit(ledIn('h5')).leds.led).toBe('off')
  })

  it('needs to be in a hole, not merely near one', () => {
    const spot = hole('c5')
    const nudged: EvalPart = {
      ...ledIn('c5'),
      pins: [
        { name: 'A', x: spot.x, y: spot.y },
        { name: 'C', x: spot.x - 10 - 6, y: spot.y }, // 6px off: out of the hole's reach
      ],
    }
    expect(circuit(nudged).leds.led).toBe('off')
  })
})

describe('a pushbutton straddling the trench', () => {
  // Its legs, in the element's own coordinates: two rows 19px apart, 67px wide.
  const half = BREADBOARDS.half
  const hole = (name: string) => {
    const pin = half.pins.find((p) => p.name === name)
    if (!pin) throw new Error(`no hole named ${name}`)
    return pin
  }
  const board: EvalPart = {
    id: 'bb',
    kind: 'breadboard',
    pins: half.pins.map((p) => ({ name: p.name, x: p.x, y: p.y })),
  }
  const legsAt = hole('e1') // the last row above the trench, so 2.l/2.r land below it
  const switchPart: EvalPart = {
    id: 'b',
    kind: 'pushbutton',
    pins: [
      { name: '1.l', x: legsAt.x, y: legsAt.y },
      { name: '1.r', x: legsAt.x + 67, y: legsAt.y },
      { name: '2.l', x: legsAt.x, y: legsAt.y + 19 },
      { name: '2.r', x: legsAt.x + 67, y: legsAt.y + 19 },
    ],
  }
  // An LED across the trench: anode in column 1 of the lower bank, cathode in
  // column 2, taken to ground. Pin 13 feeds column 1 of the upper bank.
  const ledAcross: EvalPart = {
    id: 'led',
    kind: 'led',
    attrs: { color: 'red' },
    pins: [
      { name: 'A', x: hole('f1').x, y: hole('f1').y },
      { name: 'C', x: hole('f1').x + 10, y: hole('f1').y },
    ],
  }
  const circuit = (pressed: boolean) =>
    run(
      [board, switchPart, ledAcross, resistor('220')],
      [
        wire(at('uno', '13'), at('r', '1')),
        wire(at('r', '2'), at('bb', 'c1')),
        wire(at('bb', 'f2'), at('uno', 'GND.2')),
      ],
      pressed ? { b: true } : {},
    )

  it('is dark until the switch is pressed', () => {
    expect(circuit(false).leds.led).toBe('off')
  })

  it('bridges the two banks when pressed', () => {
    // The regression test for the grid: on the old geometry a pushbutton's two
    // sides shared one strip, so it was shorted and this could never light.
    expect(circuit(true).leds.led).toBe('lit')
  })
})

describe('what a pushbutton does to the pin it is wired to', () => {
  const drive = (parts: EvalPart[], wires: EvalWire[], pressed: boolean) =>
    run(parts, wires, pressed ? { b: true } : {}).drives.find(
      (d) => d.port === 'D' && d.pin === 2,
    )

  it('pulls the pin low when it shorts to ground', () => {
    const wires = [wire(at('uno', '2'), at('b', '1.l')), wire(at('b', '2.r'), at('uno', 'GND.1'))]
    expect(drive([button()], wires, true)?.high).toBe(false)
    // Released it is an open circuit, so the sketch's pull-up holds it high.
    expect(drive([button()], wires, false)?.high).toBe(true)
  })

  it('pulls the pin high when it shorts to +5V', () => {
    // The original bug: the drive was always low-on-press, so a button to +5V read
    // as released while it was held — it looked dead.
    const wires = [wire(at('uno', '2'), at('b', '1.l')), wire(at('b', '2.r'), at('uno', '5V'))]
    expect(drive([button()], wires, true)?.high).toBe(true)
  })

  it('reads low when released if a pull-down resistor is there', () => {
    const wires = [
      wire(at('uno', '2'), at('b', '1.l')),
      wire(at('b', '2.r'), at('uno', '5V')),
      wire(at('uno', '2'), at('r', '1')),
      wire(at('r', '2'), at('uno', 'GND.1')),
    ]
    const parts = [button(), resistor('10000')]
    expect(drive(parts, wires, true)?.high).toBe(true)
    expect(drive(parts, wires, false)?.high).toBe(false)
  })
})

describe('the power rails', () => {
  const boardFor = (
    id: string,
    spec: (typeof BREADBOARDS)['half'],
    kind: EvalPart['kind'],
  ): EvalPart => ({ id, kind, pins: spec.pins.map((p) => ({ name: p.name, x: p.x, y: p.y })) })

  /** An LED sitting across two holes of the upper bank. */
  const ledAt = (spec: (typeof BREADBOARDS)['half'], anode: string): EvalPart => {
    const spot = spec.pins.find((p) => p.name === anode)
    if (!spot) throw new Error(`no hole named ${anode}`)
    return {
      id: 'led',
      kind: 'led',
      attrs: { color: 'red' },
      pins: [
        { name: 'A', x: spot.x, y: spot.y },
        { name: 'C', x: spot.x - 10, y: spot.y },
      ],
    }
  }

  it('carries power the length of a half board', () => {
    const half = BREADBOARDS.half
    const out = run(
      [boardFor('bb', half, 'breadboard'), ledAt(half, 'c5'), resistor('220')],
      [
        wire(at('uno', '13'), at('r', '1')),
        wire(at('r', '2'), at('bb', 'tp20')), // drive one end of the rail…
        wire(at('bb', 'tp1'), at('bb', 'a5')), // …and feed the strip from the other end
        wire(at('uno', 'GND.2'), at('bb', 'c4')),
      ],
    )
    expect(out.leds.led).toBe('lit')
  })

  it('does not, across the break on a full board', () => {
    const full = BREADBOARDS.full
    const out = run(
      [boardFor('bb', full, 'breadboard-full'), ledAt(full, 'c5'), resistor('220')],
      [
        wire(at('uno', '13'), at('r', '1')),
        wire(at('r', '2'), at('bb', 'tpA1')), // the A half is driven…
        wire(at('bb', 'tpB1'), at('bb', 'a5')), // …but the strip is fed from the B half
        wire(at('uno', 'GND.2'), at('bb', 'c4')),
      ],
    )
    expect(out.leds.led).toBe('off')
  })
})

describe('the far ends of the resistance ladder', () => {
  // 13 -> [resistor] -> LED -> GND, the same chain the tests above use.
  const chain = (value: string) =>
    run(
      [resistor(value), led()],
      [
        wire(at('uno', '13'), at('r', '1')),
        wire(at('r', '2'), at('led', 'A')),
        wire(at('led', 'C'), at('uno', 'GND.2')),
      ],
    )

  it('reads a dead short as burnt rather than dividing into absurdity', () => {
    // A pico-ohm resistor is not a physical part. The netlist's series pin resistance
    // is what keeps this a burnt LED instead of an unbounded current — which is why
    // the bottom of the ladder is safe to offer at all.
    expect(chain('1e-12').leds.led).toBe('burnt')
    expect(chain('0.001').leds.led).toBe('burnt')
  })

  it('reads an insulator as off, not as an error', () => {
    expect(chain('1e9').leds.led).toBe('off')
    expect(chain('1e12').leds.led).toBe('off')
  })

  it('still lights normally in the middle of the ladder', () => {
    expect(chain('470').leds.led).toBe('lit')
    expect(chain('1').leds.led).toBe('burnt')
  })
})

describe('a potentiometer sets the voltage an analogue pin reads', () => {
  const pot = (value = '0'): EvalPart => ({ id: 'p1', kind: 'potentiometer', attrs: { value } })
  const wired = (value?: string) =>
    run(
      [pot(value)],
      [
        wire(at('uno', '5V'), at('p1', 'VCC')),
        wire(at('uno', 'GND.2'), at('p1', 'GND')),
        wire(at('p1', 'SIG'), at('uno', 'A0')),
      ],
    )

  it('is a fraction of the supply between its end stops', () => {
    expect(wired('0').analogs).toEqual([{ channel: 0, millivolts: 0 }])
    expect(wired('1023').analogs).toEqual([{ channel: 0, millivolts: 5000 }])
    expect(wired('512').analogs).toEqual([{ channel: 0, millivolts: 2502 }])
  })

  it('reads the other way round when its ends are swapped, the way it would on a bench', () => {
    const out = run(
      [pot('1023')],
      [
        wire(at('uno', 'GND.2'), at('p1', 'VCC')),
        wire(at('uno', '5V'), at('p1', 'GND')),
        wire(at('p1', 'SIG'), at('uno', 'A0')),
      ],
    )
    expect(out.analogs).toEqual([{ channel: 0, millivolts: 0 }])
  })

  it('says which rail is missing rather than reading nothing', () => {
    const out = run(
      [pot('512')],
      [wire(at('uno', '5V'), at('p1', 'VCC')), wire(at('p1', 'SIG'), at('uno', 'A0'))],
    )
    expect(out.analogs).toEqual([])
    expect(out.warnings.p1).toMatch(/5V.*GND/i)
  })

  it('is quiet until its wiper reaches an analogue input', () => {
    const out = run(
      [pot('512')],
      [wire(at('uno', '5V'), at('p1', 'VCC')), wire(at('uno', 'GND.2'), at('p1', 'GND'))],
    )
    expect(out.analogs).toEqual([])
    expect(out.warnings.p1).toBeUndefined()
  })

  it('feeds every analogue pin its wiper is wired to', () => {
    const out = run(
      [pot('1023')],
      [
        wire(at('uno', '5V'), at('p1', 'VCC')),
        wire(at('uno', 'GND.2'), at('p1', 'GND')),
        wire(at('p1', 'SIG'), at('uno', 'A0')),
        wire(at('uno', 'A0'), at('uno', 'A3')),
      ],
    )
    expect(out.analogs).toEqual([
      { channel: 0, millivolts: 5000 },
      { channel: 3, millivolts: 5000 },
    ])
  })
})

describe('a slide potentiometer is the same divider on the element\'s 0–100 scale', () => {
  const slider = (value = '0'): EvalPart => ({
    id: 's1',
    kind: 'slide-potentiometer',
    attrs: { value },
  })
  const wired = (value?: string) =>
    run(
      [slider(value)],
      [
        wire(at('uno', '5V'), at('s1', 'VCC')),
        wire(at('uno', 'GND.2'), at('s1', 'GND')),
        wire(at('s1', 'SIG'), at('uno', 'A0')),
      ],
    )

  it('is a fraction of the supply between its end stops', () => {
    expect(wired('0').analogs).toEqual([{ channel: 0, millivolts: 0 }])
    expect(wired('100').analogs).toEqual([{ channel: 0, millivolts: 5000 }])
    expect(wired('50').analogs).toEqual([{ channel: 0, millivolts: 2500 }])
  })

  it('reads the other way round when its ends are swapped, like the rotary pot', () => {
    const out = run(
      [slider('100')],
      [
        wire(at('uno', 'GND.2'), at('s1', 'VCC')),
        wire(at('uno', '5V'), at('s1', 'GND')),
        wire(at('s1', 'SIG'), at('uno', 'A0')),
      ],
    )
    expect(out.analogs).toEqual([{ channel: 0, millivolts: 0 }])
  })

  it('is quiet until its wiper reaches an analogue input', () => {
    const out = run(
      [slider('50')],
      [wire(at('uno', '5V'), at('s1', 'VCC')), wire(at('uno', 'GND.2'), at('s1', 'GND'))],
    )
    expect(out.analogs).toEqual([])
    expect(out.warnings.s1).toBeUndefined()
  })

  it('complains rather than reading nothing when a rail is missing', () => {
    const out = run(
      [slider('50')],
      [wire(at('uno', '5V'), at('s1', 'VCC')), wire(at('s1', 'SIG'), at('uno', 'A0'))],
    )
    expect(out.analogs).toEqual([])
    expect(out.warnings.s1).toMatch(/5V.*GND/i)
  })
})

describe('a photoresistor module divides its rails and drives DO both ways', () => {
  const light = (v: string, threshold = '50'): EvalPart => ({
    id: 'm1',
    kind: 'photoresistor-sensor',
    attrs: { light: v, threshold },
  })
  const powered = [
    wire(at('uno', '5V'), at('m1', 'VCC')),
    wire(at('uno', 'GND.2'), at('m1', 'GND')),
  ]

  it('AO sits higher with more light', () => {
    const ao = (v: string) =>
      run([light(v)], [...powered, wire(at('m1', 'AO'), at('uno', 'A0'))])
    expect(ao('0').analogs).toEqual([{ channel: 0, millivolts: 0 }])
    expect(ao('50').analogs).toEqual([{ channel: 0, millivolts: 2500 }])
    expect(ao('100').analogs).toEqual([{ channel: 0, millivolts: 5000 }])
  })

  it('DO follows light against the threshold, driving the pin high or low', () => {
    const on = run([light('60')], [...powered, wire(at('m1', 'DO'), at('uno', '2'))])
    expect(on.sensors.m1).toEqual({ powered: true, active: true })
    expect(on.drives).toContainEqual({ port: 'D', pin: 2, high: true })

    const off = run([light('40')], [...powered, wire(at('m1', 'DO'), at('uno', '2'))])
    expect(off.sensors.m1).toEqual({ powered: true, active: false })
    expect(off.drives).toContainEqual({ port: 'D', pin: 2, high: false })
  })

  it('sits high exactly at the threshold, like a comparator without hysteresis', () => {
    const out = run([light('50')], [...powered, wire(at('m1', 'DO'), at('uno', '2'))])
    expect(out.sensors.m1?.active).toBe(true)
  })

  it('drives nothing when its rails are backwards — a module is not a bare divider', () => {
    const out = run(
      [light('80')],
      [
        wire(at('uno', 'GND.2'), at('m1', 'VCC')),
        wire(at('uno', '5V'), at('m1', 'GND')),
        wire(at('m1', 'DO'), at('uno', '2')),
        wire(at('m1', 'AO'), at('uno', 'A0')),
      ],
    )
    expect(out.analogs).toEqual([])
    expect(out.drives).toEqual([])
    expect(out.sensors.m1).toBeUndefined()
    expect(out.warnings.m1).toMatch(/VCC.*5V/i)
  })

  it('stays quiet with no rails at all', () => {
    const out = run([light('80')], [
      { a: at('m1', 'AO'), b: at('uno', 'A0') },
      { a: at('m1', 'DO'), b: at('uno', '2') },
    ])
    // AO and DO reach pins, so it says what is wrong — but it does not fake readings.
    expect(out.analogs).toEqual([])
    expect(out.drives).toEqual([])
    expect(out.warnings.m1).toMatch(/VCC/i)
  })

  it('shows its lamps with no outputs wired, and drives nothing', () => {
    const out = run([light('80')], powered)
    // Powered, and light above the threshold, the module lights its own DO lamp —
    // that is the visible check it is alive — without reaching any pin.
    expect(out.sensors.m1).toEqual({ powered: true, active: true })
    expect(out.analogs).toEqual([])
    expect(out.drives).toEqual([])
    expect(out.warnings.m1).toBeUndefined()
  })
})

describe('an NTC module reads higher the hotter it is', () => {
  const thermo = (temperature: string): EvalPart => ({
    id: 't1',
    kind: 'ntc-temperature-sensor',
    attrs: { temperature },
  })
  const wired = (temperature: string) =>
    run(
      [thermo(temperature)],
      [
        wire(at('uno', '5V'), at('t1', 'VCC')),
        wire(at('uno', 'GND.2'), at('t1', 'GND')),
        wire(at('t1', 'OUT'), at('uno', 'A0')),
      ],
    )

  it('maps −15…135 °C across the supply', () => {
    expect(wired('-15').analogs).toEqual([{ channel: 0, millivolts: 0 }])
    expect(wired('25').analogs).toEqual([{ channel: 0, millivolts: 1333 }])
    expect(wired('135').analogs).toEqual([{ channel: 0, millivolts: 5000 }])
  })

  it('clamps a temperature below the documented floor', () => {
    expect(wired('-100').analogs).toEqual([{ channel: 0, millivolts: 0 }])
  })

  it('does nothing until its rails are the right way round', () => {
    const out = run(
      [thermo('25')],
      [
        wire(at('uno', 'GND.2'), at('t1', 'VCC')),
        wire(at('uno', '5V'), at('t1', 'GND')),
        wire(at('t1', 'OUT'), at('uno', 'A0')),
      ],
    )
    expect(out.analogs).toEqual([])
    expect(out.warnings.t1).toMatch(/GND/i)
  })
})
