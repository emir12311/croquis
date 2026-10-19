/**
 * The simulation session: run/stop, the chip's pins, serial output, and the
 * netlist solution that turns the circuit into LED states and button drives.
 *
 * The Tauri commands arrive through `SimBackend`, so the session's rules (a
 * fresh run clears the old levels, a rejected input is reported, serial is
 * trimmed without rebuilding the string per byte) can be tested without a
 * simulator.
 */
import { evaluateCircuit, type EvalPart, type EvalWire, type LedState } from './netlist'

export interface SimBackend {
  isDesktop(): boolean
  compile(sketch: string): Promise<{ elf: string }>
  start(elf: string, bounce: boolean): Promise<void>
  stop(): Promise<void>
  setInput(port: number, pin: number, high: boolean): Promise<void>
  /** Hold a voltage on an ADC channel, in millivolts (0–5000). */
  setAnalog(channel: number, millivolts: number): Promise<void>
  /** Subscribe to pin changes; resolves to the unsubscribe function. */
  onPin(cb: (port: number, pin: number, high: boolean) => void): Promise<() => void>
  /** Subscribe to serial bytes; resolves to the unsubscribe function. */
  onSerial(cb: (byte: number) => void): Promise<() => void>
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
  /** How hard each LED is driven. */
  ledStates = $state<Record<string, LedState>>({})
  /** What is wrong with each part, for the canvas badge and the strip. */
  warnings = $state<Record<string, string>>({})
  /** Voltages the circuit puts on the chip's ADC inputs, in millivolts. */
  analogs = $state<{ channel: number; millivolts: number }[]>([])
  /** A button held on the canvas, by part id. */
  pressedButtons = $state<Record<string, boolean>>({})

  /** The last level sent per pin, so an unchanged drive is not re-sent every solve. */
  private lastDrives = new Map<string, boolean>()
  /** The last voltage sent per ADC channel, for the same reason. */
  private lastAnalogs = new Map<number, number>()
  private backend: SimBackend

  constructor(backend: SimBackend) {
    this.backend = backend
  }

  /** Compile and run. Returns false when the build or the start failed. */
  async run(sketch: string, bounce: boolean): Promise<boolean> {
    this.buildError = ''
    this.serial = ''
    this.pin13 = false
    // A stopped chip drives nothing — clear the levels it last reported, or an
    // external LED would stay lit at whatever the sketch left it on.
    this.avrPins = {}
    // A fresh run never inherits a held button.
    this.pressedButtons = {}
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
    try {
      await this.backend.stop()
    } catch {
      /* ignore */
    }
    this.running = false
    this.pin13 = false
    this.avrPins = {}
    this.pressedButtons = {}
  }

  /** Apply an edited sketch without leaving the run: recompile and restart. */
  async restart(sketch: string, bounce: boolean): Promise<boolean> {
    await this.stop()
    return this.run(sketch, bounce)
  }

  /** Forget everything a stopped circuit should not keep. */
  reset() {
    this.avrPins = {}
    this.pressedButtons = {}
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

  /**
   * Re-solve the circuit and push any button-driven pins back into the simulator.
   * `pressedButtons` and `avrPins` are handed over by reference, not spread: reading
   * `pressedButtons[id]` is tracked even for an id that is not in the object yet,
   * which is what carries a press from the canvas into the simulator.
   */
  solve(parts: EvalPart[], wires: EvalWire[]) {
    const result = evaluateCircuit(parts, wires, this.pressedButtons, this.avrPins)
    this.ledStates = result.leds
    this.warnings = result.warnings
    this.analogs = result.analogs
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
        this.avrPins[`${String.fromCharCode(port)}${pin}`] = high
        if (port === 66 && pin === 5) this.pin13 = high
      })
      .then((u) => alive && unlisten.push(u))
    this.backend
      .onSerial((byte) => {
        // Append, then trim only when over the cap. Slicing on every byte is O(n²) —
        // at 115200 baud that rebuilds a 20 000-character string a thousand times a
        // second, which is enough to stutter the UI.
        this.serial += String.fromCharCode(byte)
        if (this.serial.length > SERIAL_LIMIT) this.serial = this.serial.slice(-SERIAL_LIMIT)
      })
      .then((u) => alive && unlisten.push(u))
    return () => {
      alive = false
      unlisten.forEach((u) => u())
    }
  }
}
