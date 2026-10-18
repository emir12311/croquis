/**
 * The circuit document's state: parts, wires, undo history, clipboard and the
 * dirty flag.
 *
 * The view, the file system and the simulator are deliberately absent. Their
 * seams are injected (`ProjectDeps`) or live in sibling modules, which is what
 * lets this be tested without a browser: see `project.test.ts`.
 */
import {
  attrsOf,
  clipboardOf,
  dedupeWires,
  historyPush,
  renderOrderOf,
  sameWire,
  snapshotOf,
  WIRE_GRID,
  type Clipboard,
  type Instance,
  type Point,
  type Snapshot,
  type Wire,
} from './document'
import { isBreadboard, type PartKind } from './parts'
import { snapToHoles } from './snap'
import { BREADBOARD_HOLE_R, BREADBOARD_PITCH } from './breadboard'

/** A part within a hole's pitch of one snaps into it; anything further is free. */
const SNAP = { reach: BREADBOARD_PITCH * 1.1, seat: BREADBOARD_HOLE_R }

/** The geometry a project needs but does not own (it depends on mounted elements). */
export interface ProjectDeps {
  /** False while a simulation runs: the circuit is frozen then. */
  canEdit(): boolean
  /** Where a new part should sit — in a breadboard hole, or on the free grid. */
  place(kind: PartKind, x: number, y: number): Point
  /** A part's pin offsets from its own origin, rotation included. */
  pinOffsets(inst: Instance): Point[]
  /** Every breadboard hole, in canvas space. */
  boardHolePoints(): Point[]
}

export class Project {
  instances = $state<Instance[]>([])
  wires = $state<Wire[]>([])
  clipboard = $state<Clipboard | null>(null)
  /** Snapshots that undo can return to; the newest is last. */
  history = $state<Snapshot[]>([])
  future = $state<Snapshot[]>([])
  /**
   * Whether anything here differs from what is on disk. `commit()` is the one
   * "the circuit changed" hook; the sketch is watched by the caller, because
   * typing does not go through this class.
   */
  dirty = $state(false)

  /** Breadboards draw behind everything else. */
  renderOrder = $derived(renderOrderOf(this.instances))

  private seq = 0
  private deps: ProjectDeps

  constructor(deps: ProjectDeps) {
    this.deps = deps
  }

  snapshot(): Snapshot {
    return snapshotOf(this.instances, this.wires)
  }

  /** Record the current state, so the mutation that follows can be undone. */
  commit() {
    this.history = historyPush(this.history, this.snapshot())
    this.future = []
    this.dirty = true
  }

  markSaved() {
    this.dirty = false
  }

  /** Returns false when there was nothing to undo. */
  undo(): boolean {
    const prev = this.history[this.history.length - 1]
    if (!prev) return false
    this.future = [...this.future, this.snapshot()]
    this.history = this.history.slice(0, -1)
    this.instances = prev.instances
    this.wires = prev.wires
    // Undoing after a save moves the document away from the file on disk, so it
    // is dirty again. Without this, File -> New would discard it silently.
    this.dirty = true
    return true
  }

  redo(): boolean {
    const next = this.future[this.future.length - 1]
    if (!next) return false
    this.history = [...this.history, this.snapshot()]
    this.future = this.future.slice(0, -1)
    this.instances = next.instances
    this.wires = next.wires
    this.dirty = true
    return true
  }

  /** Adopt a loaded circuit: new history, no clipboard, and in step with disk. */
  load(instances: Instance[], wires: Wire[]) {
    this.instances = instances
    this.wires = wires
    this.history = []
    this.future = []
    this.clipboard = null
    this.dirty = false
  }

  reset() {
    this.load([], [])
  }

  /** Place a part, snapping it to a hole or the grid. One undo step. */
  add(
    kind: PartKind,
    x: number,
    y: number,
    /** An instance being copied: its look and rotation come along. */
    copy?: { attrs?: Record<string, string>; rotate?: number },
    /**
     * Whether to look for a hole or grid line under it. On for dropping one part;
     * off for a paste, where each part is positioned relative to the rest of the
     * copy and snapping them one at a time would pull the group out of shape.
     */
    snap = true,
  ): Instance | undefined {
    if (!this.deps.canEdit()) return
    this.commit()
    return this.create(kind, x, y, copy, snap)
  }

  /** The placement itself, with no undo step — a paste batches its own. */
  private create(
    kind: PartKind,
    x: number,
    y: number,
    copy?: { attrs?: Record<string, string>; rotate?: number },
    snap = true,
  ): Instance {
    let id: string
    do {
      id = `i${++this.seq}`
    } while (this.instances.some((i) => i.id === id))
    const rotate = copy?.rotate ?? 0
    const inst: Instance = {
      id,
      kind,
      x,
      y,
      ...(copy?.attrs ? { attrs: { ...copy.attrs } } : {}),
      ...(rotate ? { rotate } : {}),
    }
    this.instances.push(inst)
    // A rotated copy keeps the position it was pasted at and re-seats on the
    // first nudge: its hole offsets would need rotating about the element's box,
    // which is only known once it has mounted.
    if (snap && !rotate) {
      const placed = this.deps.place(kind, inst.x, inst.y)
      inst.x = placed.x
      inst.y = placed.y
    }
    return inst
  }

  addWire(wire: Wire) {
    this.commit()
    this.wires.push(wire)
  }

  /** Wires decoded from a file: part of the load, so not an undo step. */
  setWires(wires: Wire[]) {
    this.wires = wires
  }

  /** Remove the selected wire, or the selected parts and every wire touching them. */
  deleteSelection(selectedIds: string[], selectedWireId: string | null) {
    if (selectedWireId) {
      this.commit()
      this.wires = this.wires.filter((w) => w.id !== selectedWireId)
    } else if (selectedIds.length) {
      this.commit()
      const going = new Set(selectedIds)
      this.wires = this.wires.filter(
        (w) => !going.has(w.a.instanceId) && !going.has(w.b.instanceId),
      )
      this.instances = this.instances.filter((i) => !going.has(i.id))
    }
  }

  copySelection(selectedIds: string[]) {
    this.clipboard = clipboardOf(this.instances, selectedIds)
  }

  cutSelection(selectedIds: string[], selectedWireId: string | null) {
    this.copySelection(selectedIds)
    this.deleteSelection(selectedIds, selectedWireId)
  }

  /** Paste the clipboard and return the ids that were created. One undo step. */
  paste(): string[] {
    const clip = this.clipboard
    if (!clip || !this.deps.canEdit()) return []
    // Each paste steps along, so pasting twice does not stack two copies in one spot.
    const x = clip.x + WIRE_GRID
    const y = clip.y + WIRE_GRID
    this.commit()
    const pasted = clip.parts.map(
      (part) => this.create(part.kind, x + part.dx, y + part.dy, part, false).id,
    )
    this.clipboard = { ...clip, x, y }
    return pasted
  }

  duplicateSelection(selectedIds: string[]): string[] {
    if (!selectedIds.length) return []
    this.copySelection(selectedIds)
    return this.paste()
  }

  /** `quarterTurns` is 1 for the R shortcut, 2 for "Rotate 180°". One undo step. */
  rotate(selectedIds: string[], quarterTurns = 1) {
    const turning = this.instances.filter((i) => selectedIds.includes(i.id))
    if (!turning.length) return
    // One snapshot for the whole turn, so undo puts them all back together.
    this.commit()
    for (const inst of turning) this.rotateOne(inst, quarterTurns)
  }

  private rotateOne(inst: Instance, quarterTurns: number) {
    inst.rotate = ((inst.rotate ?? 0) + 90 * quarterTurns) % 360
    // Rotating moves the legs, so put the part back down where they still land in
    // holes. On a breadboard you pull a part out and push it back in.
    if (!isBreadboard(inst.kind)) {
      const seated = snapToHoles(
        { x: inst.x, y: inst.y },
        this.deps.pinOffsets(inst),
        this.deps.boardHolePoints(),
        SNAP,
      )
      if (seated) {
        inst.x = seated.x
        inst.y = seated.y
      }
    }
  }

  /** Move the selection by whole grid steps. One undo step for the lot. */
  nudge(selectedIds: string[], dx: number, dy: number) {
    if (!selectedIds.length) return
    this.commit()
    for (const id of selectedIds) {
      const inst = this.instances.find((i) => i.id === id)
      if (!inst) continue
      inst.x += dx
      inst.y += dy
    }
  }

  /** Change one property of a part (undoable). */
  setAttr(inst: Instance, name: string, value: string) {
    if (attrsOf(inst)[name] === value) return
    this.commit()
    inst.attrs = { ...inst.attrs, [name]: value }
  }

  setWireColour(wireId: string | null, name: string) {
    const wire = wireId ? this.wires.find((w) => w.id === wireId) : null
    if (!wire || wire.colour === name) return
    this.commit()
    wire.colour = name
  }

  dedupeWires() {
    this.wires = dedupeWires(this.wires)
  }

  /**
   * Finish wiring to `target`: clicking the same two pins again removes that
   * wire, otherwise a new one is added. Returns the wire that was removed, if any.
   */
  toggleWire(from: { instanceId: string; pin: string }, target: { instanceId: string; pin: string }, newWire: () => Wire): Wire | null {
    const candidate = { id: '', a: from, b: target, waypoints: [] } as Wire
    const existing = this.wires.find((w) => sameWire(w, candidate))
    this.commit()
    if (existing) {
      this.wires = this.wires.filter((w) => w.id !== existing.id)
      return existing
    }
    this.wires.push(newWire())
    return null
  }

  clear() {
    if (!this.instances.length && !this.wires.length) return
    this.commit()
    this.instances = []
    this.wires = []
  }
}
