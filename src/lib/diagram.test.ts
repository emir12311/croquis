import { describe, expect, it } from 'vitest'
import { PRESETS } from './presets'
import { decodeRoute } from './route'
import {
  WIRE_COLOR,
  buildDiagram,
  parseDiagram,
  parseRef,
  type PinRef,
  type PlacedPart,
  type RoutedWire,
} from './diagram'

/**
 * Pin positions are injected in the app because only it knows them. Here they can be
 * any consistent numbers: what is under test is the format, not the layout.
 */
const pinPoint = (ref: PinRef) => ({ x: 100 + ref.pin.length * 7, y: 40 + ref.instanceId.length * 13 })

const circuit = (): { parts: PlacedPart[]; wires: RoutedWire[]; sketch: string } => ({
  parts: [
    { id: 'uno', kind: 'uno' as const, x: 60, y: 80, attrs: {} },
    { id: 'r1', kind: 'resistor' as const, x: 430, y: 200, rotate: 90, attrs: { value: '4700' } },
    { id: 'led1', kind: 'led' as const, x: 580, y: 170, attrs: { color: 'blue' } },
  ],
  wires: [{ a: { instanceId: 'r1', pin: '2' }, b: { instanceId: 'led1', pin: 'A' }, waypoints: [] }],
  sketch: 'void setup() {}\nvoid loop() {}\n',
})

describe('the diagram.json format', () => {
  it('writes the Wokwi shape, with the sketch riding along', () => {
    const doc = buildDiagram({ ...circuit(), pinPoint })
    expect(doc.version).toBe(1)
    expect(doc.parts.map((p) => p.type)).toEqual(['wokwi-arduino-uno', 'wokwi-resistor', 'wokwi-led'])
    expect(doc.connections[0].slice(0, 3)).toEqual(['r1:2', 'led1:A', WIRE_COLOR])
    expect(doc.sketch).toMatch(/void setup/)
  })

  it('keeps position, rotation and attributes through a save and a load', () => {
    const back = parseDiagram(JSON.stringify(buildDiagram({ ...circuit(), pinPoint })))
    expect(back.parts.map((p) => [p.id, p.kind, p.x, p.y, p.rotate])).toEqual([
      ['uno', 'uno', 60, 80, 0],
      ['r1', 'resistor', 430, 200, 90],
      ['led1', 'led', 580, 170, 0],
    ])
    // A 4.7 kΩ stays a 4.7 kΩ rather than reverting to the catalogue's default.
    expect(back.parts[1].attrs).toEqual({ value: '4700' })
    expect(back.parts[2].attrs).toEqual({ color: 'blue' })
    expect(back.sketch).toBe(circuit().sketch)
  })

  it('sends a circuit back through unchanged — the round trip', () => {
    const first = parseDiagram(JSON.stringify(buildDiagram({ ...circuit(), pinPoint })))
    const again = buildDiagram({
      parts: first.parts,
      wires: first.wires.map((w) => ({
        a: w.a,
        b: w.b,
        colour: w.colour,
        waypoints: decodeRoute(pinPoint(w.a), w.route),
      })),
      sketch: first.sketch ?? '',
      pinPoint,
    })
    expect(parseDiagram(JSON.stringify(again))).toEqual(first)
  })

  it('loads every shipped preset and gives back the same circuit', () => {
    // The presets are real diagram.json files, so they are the honest input for this.
    expect(PRESETS.length).toBeGreaterThan(0)
    for (const preset of PRESETS) {
      const once = parseDiagram(preset.text)
      expect(once.parts.length, preset.id).toBeGreaterThan(0)
      expect(once.sketch, preset.id).toMatch(/void loop/)
      const rebuilt = buildDiagram({
        parts: once.parts,
        wires: once.wires.map((w) => ({
          a: w.a,
          b: w.b,
          colour: w.colour,
          waypoints: decodeRoute(pinPoint(w.a), w.route),
        })),
        sketch: once.sketch ?? '',
        pinPoint,
      })
      const twice = parseDiagram(JSON.stringify(rebuilt))
      expect(twice.parts, preset.id).toEqual(once.parts)
      expect(twice.sketch, preset.id).toBe(once.sketch)
      expect(
        twice.wires.map((w) => [w.a, w.b]),
        preset.id,
      ).toEqual(once.wires.map((w) => [w.a, w.b]))
    }
  })

  it('skips what it does not recognise rather than guessing', () => {
    const doc = parseDiagram(
      JSON.stringify({
        parts: [
          { id: 'uno', type: 'wokwi-arduino-uno', left: 10, top: 20, attrs: {} },
          { id: 'x', type: 'wokwi-something-new', left: 0, top: 0 },
          { type: 'wokwi-led', left: 0, top: 0 },
        ],
        connections: [
          ['uno:13', 'ghost:A'],
          ['uno:13', 'uno:GND.2', 'red', ['v10']],
          'nonsense',
          ['only-one'],
        ],
        sketch: 'void setup() {}\nvoid loop() {}\n',
      }),
    )
    expect(doc.parts.map((p) => p.id)).toEqual(['uno'])
    // Only the connection whose both ends exist; the colour survives, and so does the route.
    expect(doc.wires).toHaveLength(1)
    expect(doc.wires[0].colour).toBe('red')
    expect(doc.wires[0].route).toEqual(['v10'])
  })

  it('treats the default wire colour as no colour at all', () => {
    const doc = parseDiagram(
      JSON.stringify({
        parts: [{ id: 'uno', type: 'wokwi-arduino-uno', left: 0, top: 0, attrs: {} }],
        connections: [['uno:13', 'uno:GND.2', WIRE_COLOR, []]],
      }),
    )
    expect(doc.wires[0].colour).toBeUndefined()
    expect(doc.sketch).toBeNull()
  })

  it('reads a pin reference, and refuses a malformed one', () => {
    expect(parseRef('uno:13')).toEqual({ instanceId: 'uno', pin: '13' })
    expect(parseRef('led1:C')).toEqual({ instanceId: 'led1', pin: 'C' })
    expect(parseRef('nocolon')).toBeNull()
    expect(parseRef(':pin-only')).toBeNull()
  })
})
