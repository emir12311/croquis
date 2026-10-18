/**
 * The `diagram.json` format: reading and writing the file, kept separate from the app's
 * own state so it can be tested on its own.
 *
 * The app owns *applying* a diagram (instances, wires, undo history, selection) and the
 * geometry that only it knows — where a pin sits. Everything that is purely about the
 * format lives here, which is what makes "saves / loads as `diagram.json`" provable
 * instead of asserted.
 *
 * The format is Wokwi's, deliberately: a file this app writes opens in Wokwi and vice
 * versa, and unknown fields are ignored on both sides — which is how the sketch, and the
 * presets' titles, ride along in the same file.
 */
import { kindForTag, partByKind, type PartKind } from './parts'
import { encodeRoute, type Point } from './route'

export interface PinRef {
  instanceId: string
  pin: string
}

export interface DiagramPart {
  id: string
  type: string
  left: number
  top: number
  attrs: Record<string, string>
  rotate?: number
}

/** Wokwi's connection shape: `[from, to, colour, route]`. */
export type DiagramConnection = [string, string, string, string[]]

/** The colour a wire gets unless it says otherwise. */
export const WIRE_COLOR = 'green'

/** A placed part, as the app holds it. */
export interface PlacedPart {
  id: string
  kind: PartKind
  x: number
  y: number
  rotate?: number
  /** Optional to match an instance: a part with no attributes of its own has none. */
  attrs?: Record<string, string>
}

/** A wire, as the app holds it. */
export interface RoutedWire {
  a: PinRef
  b: PinRef
  colour?: string
  waypoints: Point[]
}

export interface DiagramFile {
  version: number
  author: string
  editor: string
  parts: DiagramPart[]
  connections: DiagramConnection[]
  sketch: string
}

/**
 * Turn the app's circuit into the file format.
 *
 * `pinPoint` is injected because only the app can answer it: pin positions come from the
 * rendered elements. Taking the answer as an argument is also what made this testable —
 * reaching for it directly is why the format had no test until the audit.
 */
export function buildDiagram(input: {
  parts: PlacedPart[]
  wires: RoutedWire[]
  sketch: string
  pinPoint: (ref: PinRef) => Point | null
}): DiagramFile {
  const parts: DiagramPart[] = input.parts.map((inst) => ({
    id: inst.id,
    type: partByKind(inst.kind).tag,
    left: Math.round(inst.x),
    top: Math.round(inst.y),
    attrs: { ...inst.attrs },
    ...(inst.rotate ? { rotate: inst.rotate } : {}),
  }))

  const connections: DiagramConnection[] = []
  for (const wire of input.wires) {
    const a = input.pinPoint(wire.a)
    const b = input.pinPoint(wire.b)
    // A wire whose ends cannot be located has no route to record; dropping it is
    // better than writing a corner at 0,0.
    if (!a || !b) continue
    connections.push([
      `${wire.a.instanceId}:${wire.a.pin}`,
      `${wire.b.instanceId}:${wire.b.pin}`,
      wire.colour ?? WIRE_COLOR,
      encodeRoute(a, b, wire.waypoints),
    ])
  }

  return { version: 1, author: 'croquis', editor: 'croquis', parts, connections, sketch: input.sketch }
}

/** `"uno:13"` → its part and pin. */
export function parseRef(ref: string): PinRef | null {
  const i = ref.indexOf(':')
  if (i <= 0) return null
  return { instanceId: ref.slice(0, i), pin: ref.slice(i + 1) }
}

export interface ParsedPart {
  id: string
  kind: PartKind
  x: number
  y: number
  rotate: number
  attrs?: Record<string, string>
}

export interface ParsedWire {
  a: PinRef
  b: PinRef
  /** The route as written — decoded once the part geometries are known. */
  route: string[]
  colour?: string
}

export interface ParsedDiagram {
  parts: ParsedPart[]
  wires: ParsedWire[]
  sketch: string | null
}

/**
 * Read a `diagram.json` into the app's shapes.
 *
 * Anything the catalogue does not recognise — a part from a newer version, a connection
 * naming a part that is not there — is skipped rather than guessed at. That is what lets
 * an unfamiliar file load, and it is also why a mistyped pin in a preset fails silently.
 */
export function parseDiagram(text: string): ParsedDiagram {
  const doc = JSON.parse(text) as {
    parts?: {
      id?: unknown
      type?: unknown
      left?: unknown
      top?: unknown
      rotate?: unknown
      attrs?: unknown
    }[]
    connections?: unknown[]
    sketch?: unknown
  }

  const parts: ParsedPart[] = []
  const known = new Set<string>()
  for (const p of doc.parts ?? []) {
    const kind = typeof p.type === 'string' ? kindForTag(p.type) : undefined
    if (!kind || typeof p.id !== 'string') continue
    const rotate = typeof p.rotate === 'number' ? ((p.rotate % 360) + 360) % 360 : 0
    // Keep any attributes the file carries (Wokwi writes them all), so a 10 kΩ
    // resistor stays a 10 kΩ resistor rather than silently reverting to ours.
    const attrs: Record<string, string> = {}
    if (p.attrs && typeof p.attrs === 'object') {
      for (const [key, value] of Object.entries(p.attrs as Record<string, unknown>)) {
        if (typeof value === 'string') attrs[key] = value
      }
    }
    parts.push({
      id: p.id,
      kind,
      x: Number(p.left) || 0,
      y: Number(p.top) || 0,
      rotate,
      ...(Object.keys(attrs).length ? { attrs } : {}),
    })
    known.add(p.id)
  }

  const wires: ParsedWire[] = []
  for (const c of doc.connections ?? []) {
    if (!Array.isArray(c) || c.length < 2) continue
    const a = parseRef(String(c[0]))
    const b = parseRef(String(c[1]))
    if (!a || !b || !known.has(a.instanceId) || !known.has(b.instanceId)) continue
    const route = Array.isArray(c[3]) ? (c[3] as unknown[]).map(String) : []
    const colour = typeof c[2] === 'string' && c[2] && c[2] !== WIRE_COLOR ? c[2] : undefined
    wires.push({ a, b, route, colour })
  }

  return { parts, wires, sketch: typeof doc.sketch === 'string' ? doc.sketch : null }
}
