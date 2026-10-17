import { describe, expect, it } from 'vitest'
import { CompletionContext, type Completion } from '@codemirror/autocomplete'
import { cpp } from '@codemirror/lang-cpp'
import { EditorState } from '@codemirror/state'
import { ARDUINO_API, arduinoCompletions, SERIAL_MEMBERS, snippetFor } from './arduino-api'

/** What the source offers for a document, with the cursor at the end. */
function offered(doc: string, explicit = false): Completion[] {
  const state = EditorState.create({ doc, extensions: [cpp()] })
  const result = arduinoCompletions(new CompletionContext(state, doc.length, explicit))
  if (!result) return []
  return Array.isArray(result) ? result : result.options
}
const labels = (doc: string, explicit = false) => offered(doc, explicit).map((o) => o.label)

describe('the Arduino API table', () => {
  it('documents every entry: a signature and a sentence', () => {
    // This is where people look a function up, so an entry without its signature
    // and a line of explanation is a hole in the help, not just missing polish.
    for (const entry of [...ARDUINO_API, ...SERIAL_MEMBERS]) {
      expect(entry.sig, `${entry.label} has no signature`).toBeTruthy()
      // A floor, not a quality bar: something is actually said.
      expect(entry.info.trim().length, `${entry.label} has no explanation`).toBeGreaterThan(5)
    }
  })

  it('ends every statement in a semicolon, and leaves expressions alone', () => {
    // The complaint was "it doesn't put the ; automatically". It does now, where
    // a `;` belongs: a whole statement gets one, an expression must not, or it
    // would break `map(x, 0, 1023, 0, 255) + 1`.
    for (const entry of [...ARDUINO_API, ...SERIAL_MEMBERS]) {
      const text = snippetFor(entry)
      if (!text) continue
      if (entry.expr) {
        expect(text.endsWith(';'), `${entry.label} is an expression but ends in ;`).toBe(false)
      } else {
        expect(
          /[;}]$/.test(text),
          `${entry.label} is a statement but does not end in ; or }`,
        ).toBe(true)
      }
    }
  })

  it('has no duplicate names', () => {
    const names = ARDUINO_API.map((e) => e.label)
    expect(new Set(names).size).toBe(names.length)
  })
})

describe('completing a sketch', () => {
  it('offers the function being typed, as a whole statement', () => {
    const option = offered('dig').find((o) => o.label === 'digitalWrite')
    expect(option, labels('dig').join(', ')).toBeTruthy()
    expect(option?.detail).toBe('digitalWrite(pin, value)')
    expect(option?.info).toMatch(/HIGH or LOW/i)
    // The statement form is what carries the `;` — the thing that felt missing.
    expect(snippetFor(ARDUINO_API.find((e) => e.label === 'digitalWrite')!)).toContain('digitalWrite(')
  })

  it('completes the Serial members after a dot', () => {
    expect(labels('Serial.')).toEqual(expect.arrayContaining(['println', 'print', 'available']))
  })

  it('offers pin mode constants', () => {
    expect(labels('INPUT')).toEqual(expect.arrayContaining(['INPUT', 'INPUT_PULLUP']))
  })

  it('stays quiet on an empty prefix, and on words it does not know', () => {
    // Quiet means the editor's own word completion gets to handle it — the user's
    // own variables and functions.
    expect(offered('int x = 1;\n')).toHaveLength(0)
    expect(offered('zzzz')).toHaveLength(0)
  })

  it('lists everything when explicitly asked on an empty prefix', () => {
    expect(offered('', true).length).toBeGreaterThan(20)
  })

  it('is case-insensitive, the way a beginner types', () => {
    expect(labels('led_buil')).toContain('LED_BUILTIN')
    expect(labels('DIGITALW')).toContain('digitalWrite')
  })
})
