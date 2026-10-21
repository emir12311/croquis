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
  type LedState,
  type OutputState,
  type RgbState,
  type SensorState,
} from './netlist'

export interface SimBackend {
  isDesktop(): boolean
  compile(sketch: string): Promise<{ elf: string }>
  start(elf: string, bounce: boolean): Promise<void>
  stop(): Promise<void>
  setInput(port: number, pin: number, high: boolean): Promise<void>
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

  /** The last level sent per pin, so an unchanged drive is not re-sent every solve. */
  private lastDrives = new Map<string, boolean>()
  /** The last voltage sent per ADC channel, for the same reason. */
  private lastAnalogs = new Map<number, number>()
  /** Bytes waiting for the next UI flush; serial arrives one byte per event. */
  private serialBuffer = ''
  private serialFlush: ReturnType<typeof setTimeout> | null = null
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
    // A fresh run never inherits a held button.
    this.pressedButtons = {}
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
    this.pressedButtons = {}
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
    this.pressedButtons = {}
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
    for (const [key, timing] of Object.entries(this.pinTimings)) {
      pins[key] = timing.duty > 0
      duty[key] = timing.duty
    }
    const result = evaluateCircuit(parts, wires, this.pressedButtons, pins, duty)
    this.ledStates = result.leds
    this.ledBrightness = result.ledBrightness
    this.rgbLeds = result.rgbLeds
    this.outputs = result.outputs
    this.warnings = result.warnings
    this.analogs = result.analogs
    this.sensors = result.sensors
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
        if (port === 66 && pin === 5) this.pin13 = high
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
