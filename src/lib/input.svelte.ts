/**
 * Canvas input: selection, wiring, dragging, panning and the hover state.
 *
 * The pointer handlers live here rather than in the component so that "what a
 * press means" has one home: press a pin, drag a part, drop a bend, box-select,
 * pan. It reads the document, the viewport and the surface; it does not own the
 * simulation or the file format.
 */
import { samePin, snapToGrid, WIRE_GRID, type Instance, type PinRef, type Point, type Wire } from './document'
import type { PartDef, PartKind } from './parts'
import type { Project } from './project.svelte'
import type { View } from './view.svelte'
import type { CanvasSurface, PinGeom } from './canvas.svelte'
import type { Sim } from './sim.svelte'
import { expandPath, legBend, livePoints } from './route'

export interface InputDeps {
  project: Project
  view: View
  surface: CanvasSurface
  sim: Sim
  canvasEl(): HTMLDivElement | null
  /** Close the menu bar, which a canvas press dismisses. */
  closeMenus(): void
}

/** Distance from a point to a line segment. */
function distToSegment(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x
  const dy = b.y - a.y
  const len2 = dx * dx + dy * dy
  let t = len2 ? ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2 : 0
  t = Math.max(0, Math.min(1, t))
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy))
}

/** Coalesce high-frequency pointer updates to at most one per animation frame. */function frameThrottle<T>(fn: (arg: T) => void): (arg: T) => void {
  let last!: T
  let has = false
  let queued = false
  return (arg: T) => {
    last = arg
    has = true
    if (queued) return
    queued = true
    requestAnimationFrame(() => {
      queued = false
      if (has) {
        has = false
        fn(last)
      }
    })
  }
}

export class Input {
  selectedIds = $state<string[]>([])
  selectedWireId = $state<string | null>(null)
  hoveredPin = $state<PinRef | null>(null)
  /** Where to put the pin tooltip (viewport coordinates, captured on hover). */
  hoverPos = $state<{ x: number; y: number } | null>(null)
  /** While a part is being dragged its DOM node stays put. */
  dragAnchor = $state<{ id: string; x: number; y: number } | null>(null)
  /** A right-drag selection box, in canvas-panel pixels. */
  marquee = $state<{ left: number; top: number; width: number; height: number } | null>(null)
  /** A part being dragged out of the palette (viewport-relative). */
  dragging = $state<{ def: PartDef; x: number; y: number } | null>(null)
  /** A wire being drawn: starts at a pin, ends wherever you next click. */
  wiring = $state<{ from: PinRef; waypoints: Point[] } | null>(null)
  cursor = $state<Point>({ x: 0, y: 0 })
  /** An existing wire endpoint being dragged onto another pin. */
  rerouting = $state<{ wireId: string; end: 'a' | 'b'; x: number; y: number } | null>(null)

  private wireSeq = 0
  private deps: InputDeps

  constructor(deps: InputDeps) {
    this.deps = deps
  }

  get selectedId(): string | null {
    return this.selectedIds.length === 1 ? this.selectedIds[0] : null
  }

  /** The part the properties strip describes. */
  get selectedPart(): Instance | null {
    return this.deps.project.instances.find((i) => i.id === this.selectedId) ?? null
  }

  /** The pin a wire being drawn would connect to right now (for the snap ring). */
  get wiringTarget(): PinRef | null {
    if (!this.wiring) return null
    return this.nearestPin(this.cursor.x, this.cursor.y, 26 / this.deps.view.zoom)
  }

  /** The live path of the wire being drawn, from its pin to wherever we are. */
  get wiringPoints(): Point[] {
    if (!this.wiring) return []
    const from = this.pinPoint(this.wiring.from)
    if (!from) return []
    const end = (this.wiringTarget && this.pinPoint(this.wiringTarget)) || this.cursor
    return livePoints(from, this.wiring.waypoints, end)
  }

  // ---- selection ---------------------------------------------------------

  selectOnly(id: string) {
    this.selectedIds = [id]
    this.selectedWireId = null
  }

  selectAll() {
    const parts = this.deps.project.instances
    if (!parts.length) return
    this.selectedIds = parts.map((i) => i.id)
    this.selectedWireId = null
  }

  selectWire(id: string | null) {
    this.selectedWireId = id
  }

  clearSelection() {
    this.selectedIds = []
    this.selectedWireId = null
  }

  cancelWire() {
    this.wiring = null
  }

  /** Where the (invisible) DOM node sits — frozen while it's being dragged. */
  domX(inst: Instance): number {
    return this.dragAnchor?.id === inst.id ? this.dragAnchor.x : inst.x
  }

  domY(inst: Instance): number {
    return this.dragAnchor?.id === inst.id ? this.dragAnchor.y : inst.y
  }

  // ---- geometry helpers --------------------------------------------------

  private toCanvas(clientX: number, clientY: number): Point {
    return this.deps.view.toCanvas(clientX, clientY, this.deps.canvasEl()!.getBoundingClientRect())
  }

  private overCanvas(clientX: number, clientY: number): boolean {
    return this.deps.view.overCanvas(clientX, clientY, this.deps.canvasEl()!.getBoundingClientRect())
  }

  private pinPoint(ref: PinRef): Point | null {
    return this.deps.surface.pinPoint(this.deps.project.instances, ref)
  }

  private nearestPin(cx: number, cy: number, radius: number, only?: string): PinRef | null {
    return this.deps.surface.nearestPin(this.deps.project.instances, cx, cy, radius, only)
  }

  private pinAtClient(clientX: number, clientY: number, radius = 26): PinRef | null {
    const p = this.toCanvas(clientX, clientY)
    return this.nearestPin(p.x, p.y, radius / this.deps.view.zoom)
  }

  /** Parts whose box overlaps a canvas-space rectangle. Touching counts. */
  private partsIn(left: number, top: number, right: number, bottom: number): string[] {
    const found: string[] = []
    for (const inst of this.deps.project.instances) {
      const size = this.deps.surface.partSizeOf(inst)
      const turned = inst.rotate === 90 || inst.rotate === 270
      const w = turned ? size.h : size.w
      const h = turned ? size.w : size.h
      if (inst.x <= right && inst.x + w >= left && inst.y <= bottom && inst.y + h >= top) {
        found.push(inst.id)
      }
    }
    return found
  }

  private reroutingPoint(w: Wire, end: 'a' | 'b'): Point | null {
    if (this.rerouting && this.rerouting.wireId === w.id && this.rerouting.end === end) {
      return { x: this.rerouting.x, y: this.rerouting.y }
    }
    return null
  }

  wirePoints(w: Wire): Point[] {
    const a = this.reroutingPoint(w, 'a') ?? this.pinPoint(w.a)
    const b = this.reroutingPoint(w, 'b') ?? this.pinPoint(w.b)
    if (!a || !b) return []
    return [a, ...w.waypoints, b]
  }

  // ---- marquee -----------------------------------------------------------

  startMarquee(e: PointerEvent) {
    e.preventDefault()
    this.deps.closeMenus()
    const host = this.deps.canvasEl()?.getBoundingClientRect()
    if (!host) return
    const originX = e.clientX
    const originY = e.clientY
    let moved = false
    const box = (x: number, y: number) => ({
      left: Math.min(originX, x) - host.left,
      top: Math.min(originY, y) - host.top,
      width: Math.abs(x - originX),
      height: Math.abs(y - originY),
    })
    this.marquee = box(originX, originY)

    const onMove = (ev: PointerEvent) => {
      if (Math.abs(ev.clientX - originX) + Math.abs(ev.clientY - originY) > 3) moved = true
      this.marquee = box(ev.clientX, ev.clientY)
    }
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      this.marquee = null
      if (!moved) return
      const a = this.toCanvas(originX, originY)
      const b = this.toCanvas(ev.clientX, ev.clientY)
      this.selectedIds = this.partsIn(
        Math.min(a.x, b.x),
        Math.min(a.y, b.y),
        Math.max(a.x, b.x),
        Math.max(a.y, b.y),
      )
    }
    // Cancelled, so there is no box to apply — just take the visual away.
    const onCancel = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      this.marquee = null
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
  }

  // ---- wiring ------------------------------------------------------------

  handlePinPress(e: PointerEvent, ref: PinRef) {
    if (e.button !== 0) return
    if (this.deps.sim.running) return
    e.stopPropagation()
    e.preventDefault()
    if (this.wiring) {
      this.finishWire(ref)
    } else {
      const start = this.toCanvas(e.clientX, e.clientY)
      this.cursor = { x: snapToGrid(start.x), y: snapToGrid(start.y) }
      this.wiring = { from: ref, waypoints: [] }
    }
  }

  finishWire(target: PinRef) {
    if (!this.wiring) return
    const { from, waypoints } = this.wiring
    this.wiring = null
    if (samePin(from, target)) return
    // Clicking the same two pins again takes the wire away, the way Wokwi does.
    this.deps.project.toggleWire(from, target, () => ({
      id: `w${++this.wireSeq}`,
      a: from,
      b: target,
      waypoints,
    }))
    this.selectedWireId = null
  }

  /** Commit the bend of the current leg, so the next leg starts from the corner. */
  addBend() {
    if (!this.wiring) return
    const from = this.pinPoint(this.wiring.from)
    const last = this.wiring.waypoints.length
      ? this.wiring.waypoints[this.wiring.waypoints.length - 1]
      : from
    if (!last) return
    const bend = legBend(this.wiring.waypoints.length, last, this.cursor)
    this.wiring = {
      ...this.wiring,
      waypoints: [...this.wiring.waypoints, bend ?? this.cursor],
    }
  }

  // ---- wire gestures -----------------------------------------------------

  startWireDrag(e: PointerEvent, wireId: string) {
    if (this.deps.sim.running) return
    e.stopPropagation()
    e.preventDefault()
    this.selectedWireId = wireId
    this.selectedIds = []
    if (e.button !== 0) return
    const w = this.deps.project.wires.find((x) => x.id === wireId)
    if (!w) return
    const a = this.pinPoint(w.a)
    const b = this.pinPoint(w.b)
    if (!a || !b) return
    const line = expandPath([a, ...w.waypoints, b])
    if (line.length < 2) return
    const start = this.toCanvas(e.clientX, e.clientY)
    let seg = -1
    let bestDist = Infinity
    for (let i = 0; i < line.length - 1; i++) {
      const d = distToSegment(start, line[i], line[i + 1])
      if (d < bestDist) {
        bestDist = d
        seg = i
      }
    }
    if (seg < 0 || bestDist > 24 / this.deps.view.zoom) return
    const vertical = line[seg].x === line[seg + 1].x
    let started = false

    const applyMove = frameThrottle((ev: PointerEvent) => {
      const cur = this.toCanvas(ev.clientX, ev.clientY)
      const delta = vertical ? cur.x - start.x : cur.y - start.y
      if (Math.abs(delta) < 3) return
      if (!started) {
        this.deps.project.commit()
        w.waypoints = line.slice(1, -1).map((p) => ({ ...p }))
        started = true
      }
      for (const j of [seg, seg + 1]) {
        if (j <= 0 || j >= line.length - 1) continue
        const wp = w.waypoints[j - 1]
        if (!wp) continue
        if (vertical) wp.x = snapToGrid(line[j].x + delta)
        else wp.y = snapToGrid(line[j].y + delta)
      }
    })
    const onMove = (ev: PointerEvent) => applyMove(ev)
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }

  startEndpointDrag(e: PointerEvent, wireId: string, end: 'a' | 'b') {
    if (e.button !== 0) return
    if (this.deps.sim.running) return
    e.stopPropagation()
    e.preventDefault()
    const w = this.deps.project.wires.find((x) => x.id === wireId)
    if (!w) return
    this.selectedWireId = wireId
    const p = this.toCanvas(e.clientX, e.clientY)
    this.rerouting = { wireId, end, x: p.x, y: p.y }

    const applyMove = frameThrottle((ev: PointerEvent) => {
      const q = this.toCanvas(ev.clientX, ev.clientY)
      this.rerouting = { wireId, end, x: q.x, y: q.y }
    })
    const onMove = (ev: PointerEvent) => applyMove(ev)
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      const target = this.pinAtClient(ev.clientX, ev.clientY)
      if (target) {
        const other = end === 'a' ? w.b : w.a
        if (!samePin(target, other)) {
          this.deps.project.commit()
          if (end === 'a') w.a = target
          else w.b = target
          this.deps.project.dedupeWires()
        }
      }
      this.rerouting = null
    }
    // A cancelled pointer is not a drop: drop the preview without re-attaching.
    const onCancel = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      this.rerouting = null
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
  }

  // ---- palette -----------------------------------------------------------

  startPaletteDrag(e: PointerEvent, def: PartDef) {
    if (e.button !== 0) return
    if (this.deps.sim.running) return
    e.preventDefault()
    this.dragging = { def, x: e.clientX, y: e.clientY }

    const onMove = (ev: PointerEvent) => {
      this.dragging = { def, x: ev.clientX, y: ev.clientY }
    }
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      if (this.overCanvas(ev.clientX, ev.clientY)) {
        const p = this.toCanvas(ev.clientX, ev.clientY)
        this.addInstance(def.kind, p.x - 16, p.y - 16)
      }
      this.dragging = null
    }
    // Cancelled, so nothing is dropped — only the drag preview goes away.
    const onCancel = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      this.dragging = null
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
  }

  // ---- parts -------------------------------------------------------------

  addInstance(
    kind: PartKind,
    x: number,
    y: number,
    copy?: { attrs?: Record<string, string>; rotate?: number },
    snap = true,
  ): Instance | undefined {
    const inst = this.deps.project.add(kind, x, y, copy, snap)
    if (inst) this.selectedIds = [inst.id]
    return inst
  }

  /** A press near a pin is *armed*: move and it becomes a drag; release says wire. */
  armPinPress(e: PointerEvent, ref: PinRef) {
    e.stopPropagation()
    e.preventDefault()
    if (!this.selectedIds.includes(ref.instanceId)) this.selectedIds = [ref.instanceId]
    this.selectedWireId = null
    const startX = e.clientX
    const startY = e.clientY
    let armed = true
    const cleanup = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
    }
    const onMove = (ev: PointerEvent) => {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) <= 4) return
      armed = false
      cleanup()
      // It was a drag after all — hand over from where the pointer is now.
      const inst = this.deps.project.instances.find((i) => i.id === ref.instanceId)
      if (inst) this.beginPartDrag(ev, inst)
    }
    const onUp = () => {
      cleanup()
      if (armed) this.handlePinPress(e, ref)
    }
    // A cancelled pointer is not a click: committing the armed press here would
    // start or finish a wire from a gesture the user never completed.
    const onCancel = () => {
      armed = false
      cleanup()
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
  }

  startMove(e: PointerEvent, inst: Instance) {
    if (e.button !== 0) return
    const p = this.toCanvas(e.clientX, e.clientY)
    const pin = this.nearestPin(p.x, p.y, 18 / this.deps.view.zoom, inst.id)
    if (pin) {
      this.armPinPress(e, pin)
      return
    }
    if (this.wiring) {
      // A press on a part's *body* is a corner, not a drag, while wiring.
      e.stopPropagation()
      e.preventDefault()
      this.addBend()
      return
    }
    e.stopPropagation()
    e.preventDefault()
    this.beginPartDrag(e, inst)
  }

  beginPartDrag(e: PointerEvent, inst: Instance) {
    const p = this.toCanvas(e.clientX, e.clientY)
    if (!this.selectedIds.includes(inst.id)) this.selectedIds = [inst.id]
    this.selectedWireId = null

    // A pushbutton is momentary: pressing it holds it down while the pointer is
    // on it. (If the press turns into a drag, it's cancelled below.)
    if (inst.kind === 'pushbutton') this.deps.sim.press(inst.id, true)
    if (this.deps.sim.running) {
      const release = () => {
        window.removeEventListener('pointerup', release)
        window.removeEventListener('pointercancel', release)
        if (inst.kind === 'pushbutton') this.deps.sim.press(inst.id, false)
      }
      window.addEventListener('pointerup', release)
      window.addEventListener('pointercancel', release)
      return
    }

    const offsetX = p.x - inst.x
    const offsetY = p.y - inst.y
    const startX = p.x
    const startY = p.y
    let movedOnce = false
    let dragged = false
    let group: { inst: Instance; x: number; y: number }[] = []
    let anchorX = inst.x
    let anchorY = inst.y
    const applyMove = frameThrottle((ev: PointerEvent) => {
      const q = this.toCanvas(ev.clientX, ev.clientY)
      if (!dragged && Math.hypot(q.x - startX, q.y - startY) > 4) {
        dragged = true
        // Dragging isn't pressing — release a button we pressed on the way in.
        if (inst.kind === 'pushbutton') this.deps.sim.press(inst.id, false)
      }
      if (!dragged) return
      if (!movedOnce) {
        this.deps.project.commit()
        movedOnce = true
        // Freeze the invisible DOM at where the drag began; it catches up on release.
        this.dragAnchor = { id: inst.id, x: inst.x, y: inst.y }
        anchorX = inst.x
        anchorY = inst.y
        group = this.selectedIds
          .map((id) => this.deps.project.instances.find((i) => i.id === id))
          .filter((i): i is Instance => !!i)
          .map((i) => ({ inst: i, x: i.x, y: i.y }))
      }
      // Holes first, from the raw pointer position; the free grid only if no hole
      // is in reach. (See CanvasSurface.place — this order is the bug fix.)
      const placed = this.deps.surface.place(
        inst.kind,
        this.deps.surface.pinOffsets(inst),
        q.x - offsetX,
        q.y - offsetY,
        this.deps.project.instances,
      )
      const dx = placed.x - anchorX
      const dy = placed.y - anchorY
      for (const part of group) {
        part.inst.x = part.x + dx
        part.inst.y = part.y + dy
      }
    })
    const onMove = (ev: PointerEvent) => applyMove(ev)
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
      this.dragAnchor = null
      if (inst.kind === 'pushbutton') this.deps.sim.press(inst.id, false)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    // Letting go off-window still has to release the button.
    window.addEventListener('pointercancel', onUp)
  }

  // ---- hover -------------------------------------------------------------

  onCanvasHover(e: PointerEvent) {
    if (!this.deps.canvasEl()) return
    const p = this.toCanvas(e.clientX, e.clientY)
    // Only the part under the pointer — otherwise a board's holes win and the
    // tooltip describes the board rather than the part you're on.
    const under = ((e.target as Element | null)?.closest('.instance') as HTMLElement | null)?.dataset.id
    const pin = this.nearestPin(p.x, p.y, 10 / this.deps.view.zoom, under)
    const id = pin ? `${pin.instanceId}:${pin.pin}` : null
    const cur = this.hoveredPin ? `${this.hoveredPin.instanceId}:${this.hoveredPin.pin}` : null
    if (id !== cur) {
      this.hoveredPin = pin
      // Only on change, so a drag doesn't churn the tooltip every pixel.
      this.hoverPos = pin ? { x: e.clientX, y: e.clientY } : null
    }
  }

  // ---- canvas press / pan -------------------------------------------------

  onCanvasPointerDown(e: PointerEvent) {
    const el = e.target as HTMLElement | null
    if (el?.closest('.instance') || el?.closest('.wire__hit')) return
    if (e.button === 2) {
      this.startMarquee(e)
      return
    }
    if (this.wiring) {
      this.addBend()
      return
    }
    // Drag empty space to pan; a plain click clears the selection.
    if (e.button !== 0 && e.button !== 1) return
    e.preventDefault()
    const startX = e.clientX
    const startY = e.clientY
    const startPanX = this.deps.view.panX
    const startPanY = this.deps.view.panY
    let moved = false
    const applyPan = frameThrottle((ev: PointerEvent) => {
      this.deps.view.panTo(startPanX + (ev.clientX - startX), startPanY + (ev.clientY - startY))
    })
    const onMove = (ev: PointerEvent) => {
      if (Math.abs(ev.clientX - startX) + Math.abs(ev.clientY - startY) > 4) moved = true
      applyPan(ev)
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
      if (!moved) this.clearSelection()
    }
    // A cancelled drag must not count as a click on empty space.
    const onCancel = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onCancel)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onCancel)
  }

  /** A fresh wire id. Kept here because wiring is what creates wires. */
  nextWireId(): string {
    return `w${++this.wireSeq}`
  }

  /** Follow the pointer while a wire is being drawn. Returns the cleanup. */
  trackCursor(): () => void {
    const applyMove = frameThrottle((ev: PointerEvent) => {
      const p = this.toCanvas(ev.clientX, ev.clientY)
      this.cursor = { x: snapToGrid(p.x), y: snapToGrid(p.y) }
    })
    const onMove = (ev: PointerEvent) => {
      if (this.wiring) applyMove(ev)
    }
    window.addEventListener('pointermove', onMove)
    return () => window.removeEventListener('pointermove', onMove)
  }

  // ---- commands that act on the selection --------------------------------

  deleteSelection() {
    if (this.selectedWireId) {
      this.deps.project.deleteSelection([], this.selectedWireId)
      this.selectedWireId = null
    } else if (this.selectedIds.length) {
      this.deps.project.deleteSelection(this.selectedIds, null)
      this.selectedIds = []
    }
  }

  copySelection() {
    this.deps.project.copySelection(this.selectedIds)
  }

  cutSelection() {
    this.deps.project.cutSelection(this.selectedIds, this.selectedWireId)
    this.clearSelection()
  }

  paste() {
    const pasted = this.deps.project.paste()
    if (pasted.length) this.selectedIds = pasted
  }

  duplicateSelection() {
    const made = this.deps.project.duplicateSelection(this.selectedIds)
    if (made.length) this.selectedIds = made
  }

  rotatePart(quarterTurns = 1) {
    this.deps.project.rotate(this.selectedIds, quarterTurns)
  }

  nudge(dx: number, dy: number) {
    this.deps.project.nudge(this.selectedIds, dx, dy)
  }

  setWireColour(name: string) {
    this.deps.project.setWireColour(this.selectedWireId, name)
  }
}
