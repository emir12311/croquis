/**
 * The circuit document: the shapes a circuit is made of, and the pure operations
 * that do not care about the DOM, the view, or the file system.
 *
 * State and history live in `project.svelte.ts`; this module exists so the
 * snapshot/clipboard/attribute rules can be tested without mounting anything.
 */
import { partByKind, isBreadboard, type PartKind } from './parts'

export interface Point {
  x: number
  y: number
}

export interface PinRef {
  instanceId: string
  pin: string
}

export interface Instance {
  id: string
  kind: PartKind
  x: number
  y: number
  /** Rotation in degrees (0/90/180/270), applied about the part's centre. */
  rotate?: number
  /** This part's own attributes, overriding the catalogue's defaults. */
  attrs?: Record<string, string>
}

export interface Wire {
  id: string
  a: PinRef
  b: PinRef
  waypoints: Point[]
  /** Named colour (Wokwi's red/blue/green/…); unset uses the themed wire colour. */
  colour?: string
}

export interface Snapshot {
  instances: Instance[]
  wires: Wire[]
}

/** One part on the clipboard, offset from the top-left of the copy it came from. */
export interface ClipboardPart {
  kind: PartKind
  dx: number
  dy: number
  /** A copy carries how a part *looks*, not just what it is. */
  attrs?: Record<string, string>
  rotate?: number
}

export interface Clipboard {
  x: number
  y: number
  parts: ClipboardPart[]
}

/** Wires and parts snap to this fine grid, so segments run along grid lines. */
export const WIRE_GRID = 12

/** The most undo steps kept. Old ones fall off the bottom. */
export const HISTORY_LIMIT = 200

export function snapToGrid(v: number): number {
  return Math.round(v / WIRE_GRID) * WIRE_GRID
}

/** A part's attributes: the catalogue's defaults, with its own on top. */
export function attrsOf(inst: Instance): Record<string, string> {
  return { ...partByKind(inst.kind).attrs, ...inst.attrs }
}

/**
 * A copy deep enough to undo to: parts, wires and waypoints are all copied, so
 * later mutations cannot reach back into the snapshot.
 */
export function snapshotOf(instances: Instance[], wires: Wire[]): Snapshot {
  return {
    instances: instances.map((i) => ({
      ...i,
      ...(i.attrs ? { attrs: { ...i.attrs } } : {}),
    })),
    wires: wires.map((w) => ({
      ...w,
      a: { ...w.a },
      b: { ...w.b },
      waypoints: w.waypoints.map((p) => ({ ...p })),
    })),
  }
}

/** Append a snapshot, dropping the oldest once the limit is reached. */
export function historyPush(history: Snapshot[], snapshot: Snapshot): Snapshot[] {
  return [...history, snapshot].slice(-HISTORY_LIMIT)
}

/** Two pins point at each other. */
export function samePin(a: PinRef, b: PinRef): boolean {
  return a.instanceId === b.instanceId && a.pin === b.pin
}

/** Two wires joining the same two pins, in either direction. */
export function sameWire(a: Wire, b: Wire): boolean {
  return (
    (samePin(a.a, b.a) && samePin(a.b, b.b)) || (samePin(a.a, b.b) && samePin(a.b, b.a))
  )
}

/** Drop duplicate wires, keeping the first of each pin pair. */
export function dedupeWires(wires: Wire[]): Wire[] {
  const seen = new Set<string>()
  return wires.filter((w) => {
    const ka = `${w.a.instanceId}:${w.a.pin}`
    const kb = `${w.b.instanceId}:${w.b.pin}`
    const key = ka < kb ? `${ka}|${kb}` : `${kb}|${ka}`
    if (seen.has(key)) return false
    seen.add(key)
    return true
  })
}

/**
 * Breadboards draw behind everything else, so a board placed late cannot swallow
 * the parts placed before it. Within each group it is placement order.
 */
export function renderOrderOf(instances: Instance[]): Instance[] {
  return [...instances].sort(
    (a, b) => (isBreadboard(a.kind) ? 0 : 1) - (isBreadboard(b.kind) ? 0 : 1),
  )
}

/** What a copy of the selection should hold, positioned relative to its top-left. */
export function clipboardOf(instances: Instance[], selectedIds: string[]): Clipboard | null {
  const parts = instances.filter((i) => selectedIds.includes(i.id))
  if (!parts.length) return null
  const x = Math.min(...parts.map((p) => p.x))
  const y = Math.min(...parts.map((p) => p.y))
  return {
    x,
    y,
    parts: parts.map((p) => ({
      kind: p.kind,
      dx: p.x - x,
      dy: p.y - y,
      attrs: attrsOf(p),
      rotate: p.rotate ?? 0,
    })),
  }
}
