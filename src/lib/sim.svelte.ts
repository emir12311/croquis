/**
 * The simulation session: run/stop, the chip's pins, serial output, and the
 * netlist solution that turns the circuit into LED states and button drives.
 *
 * The Tauri commands arrive through `SimBackend`, so the session's rules (a
 * fresh run clears the old levels, a rejected input is reported, serial is
 * trimmed without rebuilding the string per byte) can be tested without a
 * simulator.
 */
import {
  evaluateCircuit,
  type EvalPart,
  type EvalWire,
  type DhtSensor,
  type IrReceiver,
  type LcdWiring,
  type LedState,
  type OutputState,
  type RgbState,
  type StepperWiring,
  type SensorState,
} from './netlist'
import {
  emptyShiftState,
  stepShiftRegister,
  type ShiftRegisterState,
  type ShiftRegisterWiring,
} from './shift-register'

export interface SimBackend {
  isDesktop(): boolean
  compile(sketch: string): Promise<{ elf: string }>
  start(elf: string, bounce: boolean): Promise<void>
  stop(): Promise<void>
  setInput(port: number, pin: number, high: boolean): Promise<void>
  /** An input pulse timed in simulated microseconds — an ultrasonic echo. */
  pulseInput(port: number, pin: number, delayMicros: number, widthMicros: number): Promise<void>
  /** A timed edge train — a DHT22 response or an IR frame. */
  pulseScript(port: number, pin: number, delayMicros: number, durations: number[]): Promise<void>
  /** Hold a voltage on an ADC channel, in millivolts (0–5000). */
  setAnalog(channel: number, millivolts: number): Promise<void>
  /** Feed text into the sketch's Serial input. */
  writeSerial(text: string): Promise<void>
  /** Subscribe to pin changes; resolves to the unsubscribe function. */
  onPin(cb: (port: number, pin: number, high: boolean) => void): Promise<() => void>
  /** Subscribe to PWM timing; resolves to the unsubscribe function. */
  onTiming(
    cb: (port: number, pin: number, duty: number, highMicros: number, periodMicros: number) => void,
  ): Promise<() => void>
  /** Subscribe to serial bytes; resolves to the unsubscribe function. */
  onSerial(cb: (byte: number) => void): Promise<() => void>
}

/** The latest PWM timing for one pin. Duty is 0..1. */
export interface PinTiming {
  duty: number
  highMicros: number
  periodMicros: number
}

const SERIAL_LIMIT = 20_000

/** The two-coil sequence the bundled Stepper library writes. */
const STEPPER_PHASES = [
  [true, false, true, false],
  [false, true, true, false],
  [false, true, false, true],
  [true, false, false, true],
]

interface LcdState {
  chars: number[]
  address: number
  entryIncrement: boolean
  displayOn: boolean
  cursorOn: boolean
  blinkOn: boolean
  high: number | null
  rs: boolean
}

function emptyLcdState(): LcdState {
  return {
    chars: Array(32).fill(32),
    address: 0,
    entryIncrement: true,
    displayOn: true,
    cursorOn: false,
    blinkOn: false,
    high: null,
    rs: false,
  }
}

function lcdIndex(address: number): number | null {
  if (address <= 0x0f) return address
  if (address >= 0x40 && address <= 0x4f) return 16 + address - 0x40
  return null
}

function nextLcdAddress(address: number, increment: boolean): number {
  if (increment) {
    if (address >= 0x40) return address >= 0x4f ? 0x00 : address + 1
    return address >= 0x0f ? 0x40 : address + 1
  }
  if (address >= 0x40) return address <= 0x40 ? 0x0f : address - 1
  return address === 0 ? 0x4f : address - 1
}

function dhtDurations(humidity: number, temperature: number): number[] {
  const rh = Math.round(humidity * 10) & 0xffff
  let t = Math.round(Math.abs(temperature) * 10) & 0x7fff
  if (temperature < 0) t |= 0x8000
  const bytes = [rh >> 8, rh & 0xff, t >> 8, t & 0xff]
  bytes.push((bytes[0]! + bytes[1]! + bytes[2]! + bytes[3]!) & 0xff)
  // Stretched from the DHT22's 50/26/70 µs bit timings: the UI/train round trip
  // is not as tight as a real sensor, and the 0/1 gap stays unambiguous.
  const durations = [80, 80]
  for (const byte of bytes) {
    for (let bit = 7; bit >= 0; bit--) durations.push(80, (byte >> bit) & 1 ? 120 : 40)
  }
  durations.push(80)
  return durations
}

function irDurations(command: number, address = 0): number[] {
  const bytes = [address & 0xff, (~address) & 0xff, command & 0xff, (~command) & 0xff]
  const durations = [9000, 4500]
  for (const byte of bytes) {
    for (let bit = 0; bit < 8; bit++) {
      durations.push(560)
      durations.push((byte >> bit) & 1 ? 1690 : 560)
    }
  }
  durations.push(560)
  return durations
}

export class Sim {
  running = $state(false)
  serial = $state('')
  buildError = $state('')
  /** The Uno's built-in LED (pin 13) — a drawn state, not a source of truth. */
  pin13 = $state(false)
  /** Levels the chip last reported, keyed `"B5"`. */
  avrPins = $state<Record<string, boolean>>({})
  /** Latest PWM timing per pin, keyed `"B5"`. */
  pinTimings = $state<Record<string, PinTiming>>({})
  /** How hard each LED is driven. */
  ledStates = $state<Record<string, LedState>>({})
  /** How brightly each LED is drawn (0..1); PWM makes this continuous. */
  ledBrightness = $state<Record<string, number>>({})
  /** Per-channel brightness of each RGB LED, 0..1. */
  rgbLeds = $state<Record<string, RgbState>>({})
  /** Buzzer/relay/display/bar state by part id. */
  outputs = $state<Record<string, OutputState>>({})
  /** What is wrong with each part, for the canvas badge and the strip. */
  warnings = $state<Record<string, string>>({})
  /** Voltages the circuit puts on the chip's ADC inputs, in millivolts. */
  analogs = $state<{ channel: number; millivolts: number }[]>([])
  /** Sensor modules with an on/off output, by part id. */
  sensors = $state<Record<string, SensorState>>({})
  /** A button held on the canvas, by part id. */
  pressedButtons = $state<Record<string, boolean>>({})
  /** The key held on each keypad, by part id. */
  keypadKeys = $state<Record<string, { row: number; column: number }>>({})

  /** The last level sent per pin, so an unchanged drive is not re-sent every solve. */
  private lastDrives = new Map<string, boolean>()
  /** The last voltage sent per ADC channel, for the same reason. */
  private lastAnalogs = new Map<number, number>()
  /** Bytes waiting for the next UI flush; serial arrives one byte per event. */
  private serialBuffer = ''
  private serialFlush: ReturnType<typeof setTimeout> | null = null
  /** Per-digit segment memory for multiplexed displays, by part id. */
  private segmentMemory = new Map<string, number[]>()
  /** The last angle each servo was driven to, so a horn holds without a pulse. */
  private servoAngles = new Map<string, number>()
  /**
   * Board pins a sketch triggers, and where each trigger's echo comes back.
   * Keyed by the trigger pin (`B2`, the same key the pin events use).
   */
  private echoTriggers = new Map<
    string,
    { echoPort: number; echoPin: number; delayUs: number; widthUs: number }
  >()
  /** The 74HC595 shift registers' remembered bits, by part id. */
  private shiftRegisters: Record<string, ShiftRegisterState> = {}
  /** How each shift register's inputs are wired, rebuilt on every solve. */
  private shiftWiring: Record<string, ShiftRegisterWiring> = {}
  /** The stepper coils' board pins and step size, rebuilt on every solve. */
  private stepperWiring: Record<string, StepperWiring> = {}
  private stepperAngles = new Map<string, number>()
  private stepperPhases = new Map<string, number>()
  private lcdWiring: Record<string, LcdWiring> = {}
  private lcdStates = new Map<string, LcdState>()
  private dhtSensors: Record<string, DhtSensor> = {}
  private dhtBusy = new Map<string, number>()
  private irReceivers: Record<string, IrReceiver> = {}
  private backend: SimBackend

  constructor(backend: SimBackend) {
    this.backend = backend
  }

  /** Compile and run. Returns false when the build or the start failed. */
  async run(sketch: string, bounce: boolean): Promise<boolean> {
    this.buildError = ''
    this.clearSerialBuffer()
    this.serial = ''
    this.pin13 = false
    // A stopped chip drives nothing — clear the levels it last reported, or an
    // external LED would stay lit at whatever the sketch left it on.
    this.avrPins = {}
    this.pinTimings = {}
    this.ledBrightness = {}
    this.rgbLeds = {}
    this.outputs = {}
    this.segmentMemory.clear()
    this.shiftRegisters = {}
    this.stepperAngles.clear()
    this.stepperPhases.clear()
    this.lcdStates.clear()
    this.dhtBusy.clear()
    // A fresh run never inherits a held button.
    this.pressedButtons = {}
    this.keypadKeys = {}
    this.sensors = {}
    // A new simulator has no memory of the levels sent to the old one, so the
    // duplicate-suppression caches must not survive a restart either.
    this.lastDrives.clear()
    this.lastAnalogs.clear()
    try {
      const fw = await this.backend.compile(sketch)
      await this.backend.start(fw.elf, bounce)
      this.running = true
      return true
    } catch (err) {
      this.buildError = String(err)
      this.running = false
      return false
    }
  }

  async stop() {
    // Show bytes already received before tearing the run down.
    this.flushSerial()
    try {
      await this.backend.stop()
    } catch {
      /* ignore */
    }
    this.running = false
    this.pin13 = false
    this.avrPins = {}
    this.pinTimings = {}
    this.ledBrightness = {}
    this.rgbLeds = {}
    this.outputs = {}
    this.segmentMemory.clear()
    this.servoAngles.clear()
    this.shiftRegisters = {}
    this.stepperAngles.clear()
    this.stepperPhases.clear()
    this.lcdStates.clear()
    this.dhtBusy.clear()
    this.pressedButtons = {}
    this.keypadKeys = {}
    this.sensors = {}
  }

  /** Apply an edited sketch without leaving the run: recompile and restart. */
  async restart(sketch: string, bounce: boolean): Promise<boolean> {
    await this.stop()
    return this.run(sketch, bounce)
  }

  /** Forget everything a stopped circuit should not keep. */
  reset() {
    this.clearSerialBuffer()
    this.avrPins = {}
    this.pinTimings = {}
    this.ledBrightness = {}
    this.rgbLeds = {}
    this.outputs = {}
    this.segmentMemory.clear()
    this.servoAngles.clear()
    this.echoTriggers.clear()
    this.shiftRegisters = {}
    this.shiftWiring = {}
    this.stepperWiring = {}
    this.stepperAngles.clear()
    this.stepperPhases.clear()
    this.lcdWiring = {}
    this.lcdStates.clear()
    this.dhtSensors = {}
    this.dhtBusy.clear()
    this.pressedButtons = {}
    this.keypadKeys = {}
    this.sensors = {}
    this.analogs = []
    this.pin13 = false
    this.serial = ''
    this.buildError = ''
    this.lastDrives.clear()
    this.lastAnalogs.clear()
  }

  press(partId: string, down: boolean) {
    this.pressedButtons[partId] = down
  }

  private latchLcd(partId: string, wiring: LcdWiring) {
    const state = this.lcdStates.get(partId) ?? emptyLcdState()
    const rs = this.avrPins[wiring.pins.rs] === true
    const nibble =
      (this.avrPins[wiring.pins.d4] === true ? 1 : 0) |
      (this.avrPins[wiring.pins.d5] === true ? 2 : 0) |
      (this.avrPins[wiring.pins.d6] === true ? 4 : 0) |
      (this.avrPins[wiring.pins.d7] === true ? 8 : 0)
    if (state.high === null) {
      state.high = nibble
      state.rs = rs
      this.lcdStates.set(partId, state)
      return
    }
    const value = (state.high << 4) | nibble
    state.high = null
    if (state.rs) {
      const index = lcdIndex(state.address)
      if (index !== null) state.chars[index] = value
      state.address = nextLcdAddress(state.address, state.entryIncrement)
    } else if (value === 0x01) {
      state.chars = Array(32).fill(32)
      state.address = 0
    } else if (value === 0x02) {
      state.address = 0
    } else if ((value & 0xfc) === 0x04) {
      state.entryIncrement = (value & 0x02) !== 0
    } else if ((value & 0xf8) === 0x08) {
      state.displayOn = (value & 0x04) !== 0
      state.cursorOn = (value & 0x02) !== 0
      state.blinkOn = (value & 0x01) !== 0
    } else if ((value & 0x80) !== 0) {
      state.address = value & 0x7f
    }
    this.lcdStates.set(partId, state)
  }

  /** Send one NEC frame to every wired IR receiver. */
  sendIr(command: number) {
    if (!this.running) return
    const durations = irDurations(command)
    for (const receiver of Object.values(this.irReceivers)) {
      this.backend
        .pulseScript(receiver.pin.charCodeAt(0), Number(receiver.pin.slice(1)), 0, durations)
        .catch((err) => console.error('pulse_script failed', err))
    }
  }

  pressKey(partId: string, row: number, column: number) {
    this.keypadKeys[partId] = { row, column }
  }

  /** Release one key, or every key on that keypad when no key is named. */
  releaseKey(partId: string, row?: number, column?: number) {
    const key = this.keypadKeys[partId]
    if (!key) return
    if (row !== undefined && (key.row !== row || key.column !== column)) return
    delete this.keypadKeys[partId]
  }

  /** Send text to the sketch's Serial input, if a desktop simulation is running. */
  async sendSerial(text: string) {
    if (!text || !this.running || !this.backend.isDesktop()) return
    try {
      await this.backend.writeSerial(text)
    } catch (err) {
      console.error('serial_input failed', err)
    }
  }

  /**
   * Re-solve the circuit and push any button-driven pins back into the simulator.
   * `pressedButtons` and `avrPins` are handed over by reference, not spread: reading
   * `pressedButtons[id]` is tracked even for an id that is not in the object yet,
   * which is what carries a press from the canvas into the simulator.
   */
  solve(parts: EvalPart[], wires: EvalWire[]) {
    // A PWM pin is still a source for connectivity; its duty says how hard. Pass
    // both views so the netlist can keep the LED on and scale its brightness,
    // instead of seeing whichever edge happened to arrive last.
    const pins = { ...this.avrPins }
    const duty: Record<string, number> = {}
    const frequency: Record<string, number> = {}
    const pulse: Record<string, number> = {}
    for (const [key, timing] of Object.entries(this.pinTimings)) {
      pins[key] = timing.duty > 0
      duty[key] = timing.duty
      if (timing.periodMicros > 0) frequency[key] = 1_000_000 / timing.periodMicros
      // A servo's horn angle comes from the width of the last HIGH stretch.
      if (timing.highMicros > 0) pulse[key] = timing.highMicros
    }
    const result = evaluateCircuit(
      parts,
      wires,
      this.pressedButtons,
      pins,
      duty,
      frequency,
      pulse,
      this.shiftRegisters,
      this.keypadKeys,
    )
    const outputs = { ...result.outputs }
    // A multiplexed display is sampled one digit at a time. Keep the last lit
    // segments for every digit so the next scan does not erase the others.
    for (const part of parts) {
      if (part.kind !== 'seven-segment-4') continue
      const digits = 4
      const state = outputs[part.id]
      if (!state) continue
      const memory = this.segmentMemory.get(part.id) ?? Array(digits * 8).fill(0)
      if (state.active && typeof state.digit === 'number') {
        memory.splice(state.digit * 8, 8, ...state.values.slice(0, 8))
      }
      this.segmentMemory.set(part.id, memory)
      outputs[part.id] = { ...state, values: [...memory] }
    }
    // A servo without signal holds where it was, like the real thing: the
    // netlist leaves `angle` absent then, and the remembered value fills in.
    for (const part of parts) {
      if (part.kind !== 'servo') continue
      const state = outputs[part.id]
      if (!state) continue
      if (typeof state.angle === 'number') {
        this.servoAngles.set(part.id, state.angle)
      } else if (this.servoAngles.has(part.id)) {
        outputs[part.id] = { ...state, angle: this.servoAngles.get(part.id) }
      }
    }
    for (const part of parts) {
      if (part.kind !== 'stepper-motor') continue
      const wiring = result.stepperWiring[part.id]
      const angle = this.stepperAngles.get(part.id) ?? 0
      outputs[part.id] = {
        active: !!wiring,
        values: [],
        angle: Math.round(angle * 10) / 10,
      }
    }
    for (const part of parts) {
      if (part.kind !== 'lcd1602') continue
      const state = this.lcdStates.get(part.id) ?? emptyLcdState()
      const row = state.address >= 0x40 && state.address <= 0x4f ? 1 : 0
      const column = row ? state.address - 0x40 : state.address
      outputs[part.id] = {
        active: !!result.lcdWiring[part.id] && state.displayOn,
        values: [...state.chars],
        cursorX: Math.min(15, Math.max(0, column)),
        cursorY: row,
        cursor: state.cursorOn,
        blink: state.blinkOn,
      }
    }
    this.ledStates = result.leds
    this.ledBrightness = result.ledBrightness
    this.rgbLeds = result.rgbLeds
    this.outputs = outputs
    this.warnings = result.warnings
    this.analogs = result.analogs
    this.sensors = result.sensors
    this.shiftRegisters = result.shiftRegisters
    this.shiftWiring = result.shiftWiring
    this.stepperWiring = result.stepperWiring
    this.lcdWiring = result.lcdWiring
    this.dhtSensors = result.dhtSensors
    this.irReceivers = result.irReceivers
    // Triggers the sketch will pull: remember where each echo should come back.
    // The map is rebuilt every solve, so re-wiring takes effect immediately.
    this.echoTriggers.clear()
    for (const echo of result.echoes) {
      this.echoTriggers.set(`${echo.trigger.port}${echo.trigger.pin}`, {
        echoPort: echo.echo.port.charCodeAt(0),
        echoPin: echo.echo.pin,
        delayUs: echo.delayUs,
        widthUs: echo.widthUs,
      })
    }
    if (!this.backend.isDesktop() || !this.running) {
      this.lastDrives.clear()
      this.lastAnalogs.clear()
      return
    }
    for (const drive of result.drives) {
      const key = `${drive.port}${drive.pin}`
      if (this.lastDrives.get(key) === drive.high) continue
      this.lastDrives.set(key, drive.high)
      // Not fire-and-forget: a rejected command used to vanish silently, which is
      // exactly how "the button does nothing" hid for this long.
      this.backend
        .setInput(drive.port.charCodeAt(0), drive.pin, drive.high)
        .catch((err) => console.error('set_input failed', err))
    }
    for (const analog of result.analogs) {
      if (this.lastAnalogs.get(analog.channel) === analog.millivolts) continue
      this.lastAnalogs.set(analog.channel, analog.millivolts)
      this.backend
        .setAnalog(analog.channel, analog.millivolts)
        .catch((err) => console.error('set_analog failed', err))
    }
  }

  /** Subscribe to the engine's events. Returns the unsubscribe function. */
  attach(): () => void {
    if (!this.backend.isDesktop()) return () => {}
    let alive = true
    const unlisten: (() => void)[] = []
    this.backend
      .onPin((port, pin, high) => {
        const key = `${String.fromCharCode(port)}${pin}`
        // A Pin event means this is no longer PWM; clear the timing so it does
        // not keep overriding the level in solve().
        if (this.pinTimings[key]) {
          const next = { ...this.pinTimings }
          delete next[key]
          this.pinTimings = next
        }
        this.avrPins[key] = high
        // A shiftOut burst is several clock edges in a few microseconds, so the
        // register is stepped per edge here rather than once per rendered frame.
        for (const [partId, wiring] of Object.entries(this.shiftWiring)) {
          if (!Object.values(wiring.pins).includes(key)) continue
          this.shiftRegisters[partId] = stepShiftRegister(
            this.shiftRegisters[partId] ?? emptyShiftState(),
            wiring,
            this.avrPins,
          )
        }
        for (const [partId, wiring] of Object.entries(this.stepperWiring)) {
          if (!wiring.pins.includes(key)) continue
          const levels = wiring.pins.map((coil) => this.avrPins[coil] === true)
          const phase = STEPPER_PHASES.findIndex((pattern) =>
            pattern.every((value, i) => value === levels[i]),
          )
          if (phase < 0) continue
          const previous = this.stepperPhases.get(partId)
          if (previous === undefined) {
            this.stepperPhases.set(partId, phase)
            continue
          }
          if (phase === previous) continue
          const delta = (phase - previous + 4) % 4
          const direction = delta === 1 ? 1 : delta === 3 ? -1 : 0
          if (!direction) continue
          this.stepperAngles.set(
            partId,
            (this.stepperAngles.get(partId) ?? 0) + direction * wiring.stepAngle,
          )
          this.stepperPhases.set(partId, phase)
        }
        for (const [partId, wiring] of Object.entries(this.lcdWiring)) {
          if (key !== wiring.pins.e || high) continue
          this.latchLcd(partId, wiring)
        }
        if (high) {
          for (const [partId, sensor] of Object.entries(this.dhtSensors)) {
            if (sensor.pin !== key) continue
            const now = Date.now()
            if ((this.dhtBusy.get(partId) ?? 0) > now) continue
            this.dhtBusy.set(partId, now + 1000)
            this.backend
              .pulseScript(
                Number(sensor.pin.charCodeAt(0)),
                Number(sensor.pin.slice(1)),
                30,
                dhtDurations(sensor.humidity, sensor.temperature),
              )
              .catch((err) => console.error('pulse_script failed', err))
          }
        }
        if (port === 66 && pin === 5) this.pin13 = high
        // A trigger pulse ends on the falling edge: that is the moment the
        // sensor answers, with an echo timed on the sketch's own clock.
        if (!high) {
          const trigger = this.echoTriggers.get(key)
          if (trigger) {
            this.backend
              .pulseInput(trigger.echoPort, trigger.echoPin, trigger.delayUs, trigger.widthUs)
              .catch((err) => console.error('pulse_input failed', err))
          }
        }
      })
      .then((u) => alive && unlisten.push(u))
    this.backend
      .onTiming((port, pin, duty, highMicros, periodMicros) => {
        this.pinTimings[`${String.fromCharCode(port)}${pin}`] = {
          duty,
          highMicros,
          periodMicros,
        }
      })
      .then((u) => alive && unlisten.push(u))
    this.backend
      .onSerial((byte) => {
        // Serial arrives one byte per event, but the UI does not need a reactive
        // update per byte: a 20 Hz print loop is ~1,400 renders a second. Collect
        // bytes and flush once per frame; the monitor and auto-scroll follow that.
        this.serialBuffer += String.fromCharCode(byte)
        if (this.serialFlush === null) {
          this.serialFlush = setTimeout(() => {
            this.serialFlush = null
            this.flushSerial()
          }, 16)
        }
      })
      .then((u) => alive && unlisten.push(u))
    return () => {
      alive = false
      unlisten.forEach((u) => u())
      this.clearSerialBuffer()
    }
  }

  /** Move buffered serial bytes into the reactive string, at most once a frame. */
  private flushSerial() {
    if (this.serialFlush !== null) {
      clearTimeout(this.serialFlush)
      this.serialFlush = null
    }
    if (!this.serialBuffer) return
    const next = this.serial + this.serialBuffer
    this.serialBuffer = ''
    this.serial = next.length > SERIAL_LIMIT ? next.slice(-SERIAL_LIMIT) : next
  }

  /** Drop bytes from a run that is being replaced, and cancel the pending flush. */
  private clearSerialBuffer() {
    if (this.serialFlush !== null) {
      clearTimeout(this.serialFlush)
      this.serialFlush = null
    }
    this.serialBuffer = ''
  }
}
