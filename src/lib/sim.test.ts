import { describe, expect, it, vi } from 'vitest'
import { Sim, type SimBackend } from './sim.svelte'
import { unoPin, type EvalPart, type EvalWire } from './netlist'

class FakeBackend implements SimBackend {
  desktop = true
  compiled: string[] = []
  started: { elf: string; bounce: boolean }[] = []
  stopped = 0
  inputs: [number, number, boolean][] = []
  pulses: [number, number, number, number][] = []
  scripts: [number, number, number, number[]][] = []
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
  async pulseScript(port: number, pin: number, delayMicros: number, durations: number[]) {
    this.scripts.push([port, pin, delayMicros, durations])
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

  it('answers a DHT start with a timed pulse script, once', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()
    await sim.run('void setup(){}', false)

    const dhtParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'dht', kind: 'dht22', attrs: { temperature: '25', humidity: '50' } },
    ]
    const dhtWires: EvalWire[] = [
      { a: { instanceId: 'uno', pin: '5V' }, b: { instanceId: 'dht', pin: 'VCC' } },
      { a: { instanceId: 'dht', pin: 'GND' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'uno', pin: '2' }, b: { instanceId: 'dht', pin: 'SDA' } },
    ]
    sim.solve(dhtParts, dhtWires)
    expect(backend.scripts).toHaveLength(0)

    backend.pinCb?.(68, 2, false)
    backend.pinCb?.(68, 2, true)
    expect(backend.scripts).toHaveLength(1)
    const [port, pin, delay, durations] = backend.scripts[0]!
    expect([port, pin, delay]).toEqual([68, 2, 30])
    expect(durations).toHaveLength(83)

    // The response's own rising edges must not start another read.
    backend.pinCb?.(68, 2, false)
    backend.pinCb?.(68, 2, true)
    expect(backend.scripts).toHaveLength(1)
    stop()
  })

  it('answers a DHT read with the humidity it was just given', async () => {
    // Changing only the humidity slider must reach the wire: the sensor is
    // re-solved with both values, and the next transaction encodes the new one.
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()
    await sim.run('void setup(){}', false)

    const dhtParts = (humidity: string): EvalPart[] => [
      { id: 'uno', kind: 'uno' },
      { id: 'dht', kind: 'dht22', attrs: { temperature: '25', humidity } },
    ]
    const dhtWires: EvalWire[] = [
      { a: { instanceId: 'uno', pin: '5V' }, b: { instanceId: 'dht', pin: 'VCC' } },
      { a: { instanceId: 'dht', pin: 'GND' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'uno', pin: '2' }, b: { instanceId: 'dht', pin: 'SDA' } },
    ]
    const startRead = () => {
      backend.pinCb?.(68, 2, false)
      backend.pinCb?.(68, 2, true)
    }

    sim.solve(dhtParts('50'), dhtWires)
    startRead()
    expect(backend.scripts).toHaveLength(1)

    // Step past the one-second guard that stops the response's own edges from
    // starting another read, without making the test wait for it.
    const realNow = Date.now
    Date.now = () => realNow() + 2000
    try {
      sim.solve(dhtParts('80'), dhtWires)
      startRead()
    } finally {
      Date.now = realNow
    }

    expect(backend.scripts).toHaveLength(2)
    const humid50 = backend.scripts[0]![3]
    const humid80 = backend.scripts[1]![3]
    expect(humid80).not.toEqual(humid50)
    stop()
  })

  it('sends a NEC frame to every wired IR receiver', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('void setup(){}', false)

    const irParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'ir', kind: 'ir-receiver' },
    ]
    const irWires: EvalWire[] = [
      { a: { instanceId: 'ir', pin: 'VCC' }, b: { instanceId: 'uno', pin: '5V' } },
      { a: { instanceId: 'ir', pin: 'GND' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'uno', pin: '2' }, b: { instanceId: 'ir', pin: 'DAT' } },
    ]
    sim.solve(irParts, irWires)
    sim.sendIr(0xa2)

    expect(backend.scripts).toHaveLength(1)
    const [port, pin, delay, durations] = backend.scripts[0]!
    expect([port, pin, delay]).toEqual([68, 2, 0])
    expect(durations.slice(0, 2)).toEqual([9000, 4500])
    expect(durations).toHaveLength(67)
    expect(durations.at(-1)).toBe(560)
  })

  it('decodes LCD commands and writes text', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()
    await sim.run('void setup(){}', false)

    const lcdParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'lcd', kind: 'lcd1602' },
    ]
    const lcdWires: EvalWire[] = [
      { a: { instanceId: 'lcd', pin: 'VSS' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'lcd', pin: 'VDD' }, b: { instanceId: 'uno', pin: '5V' } },
      { a: { instanceId: 'lcd', pin: 'RS' }, b: { instanceId: 'uno', pin: '12' } },
      { a: { instanceId: 'lcd', pin: 'E' }, b: { instanceId: 'uno', pin: '11' } },
      { a: { instanceId: 'lcd', pin: 'D4' }, b: { instanceId: 'uno', pin: '5' } },
      { a: { instanceId: 'lcd', pin: 'D5' }, b: { instanceId: 'uno', pin: '4' } },
      { a: { instanceId: 'lcd', pin: 'D6' }, b: { instanceId: 'uno', pin: '3' } },
      { a: { instanceId: 'lcd', pin: 'D7' }, b: { instanceId: 'uno', pin: '2' } },
    ]
    sim.solve(lcdParts, lcdWires)

    const data = [
      ['D5', 68, 5],
      ['D4', 68, 4],
      ['D3', 68, 3],
      ['D2', 68, 2],
    ] as const
    const setNibble = (nibble: number) => {
      for (let i = 0; i < 4; i++) backend.pinCb?.(data[i]![1], data[i]![2], ((nibble >> i) & 1) === 1)
    }
    const send = (value: number, rs: boolean) => {
      backend.pinCb?.(66, 4, rs)
      setNibble(value >> 4)
      backend.pinCb?.(66, 3, true)
      backend.pinCb?.(66, 3, false)
      setNibble(value & 0x0f)
      backend.pinCb?.(66, 3, true)
      backend.pinCb?.(66, 3, false)
    }

    send(0x01, false) // clear
    send(0x80, false) // cursor to 0,0
    send('H'.charCodeAt(0), true)
    send('i'.charCodeAt(0), true)
    sim.solve(lcdParts, lcdWires)

    expect(sim.outputs.lcd.values.slice(0, 2)).toEqual([72, 105])
    expect(sim.outputs.lcd.cursorX).toBe(2)
    expect(sim.outputs.lcd.cursorY).toBe(0)
    stop()
  })

  it('turns a stepper motor from the coil sequence', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()
    await sim.run('void setup(){}', false)

    const motorParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'motor', kind: 'stepper-motor', attrs: { steps: '200' } },
    ]
    const motorWires: EvalWire[] = [
      { a: { instanceId: 'uno', pin: '8' }, b: { instanceId: 'motor', pin: 'A-' } },
      { a: { instanceId: 'uno', pin: '9' }, b: { instanceId: 'motor', pin: 'A+' } },
      { a: { instanceId: 'uno', pin: '10' }, b: { instanceId: 'motor', pin: 'B+' } },
      { a: { instanceId: 'uno', pin: '11' }, b: { instanceId: 'motor', pin: 'B-' } },
    ]
    sim.solve(motorParts, motorWires)
    expect(sim.outputs.motor.angle).toBe(0)

    // Phase 0 = A-/B+; then phase 1 = A+/B+; then phase 2 = A+/B-.
    for (const levels of [
      [true, false, true, false],
      [false, true, true, false],
      [false, true, false, true],
    ]) {
      for (let pin = 0; pin < 4; pin++) backend.pinCb?.(66, pin, levels[pin]!)
    }
    sim.solve(motorParts, motorWires)
    expect(sim.outputs.motor.angle).toBeCloseTo(3.6)
    stop()
  })

  it('does not latch a phantom nibble from the startup pin levels', async () => {
    // The simulator announces each pin's level when it starts. That lone `low` on
    // the enable line is not a clock pulse: latching it shifts every later byte by
    // half a byte, which showed up as a couple of stray glyphs in wrong cells.
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()
    await sim.run('void setup(){}', false)

    const parts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'lcd', kind: 'lcd1602' },
    ]
    const w = (pin: string, lcdPin: string): EvalWire => ({
      a: { instanceId: 'lcd', pin: lcdPin },
      b: { instanceId: 'uno', pin },
    })
    const wires: EvalWire[] = [
      w('GND.1', 'VSS'),
      w('5V', 'VDD'),
      w('GND.1', 'V0'),
      w('12', 'RS'),
      w('GND.1', 'RW'),
      w('11', 'E'),
      w('5', 'D4'),
      w('4', 'D5'),
      w('3', 'D6'),
      w('2', 'D7'),
      w('5V', 'A'),
      w('GND.1', 'K'),
    ]
    sim.solve(parts, wires)

    const levels = new Map<string, boolean>()
    const drive = (pin: string, high: boolean) => {
      if ((levels.get(pin) ?? false) === high) return
      levels.set(pin, high)
      const info = unoPin(pin)
      if (!info || info.kind !== 'mcu') throw new Error(`${pin} is not a chip pin`)
      backend.pinCb?.(info.port.charCodeAt(0), info.pin, high)
    }
    const dataPins = ['5', '4', '3', '2']
    const nibble = (value: number) => {
      for (let i = 0; i < 4; i++) drive(dataPins[i]!, ((value >> i) & 1) === 1)
      drive('11', true)
      drive('11', false)
    }
    const send = (value: number, data: boolean) => {
      drive('12', data)
      nibble(value >> 4)
      nibble(value & 0x0f)
    }

    // The startup announcement, exactly as simavr sends it: every pin low. It has
    // to bypass `drive`, whose job is to drop unchanged levels.
    for (const pin of ['12', '11', '5', '4', '3', '2']) {
      levels.set(pin, false)
      const info = unoPin(pin)
      if (!info || info.kind !== 'mcu') throw new Error(`${pin} is not a chip pin`)
      backend.pinCb?.(info.port.charCodeAt(0), info.pin, false)
    }

    for (const cmd of [0x33, 0x32, 0x28, 0x0c, 0x06, 0x01]) send(cmd, false)
    for (const ch of 'Hi') send(ch.charCodeAt(0), true)

    sim.solve(parts, wires)
    const values = sim.outputs.lcd!.values
    expect(String.fromCharCode(...values.slice(0, 2))).toBe('Hi')
    stop()
  })

  it('keeps a tapped key closed long enough for one scan', () => {
    // A click arrives as press and release in the same task, so a solve that runs
    // afterwards would never see the key. The release is held back briefly.
    vi.useFakeTimers()
    try {
      const sim = new Sim(new FakeBackend())
      sim.pressKey('pad', 1, 2)
      sim.releaseKey('pad', 1, 2)
      expect(sim.keypadKeys.pad).toEqual({ row: 1, column: 2 })
      vi.advanceTimersByTime(600)
      expect(sim.keypadKeys.pad).toBeUndefined()
    } finally {
      vi.useRealTimers()
    }
  })

  it('drives a keypad column through the matrix while a key is held', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    await sim.run('void setup(){}', false)

    const padParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'pad', kind: 'membrane-keypad' },
    ]
    const padWires: EvalWire[] = [
      { a: { instanceId: 'uno', pin: '9' }, b: { instanceId: 'pad', pin: 'R1' } },
      { a: { instanceId: 'uno', pin: '8' }, b: { instanceId: 'pad', pin: 'R2' } },
      { a: { instanceId: 'uno', pin: '7' }, b: { instanceId: 'pad', pin: 'R3' } },
      { a: { instanceId: 'uno', pin: '6' }, b: { instanceId: 'pad', pin: 'R4' } },
      { a: { instanceId: 'uno', pin: '5' }, b: { instanceId: 'pad', pin: 'C1' } },
      { a: { instanceId: 'uno', pin: '4' }, b: { instanceId: 'pad', pin: 'C2' } },
      { a: { instanceId: 'uno', pin: '3' }, b: { instanceId: 'pad', pin: 'C3' } },
      { a: { instanceId: 'uno', pin: '2' }, b: { instanceId: 'pad', pin: 'C4' } },
    ]

    sim.avrPins = { B1: false }
    sim.pressKey('pad', 0, 0)
    sim.solve(padParts, padWires)
    expect(backend.inputs).toContainEqual([68, 5, false])

    // Pressing and releasing in the same task is a tap, and a tap's release is
    // held back so a scan of the matrix can still find the key — otherwise a
    // click is over before the sketch looks. The column therefore stays low
    // until the hold expires.
    vi.useFakeTimers()
    try {
      sim.releaseKey('pad')
      sim.solve(padParts, padWires)
      expect(backend.inputs).toContainEqual([68, 5, false])
      vi.advanceTimersByTime(600)
    } finally {
      vi.useRealTimers()
    }
    sim.solve(padParts, padWires)
    expect(backend.inputs).toContainEqual([68, 5, true])
  })

  it('shows the text a LiquidCrystal sketch prints', async () => {
    // Replays the exact byte stream `LiquidCrystal` produces for the shipped LCD
    // preset: begin(), two prints, and a setCursor between them.
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()
    await sim.run('void setup(){}', false)

    const parts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'lcd', kind: 'lcd1602' },
    ]
    const w = (pin: string, lcdPin: string): EvalWire => ({
      a: { instanceId: 'lcd', pin: lcdPin },
      b: { instanceId: 'uno', pin },
    })
    const wires: EvalWire[] = [
      w('GND.1', 'VSS'),
      w('5V', 'VDD'),
      w('GND.1', 'V0'),
      w('12', 'RS'),
      w('GND.1', 'RW'),
      w('11', 'E'),
      w('5', 'D4'),
      w('4', 'D5'),
      w('3', 'D6'),
      w('2', 'D7'),
      w('5V', 'A'),
      w('GND.1', 'K'),
    ]
    sim.solve(parts, wires)

    // simavr reports transitions only, so the replay drives levels, not events.
    const levels = new Map<string, boolean>()
    const drive = (pin: string, high: boolean) => {
      if ((levels.get(pin) ?? false) === high) return
      levels.set(pin, high)
      const info = unoPin(pin)
      if (!info || info.kind !== 'mcu') throw new Error(`${pin} is not a chip pin`)
      backend.pinCb?.(info.port.charCodeAt(0), info.pin, high)
    }
    const dataPins = ['5', '4', '3', '2'] // D4..D7
    const nibble = (value: number) => {
      for (let i = 0; i < 4; i++) drive(dataPins[i]!, ((value >> i) & 1) === 1)
      drive('11', true)
      drive('11', false) // the falling edge latches the nibble
    }
    const send = (value: number, data: boolean) => {
      drive('12', data)
      nibble(value >> 4)
      nibble(value & 0x0f)
    }

    for (const cmd of [0x33, 0x32, 0x28, 0x0c, 0x06, 0x01]) send(cmd, false)
    for (const ch of 'Hello, Croquis!') send(ch.charCodeAt(0), true)
    send(0x80 | 0x40, false)
    for (const ch of 'LCD 1602 ready') send(ch.charCodeAt(0), true)

    sim.solve(parts, wires)
    const values = sim.outputs.lcd!.values
    expect(String.fromCharCode(...values.slice(0, 16))).toBe('Hello, Croquis! ')
    expect(String.fromCharCode(...values.slice(16, 32))).toBe('LCD 1602 ready  ')
    stop()
  })

  it('steps a 74HC595 on every clock edge, not once per solve', async () => {
    const backend = new FakeBackend()
    const sim = new Sim(backend)
    const stop = sim.attach()
    await Promise.resolve()

    const shiftParts: EvalPart[] = [
      { id: 'uno', kind: 'uno' },
      { id: 'hc', kind: '74hc595' },
    ]
    const shiftWires: EvalWire[] = [
      { a: { instanceId: 'uno', pin: '5V' }, b: { instanceId: 'hc', pin: 'VCC' } },
      { a: { instanceId: 'hc', pin: 'GND' }, b: { instanceId: 'uno', pin: 'GND.2' } },
      { a: { instanceId: 'uno', pin: '2' }, b: { instanceId: 'hc', pin: 'DS' } },
      { a: { instanceId: 'uno', pin: '3' }, b: { instanceId: 'hc', pin: 'SHCP' } },
      { a: { instanceId: 'uno', pin: '4' }, b: { instanceId: 'hc', pin: 'STCP' } },
      { a: { instanceId: 'uno', pin: '5' }, b: { instanceId: 'hc', pin: 'MR' } },
      { a: { instanceId: 'uno', pin: '6' }, b: { instanceId: 'hc', pin: 'OE' } },
      { a: { instanceId: 'hc', pin: 'Q0' }, b: { instanceId: 'uno', pin: '7' } },
    ]
    await sim.run('void setup(){}', false)
    sim.avrPins = { D5: true, D6: false }
    sim.solve(shiftParts, shiftWires)
    expect(backend.inputs.at(-1)).toEqual([68, 7, false])

    // 0x01 MSBFIRST: seven zeros, then a one in Q0. Each edge is its own
    // callback; a single solve at the end must still see all eight.
    for (let bit = 7; bit >= 0; bit--) {
      backend.pinCb?.(68, 2, ((0x01 >> bit) & 1) === 1)
      backend.pinCb?.(68, 3, true)
      backend.pinCb?.(68, 3, false)
    }
    backend.pinCb?.(68, 4, true)
    sim.solve(shiftParts, shiftWires)
    expect(backend.inputs.at(-1)).toEqual([68, 7, true])

    sim.solve(shiftParts, shiftWires)
    expect(backend.inputs.at(-1)).toEqual([68, 7, true])
    stop()
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
