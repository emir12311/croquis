import { describe, expect, it } from 'vitest'
import { evaluateCircuit, type EvalPart, type EvalWire } from './netlist'
import { kindForTag } from './parts'
import { PRESETS } from './presets'

interface DiagramFile {
  title?: string
  parts: { id: string; type: string; left: number; top: number; attrs?: Record<string, string> }[]
  connections: string[][]
  sketch: string
}

/** A preset, turned into what the netlist evaluates — the same shape the app builds. */
function evalPreset(
  id: string,
  pressed: Record<string, boolean> = {},
  avrPins: Record<string, boolean> = { B5: true },
) {
  const preset = PRESETS.find((p) => p.id === id)
  if (!preset) throw new Error(`no preset called ${id}`)
  const doc = JSON.parse(preset.text) as DiagramFile
  const parts: EvalPart[] = doc.parts.map((part) => ({
    id: part.id,
    kind: kindForTag(part.type)!,
    attrs: part.attrs,
  }))
  const ref = (end: string) => {
    const at = end.indexOf(':')
    return { instanceId: end.slice(0, at), pin: end.slice(at + 1) }
  }
  const wires: EvalWire[] = doc.connections.map((c) => ({ a: ref(c[0]), b: ref(c[1]) }))
  return evaluateCircuit(parts, wires, pressed, avrPins)
}

describe('the presets we ship', () => {
  it('ships the circuits a first run should offer', () => {
    expect(PRESETS.length).toBeGreaterThanOrEqual(3)
    expect(PRESETS.map((p) => p.id)).toContain('blink')
  })

  it('gives each one a label someone can read', () => {
    for (const preset of PRESETS) {
      expect(preset.label, preset.id).not.toMatch(/[-_]/)
      expect(preset.label.trim(), preset.id).toBe(preset.label)
      expect(preset.label.length, preset.id).toBeGreaterThan(2)
    }
  })

  for (const preset of PRESETS) {
    describe(preset.id, () => {
      const doc = JSON.parse(preset.text) as DiagramFile

      it('carries parts, connections and a sketch', () => {
        expect(doc.parts.length).toBeGreaterThan(0)
        expect(doc.connections.length).toBeGreaterThan(0)
        expect(doc.sketch.length).toBeGreaterThan(20)
      })

      it('has a sketch with setup() and loop(), so it can actually run', () => {
        expect(doc.sketch).toMatch(/void\s+setup\s*\(/)
        expect(doc.sketch).toMatch(/void\s+loop\s*\(/)
      })

      it('places each part once, and only parts the catalogue knows', () => {
        const ids = doc.parts.map((p) => p.id)
        expect(new Set(ids).size, 'duplicate ids').toBe(ids.length)
        for (const part of doc.parts) {
          expect(kindForTag(part.type), `${preset.id}: ${part.type}`).toBeDefined()
        }
      })

      it('wires only parts it places, with a named pin on each end', () => {
        const ids = new Set(doc.parts.map((p) => p.id))
        for (const connection of doc.connections) {
          expect(connection.length, `${preset.id}: ${connection}`).toBeGreaterThanOrEqual(2)
          for (const ref of connection.slice(0, 2)) {
            const [id, pin] = ref.split(':')
            expect(ids.has(id), `${preset.id}: no part ${id}`).toBe(true)
            expect(pin, `${preset.id}: ${ref} has no pin`).toBeTruthy()
          }
        }
      })

      it('does not wire a part to itself', () => {
        for (const [a, b] of doc.connections) expect(a, preset.id).not.toBe(b)
      })
    })
  }
})

describe('what the shipped circuits actually do', () => {
  it('blink: the LED on pin 13 lights up', () => {
    expect(evalPreset('blink').leds.led1).toBe('lit')
  })

  it('button: held, pin 2 is pulled low; released, the pull-up takes it back high', () => {
    // This is what "the button works" means. The sketch lights the built-in LED
    // when pin 2 reads low, so the whole thing turns on these two drives.
    expect(evalPreset('button', { b1: true }).drives).toContainEqual({ port: 'D', pin: 2, high: false })
    expect(evalPreset('button', {}).drives).toContainEqual({ port: 'D', pin: 2, high: true })
  })

  it('button: held, it pulls the pin low even though the pin itself reads high', () => {
    // This is the bug that shipped. The pin is already being driven high, and pressing
    // the button makes its two sides one net — so that net holds the pin as well as the
    // rail. Deciding the pressed level from the net let the pin answer for itself:
    // it computed "drive high" forever and a press never changed anything.
    const held = evalPreset('button', { b1: true }, { B5: true, D2: true }).drives
    expect(held).toContainEqual({ port: 'D', pin: 2, high: false })
    // And it still reads low when nothing is driving the pin yet.
    const first = evalPreset('button', { b1: true }).drives
    expect(first).toContainEqual({ port: 'D', pin: 2, high: false })
  })

  it('button + serial: the same circuit, so the same drive', () => {
    expect(evalPreset('serial', { b1: true }).drives).toContainEqual({ port: 'D', pin: 2, high: false })
    expect(evalPreset('serial', {}).drives).toContainEqual({ port: 'D', pin: 2, high: true })
  })

  it('potentiometer: mid-track, A0 reads about half the supply, and wired right', () => {
    const out = evalPreset('potentiometer')
    expect(out.analogs).toContainEqual({ channel: 0, millivolts: 2502 })
    expect(out.warnings.pot1).toBeUndefined()
  })
})
