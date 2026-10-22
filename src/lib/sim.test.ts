import { describe, expect, it } from 'vitest'
import { Sim, type SimBackend } from './sim.svelte'
import type { EvalPart, EvalWire } from './netlist'

class FakeBackend implements SimBackend {
  desktop = true
  compiled: string[] = []
  started: { elf: string; bounce: boolean }[] = []
  stopped = 0
  inputs: [number, number, boolean][] = []
  pulses: [number, number, number, number][] = []
  analogs: [number, number][] = []
  serialWrites: string[] = []
  failCompile = ''
  unsubscribed = 0
  pinCb: ((port: number, pin: number, high: boolean) => void) | null = null
  timingCb: ((port: number, pin: number, duty: number, high: number, period: number) => void) | null =
    null
  serialCb: ((byte: number) => void) | null = null

  isDesktop() {
    return this.desktop
  }
  async compile(sketch: string) {
    if (this.failCompile) throw new Error(this.failCompile)
    this.compiled.push(sketch)
    return { elf: '/tmp/sketch.elf' }
  }
  async start(elf: string, bounce: boolean) {
    this.started.push({ elf, bounce })
  }
  async stop() {
    this.stopped++
  }
  async setInput(port: number, pin: number, high: boolean) {
    this.inputs.push([port, pin, high])
  }
  async pulseInput(port: number, pin: number, delayMicros: number, widthMicros: number) {
    this.pulses.push([port, pin, delayMicros, widthMicros])
  }
  async setAnalog(channel: number, millivolts: number) {
    this.analogs.push([channel, millivolts])
  }
  async writeSerial(text: string) {
    this.serialWrites.push(text)
  }
  async onPin(cb: (port: number, pin: number, high: boolean) => void) {
    this.pinCb = cb
    return () => {
      this.unsubscribed++
    }
  }
  async onTiming(cb: (port: number, pin: number, duty: number, high: number, period: number) => void) {
    this.timingCb = cb
    return () => {
      this.unsubscribed++
    }
  }
  async onSerial(cb: (byte: number) => void) {
    this.serialCb = cb
    return () => {
      this.unsubscribed++
    }
  }
}

/** An Uno with a button wired from pin 2 to GND. */
const parts: EvalPart[] = [
  { id: 'uno', kind: 'uno' },
  { id: 'b1', kind: 'pushbutton' },
]
const wires: EvalWire[] = [
  { a: { instanceId: 'uno', pin: '2' }, b: { instanceId: 'b1', pin: '1' } },
  { a: { instanceId: 'b1', pin: '2' }, b: { instanceId: 'uno', pin: 'GND.3' } },
]

describe('Sim.run', () => {
  it('compiles and starts, clearing everything a previous run left behind', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    sim.serial = 'old output'
    sim.buildError = 'old error'
    sim.pin13 = true
    sim.avrPins = { B5: true }
    sim.pressedButtons = { b1: true }

    expect(await sim.run('void setup(){}', true)).toBe(true)
    expect(backend.compiled).toEqual(['void setup(){}'])
    expect(backend.started).toEqual([{ elf: '/tmp/sketch.elf', bounce: true }])
    expect(sim.running).toBe(true)
    expect(sim.serial).toBe('')
    expect(sim.buildError).toBe('')
    expect(sim.pin13).toBe(false)
    expect(sim.avrPins).toEqual({})
    expect(sim.pressedButtons).toEqual({})
  })

  it('reports a compile failure and stays stopped', async () => {
    const backend = new FakeBackend()
    backend.failCompile = 'sketch.cpp:3: error: nope'
    const sim = new Sim(backend)
    expect(await sim.run('bad', false)).toBe(false)
    expect(sim.running).toBe(false)
    expect(sim.buildError).toContain('nope')
    expect(backend.started).toHaveLength(0)
  })

  it('stop resets the levels a stopped chip no longer drives', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('void setup(){}', false)
    sim.pin13 = true
    sim.avrPins = { B5: true }
    await sim.stop()
    expect(backend.stopped).toBe(1)
    expect(sim.running).toBe(false)
    expect(sim.pin13).toBe(false)
    expect(sim.avrPins).toEqual({})
  })

  it('restart stops before it starts again', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('one', false)
    await sim.restart('two', true)
    expect(backend.stopped).toBe(1)
    expect(backend.compiled).toEqual(['one', 'two'])
    expect(backend.started.at(-1)?.bounce).toBe(true)
  })
})

describe('Sim.solve', () => {
  it('pushes a changed button drive once, and the release too', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('void setup(){}', false)

    sim.pressedButtons.b1 = true
    sim.solve(parts, wires)
    expect(backend.inputs).toEqual([[68, 2, false]]) // 'D' = 68

    // Nothing changed: the same level is not sent again.
    sim.solve(parts, wires)
    expect(backend.inputs).toHaveLength(1)

    sim.pressedButtons.b1 = false
    sim.solve(parts, wires)
    expect(backend.inputs).toEqual([
      [68, 2, false],
      [68, 2, true],
    ])
  })

  it('holds a potentiometer voltage on the ADC channel, and only when it changes', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('void setup(){}', false)

    const parts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'p1', kind: 'potentiometer', attrs: { value: '512' } },
    ]
    const wires: EvalWire[] = [
      { a: { instanceId: 'p1', pin: 'VCC' }, b: { instanceId: 'uno', pin: '5V' } },
      { a: { instanceId: 'p1', pin: 'GND' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'p1', pin: 'SIG' }, b: { instanceId: 'uno', pin: 'A0' } },
    ]

    sim.solve(parts, wires)
    expect(backend.analogs).toEqual([[0, 2502]]) // 512/1023 of 5 V

    // Nothing changed: the same voltage is not sent again.
    sim.solve(parts, wires)
    expect(backend.analogs).toHaveLength(1)

    parts[1].attrs = { value: '1023' }
    sim.solve(parts, wires)
    expect(backend.analogs).toEqual([
      [0, 2502],
      [0, 5000],
    ])
  })

  it('re-sends held inputs to the fresh simulator after a restart', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('void setup(){}', false)

    const potParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'p1', kind: 'potentiometer', attrs: { value: '512' } },
    ]
    const potWires: EvalWire[] = [
      { a: { instanceId: 'p1', pin: 'VCC' }, b: { instanceId: 'uno', pin: '5V' } },
      { a: { instanceId: 'p1', pin: 'GND' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'p1', pin: 'SIG' }, b: { instanceId: 'uno', pin: 'A0' } },
    ]

    sim.solve(potParts, potWires)
    sim.pressedButtons.b1 = true
    sim.solve(parts, wires)
    expect(backend.analogs).toEqual([[0, 2502]])
    expect(backend.inputs).toEqual([[68, 2, false]])

    // The restart starts a new simulator, which knows neither level. The cache
    // that suppresses duplicate sends must be cleared with it.
    await sim.restart('void setup(){}', false)
    sim.pressedButtons.b1 = true
    sim.solve(potParts, potWires)
    sim.solve(parts, wires)
    expect(backend.analogs).toEqual([
      [0, 2502],
      [0, 2502],
    ])
    expect(backend.inputs).toEqual([
      [68, 2, false],
      [68, 2, false],
    ])
  })

  it('sends serial input only while a desktop simulation is running', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)

    await sim.sendSerial('too early')
    expect(backend.serialWrites).toEqual([])

    await sim.run('void setup(){}', false)
    await sim.sendSerial('hello')
    expect(backend.serialWrites).toEqual(['hello'])

    backend.desktop = false
    await sim.sendSerial('not here')
    expect(backend.serialWrites).toEqual(['hello'])
  })

  it('turns a pin timing into LED brightness when solving', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('void setup(){}', false)

    const ledParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'r', kind: 'resistor', attrs: { value: '220' } },
      { id: 'led', kind: 'led' },
    ]
    const ledWires: EvalWire[] = [
      { a: { instanceId: 'uno', pin: '13' }, b: { instanceId: 'r', pin: '1' } },
      { a: { instanceId: 'r', pin: '2' }, b: { instanceId: 'led', pin: 'A' } },
      { a: { instanceId: 'led', pin: 'C' }, b: { instanceId: 'uno', pin: 'GND.2' } },
    ]
    sim.pinTimings.B5 = { duty: 0.25, highMicros: 500, periodMicros: 2000 }
    sim.solve(ledParts, ledWires)
    expect(sim.ledStates.led).toBe('lit')
    expect(sim.ledBrightness.led).toBeCloseTo(0.25)
  })

  it('remembers each digit of a multiplexed display', () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const segParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'seg', kind: 'seven-segment-4', attrs: { digits: '4' } },
    ]
    const wiresFor = (segment: string, common: string, pin: string): EvalWire[] => [
      { a: { instanceId: 'uno', pin }, b: { instanceId: 'seg', pin: segment } },
      { a: { instanceId: 'seg', pin: common }, b: { instanceId: 'uno', pin: 'GND.2' } },
    ]

    sim.avrPins = { D2: true }
    sim.solve(segParts, wiresFor('A', 'DIG1', '2'))
    expect(sim.outputs.seg.values[0]).toBe(1)

    // The next scan drives digit 2's B segment; digit 1 must not be erased.
    sim.avrPins = { D3: true }
    sim.solve(segParts, wiresFor('B', 'DIG2', '3'))
    expect(sim.outputs.seg.values[0]).toBe(1)
    expect(sim.outputs.seg.values[8 + 1]).toBe(1)
  })

  it('still solves the netlist in a plain browser, without sending anything', async () => {
    const backend = new FakeBackend()
    backend.desktop = false
    const sim = new Sim(backend)
    sim.pressedButtons.b1 = true
    sim.solve(parts, wires)
    expect(backend.inputs).toHaveLength(0)
    expect(sim.ledStates).toEqual({})
    expect(sim.warnings).toEqual({})
  })
})

describe('Sim.attach', () => {
  it('turns pin and serial events into state, and unsubscribes on cleanup', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()

    backend.pinCb?.(66, 5, true)
    backend.pinCb?.(68, 2, false)
    backend.timingCb?.(66, 5, 0.25, 500, 2000)
    backend.serialCb?.(72)
    backend.serialCb?.(105)

    expect(sim.pin13).toBe(true)
    expect(sim.avrPins).toEqual({ B5: true, D2: false })
    expect(sim.pinTimings).toEqual({ B5: { duty: 0.25, highMicros: 500, periodMicros: 2000 } })
    // Serial is batched for the UI, so it appears on the next flush.
    expect(sim.serial).toBe('')
    await new Promise((r) => setTimeout(r, 25))
    expect(sim.serial).toBe('Hi')

    stop()
    expect(backend.unsubscribed).toBe(3)
  })

  it('answers a trigger falling edge with an echo pulse, and not a rising one', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()

    const sonarParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'sonar', kind: 'hc-sr04', attrs: { distance: '100' } },
    ]
    const sonarWires: EvalWire[] = [
      { a: { instanceId: 'uno', pin: '5V' }, b: { instanceId: 'sonar', pin: 'VCC' } },
      { a: { instanceId: 'sonar', pin: 'GND' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'uno', pin: '7' }, b: { instanceId: 'sonar', pin: 'TRIG' } },
      { a: { instanceId: 'sonar', pin: 'ECHO' }, b: { instanceId: 'uno', pin: '8' } },
    ]
    sim.solve(sonarParts, sonarWires)

    // The sketch raises TRIG first; the sensor answers when it falls.
    backend.pinCb?.(68, 7, true)
    expect(backend.pulses).toEqual([])
    backend.pinCb?.(68, 7, false)
    expect(backend.pulses).toEqual([[66, 0, 600, 5800]]) // echo pin 8 = port B, bit 0

    stop()
  })

  it('clears a PWM timing when a pin event says it is digital again', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()

    sim.pinTimings.B5 = { duty: 0.25, highMicros: 500, periodMicros: 2000 }
    backend.pinCb?.(66, 5, false)

    expect(sim.pinTimings).toEqual({})
    expect(sim.avrPins.B5).toBe(false)
    stop()
  })

  it('does nothing in a plain browser', () => {
    const backend = new FakeBackend()
    backend.desktop = false
    const sim = new Sim(backend)
    const stop = sim.attach()
    expect(backend.pinCb).toBeNull()
    stop()
    expect(backend.unsubscribed).toBe(0)
  })
})
