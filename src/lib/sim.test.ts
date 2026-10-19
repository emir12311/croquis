import { describe, expect, it } from 'vitest'
import { Sim, type SimBackend } from './sim.svelte'
import type { EvalPart, EvalWire } from './netlist'

class FakeBackend implements SimBackend {
  desktop = true
  compiled: string[] = []
  started: { elf: string; bounce: boolean }[] = []
  stopped = 0
  inputs: [number, number, boolean][] = []
  analogs: [number, number][] = []
  failCompile = ''
  unsubscribed = 0
  pinCb: ((port: number, pin: number, high: boolean) => void) | null = null
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
  async setAnalog(channel: number, millivolts: number) {
    this.analogs.push([channel, millivolts])
  }
  async onPin(cb: (port: number, pin: number, high: boolean) => void) {
    this.pinCb = cb
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
    backend.serialCb?.(72)
    backend.serialCb?.(105)

    expect(sim.pin13).toBe(true)
    expect(sim.avrPins).toEqual({ B5: true, D2: false })
    expect(sim.serial).toBe('Hi')

    stop()
    expect(backend.unsubscribed).toBe(2)
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
