/**
 * Completions for the Arduino API.
 *
 * The editor had the machinery but no source, so it offered nothing — which is
 * the "no autocomplete" complaint. This is the one place a beginner looks for a
 * function and how to call it, so each entry carries its signature *and* a line
 * saying what it does, and the common calls insert a whole statement (including
 * the `;`) with the cursor in the first argument.
 *
 * Only what sketches can actually use here: no `#include` suggestions, because
 * we don't bundle libraries yet, and offering them would be a lie.
 */
import { snippetCompletion, type Completion, type CompletionContext } from '@codemirror/autocomplete'

export interface Api {
  /** Names are typed, so this is what the picker filters on. */
  /** The name, as typed. */
  label: string
  /** The signature, shown beside it. */
  sig: string
  /** One line on what it does. */
  info: string
  /** A statement, with `${placeholders}` to tab through. Gets its `;` added. */
  snippet?: string
  /** Used *inside* a bigger expression (`map(...) + 1`), so no `;` is added. */
  expr?: boolean
  /** The completion kind — `function`, `type`, `constant` — which picks the row's icon. */
  type?: string
  boost?: number
}

export const ARDUINO_API: Api[] = [
  // Digital I/O
  { label: 'pinMode', sig: 'pinMode(pin, mode)', info: 'Set a pin to INPUT, OUTPUT or INPUT_PULLUP.', snippet: 'pinMode(${pin}, ${mode});', boost: 2 },
  { label: 'digitalWrite', sig: 'digitalWrite(pin, value)', info: 'Drive a pin HIGH or LOW.', snippet: 'digitalWrite(${pin}, ${HIGH});', boost: 2 },
  { expr: true, label: 'digitalRead', sig: 'digitalRead(pin)', info: 'Read a pin as HIGH or LOW.', snippet: 'digitalRead(${pin})', boost: 2 },
  { expr: true, label: 'analogRead', sig: 'analogRead(pin)', info: 'Read an analog pin: 0 to 1023.', snippet: 'analogRead(${A0})' },
  { label: 'analogWrite', sig: 'analogWrite(pin, value)', info: 'PWM on a digital pin: 0 to 255.', snippet: 'analogWrite(${pin}, ${value})' },
  { label: 'analogReference', sig: 'analogReference(type)', info: 'Pick the reference analogRead measures against.' },
  { label: 'tone', sig: 'tone(pin, frequency, duration)', info: 'Play a square wave on a pin.' },
  { label: 'noTone', sig: 'noTone(pin)', info: 'Stop a tone started by tone().' },
  { expr: true, label: 'pulseIn', sig: 'pulseIn(pin, value, timeout)', info: 'Measure a pulse length in microseconds.', snippet: 'pulseIn(${pin}, ${HIGH})' },
  { label: 'shiftOut', sig: 'shiftOut(dataPin, clockPin, bitOrder, value)', info: 'Shift a byte out one bit at a time.' },
  { expr: true, label: 'shiftIn', sig: 'shiftIn(dataPin, clockPin, bitOrder)', info: 'Shift a byte in one bit at a time.' },

  // Time
  { label: 'delay', sig: 'delay(ms)', info: 'Wait, in milliseconds. Blocks everything.', snippet: 'delay(${1000});', boost: 2 },
  { label: 'delayMicroseconds', sig: 'delayMicroseconds(us)', info: 'Wait, in microseconds.' },
  { expr: true, label: 'millis', sig: 'millis()', info: 'Milliseconds since the sketch started. wraps after ~50 days.', snippet: 'millis()', boost: 1 },
  { expr: true, label: 'micros', sig: 'micros()', info: 'Microseconds since the sketch started.', snippet: 'micros()' },

  // Math
  { expr: true, label: 'map', sig: 'map(value, fromLow, fromHigh, toLow, toHigh)', info: 'Rescale a number from one range to another.', snippet: 'map(${value}, ${0}, ${1023}, ${0}, ${255})' },
  { expr: true, label: 'constrain', sig: 'constrain(value, low, high)', info: 'Clamp a number into a range.', snippet: 'constrain(${value}, ${low}, ${high})' },
  { expr: true, label: 'min', sig: 'min(a, b)', info: 'The smaller of two numbers.', snippet: 'min(${a}, ${b})' },
  { expr: true, label: 'max', sig: 'max(a, b)', info: 'The larger of two numbers.', snippet: 'max(${a}, ${b})' },
  { expr: true, label: 'abs', sig: 'abs(x)', info: 'Absolute value.' },
  { expr: true, label: 'pow', sig: 'pow(base, exponent)', info: 'Raise to a power.' },
  { expr: true, label: 'sqrt', sig: 'sqrt(x)', info: 'Square root.' },
  { expr: true, label: 'random', sig: 'random(max)', info: 'A pseudo-random number (long).', snippet: 'random(${max})' },
  { label: 'randomSeed', sig: 'randomSeed(seed)', info: 'Seed the random generator.' },

  // Bits and interrupts
  { label: 'interrupts', sig: 'interrupts()', info: 'Re-enable interrupts after noInterrupts().' },
  { label: 'noInterrupts', sig: 'noInterrupts()', info: 'Disable interrupts for a critical section.' },
  { label: 'attachInterrupt', sig: 'attachInterrupt(interrupt, isr, mode)', info: 'Run a function when a pin changes.', snippet: 'attachInterrupt(digitalPinToInterrupt(${2}), ${onChange}, ${RISING});' },
  { label: 'detachInterrupt', sig: 'detachInterrupt(interrupt)', info: 'Stop an attached interrupt.' },
  { expr: true, label: 'digitalPinToInterrupt', sig: 'digitalPinToInterrupt(pin)', info: 'Turn a pin number into an interrupt number.', snippet: 'digitalPinToInterrupt(${pin})' },
  { expr: true, label: 'bitRead', sig: 'bitRead(value, bit)', info: 'Read one bit of a number.' },
  { label: 'bitSet', sig: 'bitSet(value, bit)', info: 'Set one bit of a number.' },
  { label: 'bitClear', sig: 'bitClear(value, bit)', info: 'Clear one bit of a number.' },

  // Sketch structure
  { label: 'setup', sig: 'void setup()', info: 'Runs once, when the board starts.', snippet: 'void setup() {\n\t${}\n}', boost: 1 },
  { label: 'loop', sig: 'void loop()', info: 'Runs over and over, forever.', snippet: 'void loop() {\n\t${}\n}', boost: 1 },

  // Types
  { label: 'int', sig: 'int', info: '-32768 to 32767.', type: 'type', boost: 1 },
  { label: 'unsigned long', sig: 'unsigned long', info: '32-bit unsigned — for millis() comparisons.', type: 'type' },
  { label: 'long', sig: 'long', info: '-2147483648 to 2147483647.', type: 'type' },
  { label: 'byte', sig: 'byte', info: '0 to 255.', type: 'type' },
  { label: 'bool', sig: 'bool', info: 'true or false.', type: 'type' },
  { label: 'float', sig: 'float', info: 'A decimal number, ~6 digits.', type: 'type' },
  { label: 'double', sig: 'double', info: 'Same as float on AVR.', type: 'type' },
  { label: 'char', sig: 'char', info: 'One character.', type: 'type' },
  { label: 'String', sig: 'String', info: 'An Arduino string object.', type: 'type' },
  { label: 'void', sig: 'void', info: 'No value returned.', type: 'type' },

  // Constants
  { label: 'HIGH', sig: 'HIGH', info: 'A high pin: 5 volts.', type: 'constant', boost: 1 },
  { label: 'LOW', sig: 'LOW', info: 'A low pin: 0 volts.', type: 'constant', boost: 1 },
  { label: 'INPUT', sig: 'INPUT', info: 'Pin mode: read a level.', type: 'constant', boost: 1 },
  { label: 'OUTPUT', sig: 'OUTPUT', info: 'Pin mode: drive a level.', type: 'constant', boost: 1 },
  { label: 'INPUT_PULLUP', sig: 'INPUT_PULLUP', info: 'Input with the internal pull-up on — a button to GND reads LOW when pressed.', type: 'constant', boost: 2 },
  { label: 'LED_BUILTIN', sig: 'LED_BUILTIN', info: 'The onboard LED, pin 13 on an Uno.', type: 'constant', boost: 2 },
  { label: 'A0', sig: 'A0', info: 'Analog input 0.', type: 'constant' },
  { label: 'A1', sig: 'A1', info: 'Analog input 1.', type: 'constant' },
  { label: 'A2', sig: 'A2', info: 'Analog input 2.', type: 'constant' },
  { label: 'A3', sig: 'A3', info: 'Analog input 3.', type: 'constant' },
  { label: 'A4', sig: 'A4', info: 'Analog input 4.', type: 'constant' },
  { label: 'A5', sig: 'A5', info: 'Analog input 5.', type: 'constant' },
  { label: 'RISING', sig: 'RISING', info: 'Interrupt on a rising edge.', type: 'constant' },
  { label: 'FALLING', sig: 'FALLING', info: 'Interrupt on a falling edge.', type: 'constant' },
  { label: 'CHANGE', sig: 'CHANGE', info: 'Interrupt on any change.', type: 'constant' },
  { label: 'true', sig: 'true', info: 'Boolean true.', type: 'constant' },
  { label: 'false', sig: 'false', info: 'Boolean false.', type: 'constant' },
  { label: 'nullptr', sig: 'nullptr', info: 'A pointer to nothing.', type: 'constant' },
]

/** The `Serial` object's members, offered after `Serial.`. */
export const SERIAL_MEMBERS: Api[] = [
  { label: 'begin', sig: 'Serial.begin(speed)', info: 'Start serial at a baud rate. 9600 is the usual.', snippet: 'begin(${9600});', boost: 2 },
  { label: 'print', sig: 'Serial.print(value)', info: 'Print without a newline.', snippet: 'print(${value})', boost: 2 },
  { label: 'println', sig: 'Serial.println(value)', info: 'Print and end the line.', snippet: 'println(${value})', boost: 2 },
  { expr: true, label: 'available', sig: 'Serial.available()', info: 'How many bytes are waiting to be read.', snippet: 'available()' },
  { expr: true, label: 'read', sig: 'Serial.read()', info: 'Read one byte, or -1 if nothing is waiting.', snippet: 'read()' },
  { expr: true, label: 'readString', sig: 'Serial.readString()', info: 'Read until it times out.', snippet: 'readString()' },
  { label: 'write', sig: 'Serial.write(byte)', info: 'Write raw bytes.', snippet: 'write(${byte})' },
  { label: 'flush', sig: 'Serial.flush()', info: 'Wait for outgoing data to finish sending.', snippet: 'flush()' },
  { label: 'end', sig: 'Serial.end()', info: 'Release the serial pins.', snippet: 'end()' },
]

/**
 * The text a snippet inserts. Statements end in `;` — that is the promise, added
 * here so the table stays readable — while expressions (`map(...)`, `millis()`)
 * are used inside something larger, where a `;` would be wrong.
 */
export function snippetFor(entry: Api): string | undefined {
  if (!entry.snippet) return undefined
  if (entry.expr || /[;}]$/.test(entry.snippet.trimEnd())) return entry.snippet
  return `${entry.snippet};`
}

function toCompletion(entry: Api): Completion {
  // A snippet carries a whole statement, cursor in the first argument — which is
  // where the `;` comes from without hijacking Enter.
  const text = snippetFor(entry)
  if (text) {
    return snippetCompletion(text, {
      label: entry.label,
      detail: entry.sig,
      info: entry.info,
      type: entry.type,
      boost: entry.boost,
    })
  }
  return {
    label: entry.label,
    detail: entry.sig,
    info: entry.info,
    type: entry.type,
    boost: entry.boost,
  }
}

/**
 * The completion source. Everything is looked up from the word being typed; after
 * `Serial.` it offers that object's members instead.
 */
export function arduinoCompletions(context: CompletionContext) {
  const word = context.matchBefore(/[\w.]*/)
  if (!word) return null
  // `Serial.` — offer the members, replacing nothing, so the result reads
  // `Serial.print(`.
  const member = /^([A-Za-z_]\w*)\.$/.exec(word.text)
  if (member && member[1] === 'Serial') {
    return { from: context.pos, options: SERIAL_MEMBERS.map(toCompletion) }
  }
  // Don't pop up on an empty prefix unless the user asked (Ctrl-Space).
  if (word.text === '' && !context.explicit) return null
  const prefix = word.text.toLowerCase()
  const options = ARDUINO_API.filter((e) => e.label.toLowerCase().startsWith(prefix)).map(toCompletion)
  if (!options.length) return null
  return { from: word.from, options }
}
