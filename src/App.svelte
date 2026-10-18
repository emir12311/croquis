<script lang="ts">
  import {
    PARTS,
    partByKind,
    isBreadboard,
    type PartDef,
    type PartKind,
    formatResistance,
    parseResistance,
    type PartField,
  } from './lib/parts'
  import { evaluateCircuit, type LedState } from './lib/netlist'
  import { snapToHoles } from './lib/snap'
  import { BREADBOARD_HOLE_R, BREADBOARD_PITCH } from './lib/breadboard'
  import { shadowSvgMarkup, svgMarkupToImage } from './lib/part-art'
  import { PRESETS, type Preset } from './lib/presets'
  import { buildDiagram as buildDiagramFile, parseDiagram } from './lib/diagram'
  import { decodeRoute, encodeRoute, expandPath, legBend, livePoints, routePath } from './lib/route'
  import CodeEditor from './lib/CodeEditor.svelte'
  import {
    clear,
    drawDisc,
    drawGrid,
    drawPolyline,
    drawSquare,
    makeTransform,
    readPalette,
    type Palette,
    type PartSprite,
  } from './lib/circuit-render'
  import { untrack } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { documentDir } from '@tauri-apps/api/path'
  import { listen } from '@tauri-apps/api/event'
  import {
    open as openDialog,
    save as saveDialog,
    message as messageDialog,
  } from '@tauri-apps/plugin-dialog'

  type Theme = 'dark' | 'light'
  let theme = $state<Theme>('dark')

  $effect(() => {
    document.documentElement.dataset.theme = theme
  })

  const DEFAULT_SKETCH = `void setup() {
  // the built-in LED is on pin 13
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(9600);
  Serial.println("croquis ready");
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  delay(500);
  digitalWrite(LED_BUILTIN, LOW);
  delay(500);
  Serial.println("blink");
}
`
  let sketch = $state(DEFAULT_SKETCH)

  // ---- Layout: resizable columns, panels that collapse to a rail ----
  const MIN_PANEL_W = 200
  const MAX_PANEL_W = 560

  let leftWidth = $state(320)
  let rightWidth = $state(240)
  let leftCollapsed = $state(false)
  let rightCollapsed = $state(false)

  function startResize(e: PointerEvent, side: 'left' | 'right') {
    if (e.button !== 0) return
    e.preventDefault()
    const startX = e.clientX
    const startW = side === 'left' ? leftWidth : rightWidth
    const onMove = (ev: PointerEvent) => {
      const delta = ev.clientX - startX
      const next = side === 'left' ? startW + delta : startW - delta
      const clamped = Math.min(MAX_PANEL_W, Math.max(MIN_PANEL_W, next))
      if (side === 'left') leftWidth = clamped
      else rightWidth = clamped
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  const ICON_SKETCH = '<path d="M9 7l-5 5 5 5M15 7l5 5-5 5"/>'
  const ICON_COMPONENTS =
    '<rect x="6" y="6" width="12" height="12" rx="1"/><path d="M9 3.5v2M15 3.5v2M9 18.5v2M15 18.5v2M3.5 9h2M3.5 15h2M18.5 9h2M18.5 15h2"/>'
  const CHEVRON_LEFT = '<path d="M13.5 5.5l-6.5 6.5 6.5 6.5"/>'
  const CHEVRON_RIGHT = '<path d="M10.5 5.5l6.5 6.5-6.5 6.5"/>'

  // ---- Circuit canvas ----
  type Point = { x: number; y: number }
  type PinGeom = { name: string; x: number; y: number; description?: string }
  interface PinRef {
    instanceId: string
    pin: string
  }
  interface Wire {
    id: string
    a: PinRef
    b: PinRef
    waypoints: Point[]
    /** Named colour (Wokwi's red/blue/green/…); unset uses the themed wire colour. */
    colour?: string
  }

  // Wokwi's named wire colours; the default (no colour) uses the themed token.
  const WIRE_COLORS: { name: string; hex: string }[] = [
    { name: 'green', hex: '#3fa34d' },
    { name: 'red', hex: '#d64545' },
    { name: 'blue', hex: '#3b7dd8' },
    { name: 'yellow', hex: '#d9b23a' },
    { name: 'orange', hex: '#dd8a3c' },
    { name: 'purple', hex: '#9b6dd6' },
    { name: 'teal', hex: '#2f9e8f' },
    { name: 'pink', hex: '#d96a9c' },
  ]
  const WIRE_HEX = new Map(WIRE_COLORS.map((c) => [c.name, c.hex]))
  function wireHex(name?: string): string {
    return name ? (WIRE_HEX.get(name) ?? name) : 'var(--wire)'
  }
  interface Instance {
    id: string
    kind: PartKind
    x: number
    y: number
    /** Rotation in degrees (0/90/180/270), applied about the part's centre. */
    rotate?: number
    /** This part's own attributes, overriding the catalogue's defaults. */
    attrs?: Record<string, string>
  }

  let instances = $state<Instance[]>([])
  let wires = $state<Wire[]>([])
  let pinsById = $state<Record<string, PinGeom[]>>({})
  let sizesById = $state<Record<string, { w: number; h: number }>>({})

  let selectedIds = $state<string[]>([])
  /**
   * The selection as something the rest of the app can read.
   *
   * Most of it means "the part I am working on" — the properties strip, Edit code, the
   * context menu — and that only has an answer when exactly one thing is selected. So
   * this stays, derived, and everything that already asked for a single part keeps
   * working. Operations that should act on all of them read `selectedIds` directly.
   */
  const selectedId = $derived(selectedIds.length === 1 ? selectedIds[0] : null)
  let selectedWireId = $state<string | null>(null)
  let hoveredPin = $state<PinRef | null>(null)
  // Where to put the pin tooltip (viewport coordinates, captured on hover).
  let hoverPos = $state<{ x: number; y: number } | null>(null)

  // While a part is being dragged its DOM node stays put (see startMove).
  let dragAnchor = $state<{ id: string; x: number; y: number } | null>(null)

  /**
   * Breadboards are the surface parts sit on, so they always draw behind them —
   * otherwise whichever you placed last wins, and a board placed late swallows
   * the parts placed before it. Within each group it's placement order, as usual.
   */
  const renderOrder = $derived(
    [...instances].sort(
      (a, b) => (isBreadboard(a.kind) ? 0 : 1) - (isBreadboard(b.kind) ? 0 : 1),
    ),
  )

  /** The part the properties strip describes. */
  const selectedPart = $derived(instances.find((i) => i.id === selectedId) ?? null)

  /** Where the (invisible) DOM node sits — frozen while it's being dragged. */
  function domX(inst: Instance): number {
    return dragAnchor?.id === inst.id ? dragAnchor.x : inst.x
  }
  function domY(inst: Instance): number {
    return dragAnchor?.id === inst.id ? dragAnchor.y : inst.y
  }

  let seq = 0
  let wireSeq = 0

  // Save/load + simulation. Only meaningful inside the Tauri app; a plain
  // browser hides these.
  const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
  let running = $state(false)
  let serial = $state('')
  let buildError = $state('')
  let showSerial = $state(false)
  let pin13 = $state(false)
  /**
   * A right-drag selection box, in canvas-panel pixels. Left-drag already means move
   * (or pan), so the selection gesture needs its own; right-drag is otherwise just the
   * context menu, which is what the suppression in `openContextMenu` is for.
   */
  let marquee = $state<{ left: number; top: number; width: number; height: number } | null>(null)

  /** Parts whose box overlaps a canvas-space rectangle. Touching counts. */
  function partsIn(left: number, top: number, right: number, bottom: number): string[] {
    const found: string[] = []
    for (const inst of instances) {
      const size = partSizeOf(inst)
      // A quarter-turned part occupies its box the other way round.
      const turned = inst.rotate === 90 || inst.rotate === 270
      const w = turned ? size.h : size.w
      const h = turned ? size.w : size.h
      if (inst.x <= right && inst.x + w >= left && inst.y <= bottom && inst.y + h >= top) {
        found.push(inst.id)
      }
    }
    return found
  }

  function startMarquee(e: PointerEvent) {
    e.preventDefault()
    openMenu = null
    const host = canvasEl?.getBoundingClientRect()
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
    marquee = box(originX, originY)

    const onMove = (ev: PointerEvent) => {
      if (Math.abs(ev.clientX - originX) + Math.abs(ev.clientY - originY) > 3) moved = true
      marquee = box(ev.clientX, ev.clientY)
    }
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
      marquee = null
      if (!moved) {
        // A plain right-click is not a selection gesture; leave the selection alone.
        return
      }
      const a = toCanvas(originX, originY)
      const b = toCanvas(ev.clientX, ev.clientY)
      selectedIds = partsIn(Math.min(a.x, b.x), Math.min(a.y, b.y), Math.max(a.x, b.x), Math.max(a.y, b.y))
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }

  /**
   * Whether a pressed switch chatters the way a real contact does. Off by default:
   * a first-run button sketch should read the way a beginner expects. Turn it on in
   * the Run menu to watch a sketch that never debounced misread a single press.
   */
  let switchBounce = $state(false)
  /**
   * Where this circuit was last written, so Save does not interrogate you twice.
   * Session-only on purpose: it is about the file you are holding, not a preference.
   * Opening a file adopts its path too — save-from-there is what "the file I have
   * open" should mean. File → New clears it, or the next save would quietly
   * overwrite whatever you had open before.
   */
  let savePath = $state<string | null>(null)
  /**
   * A short-lived note that the circuit reached the disk. Save is silent by design now
   * that it no longer prompts, and silence with no acknowledgement reads as a keystroke
   * that did nothing at all.
   */
  /**
   * Whether anything here differs from what is on disk. `commit()` is the app's one
   * "something changed" hook for the circuit; the editor is watched separately, because
   * typing a sketch does not go through it.
   */
  let dirty = $state(false)
  /** The sketch as it was when this project was loaded or last saved. */
  let syncedSketch = $state(DEFAULT_SKETCH)

  $effect(() => {
    if (sketch !== syncedSketch) dirty = true
  })

  let savedNote = $state(false)
  let savedNoteTimer: ReturnType<typeof setTimeout> | null = null

  function noteSaved() {
    savedNote = true
    if (savedNoteTimer) clearTimeout(savedNoteTimer)
    savedNoteTimer = setTimeout(() => {
      savedNote = false
      savedNoteTimer = null
    }, 1400)
  }
  // Electrical state derived from the simulation (see lib/netlist.ts).
  let avrPins = $state<Record<string, boolean>>({})
  let ledStates = $state<Record<string, LedState>>({})
  /** What's wrong with a part — the strip says it in words, the canvas badges it. */
  let warnings = $state<Record<string, string>>({})
  let pressedButtons = $state<Record<string, boolean>>({})
  let renderError = $state('')
  interface PendingWire {
    a: PinRef
    b: PinRef
    route: string[]
    colour?: string
  }
  // Wires from a loaded file, decoded once their pins have rendered.
  let pendingWires = $state<PendingWire[] | null>(null)

  let canvasEl = $state<HTMLDivElement | null>(null)
  let renderCanvas = $state<HTMLCanvasElement | null>(null)

  // View transform: parts are stored in canvas coordinates, then the whole
  // content is scaled (zoom) and shifted (pan) for display.
  const ZOOM_MIN = 0.25
  const ZOOM_MAX = 4
  let zoom = $state(1)
  let panX = $state(0)
  let panY = $state(0)

  // A part being dragged out of the palette (viewport-relative).
  let dragging = $state<{ def: PartDef; x: number; y: number } | null>(null)
  // A wire being drawn: starts at a pin, ends wherever you next click.
  let wiring = $state<{ from: PinRef; waypoints: Point[] } | null>(null)
  let cursor = $state<Point>({ x: 0, y: 0 })
  // An existing wire endpoint being dragged onto another pin.
  let rerouting = $state<{ wireId: string; end: 'a' | 'b'; x: number; y: number } | null>(null)

  // Wires snap to a fine grid, so segments run along grid lines (like Wokwi)
  // instead of landing mid-square and making a straight run hard to line up.
  const WIRE_GRID = 12
  function snapToGrid(v: number) {
    return Math.round(v / WIRE_GRID) * WIRE_GRID
  }

  // Coalesce high-frequency pointer updates to at most one per animation frame.
  // A high-poll mouse fires far more moves than the screen can show, and each
  // one would otherwise trigger its own render.
  function frameThrottle<T>(fn: (arg: T) => void): (arg: T) => void {
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

  function toCanvas(clientX: number, clientY: number) {
    const r = canvasEl!.getBoundingClientRect()
    return { x: (clientX - r.left - panX) / zoom, y: (clientY - r.top - panY) / zoom }
  }

  function overCanvas(clientX: number, clientY: number) {
    const r = canvasEl!.getBoundingClientRect()
    return clientX >= r.left && clientX <= r.right && clientY >= r.top && clientY <= r.bottom
  }

  function addInstance(
    kind: PartKind,
    x: number,
    y: number,
    /** An instance being copied: its look and rotation come along. */
    copy?: { attrs?: Record<string, string>; rotate?: number },
    /**
     * Whether to look for a hole or grid line under it. On for dropping one part; off
     * for a paste, where each part is positioned relative to the rest of the copy and
     * snapping them one at a time would pull the group out of shape.
     */
    snap = true,
  ): Instance | undefined {
    if (running) return
    commit()
    let id: string
    do {
      id = `i${++seq}`
    } while (instances.some((i) => i.id === id))
    const rotate = copy?.rotate ?? 0
    const inst: Instance = {
      id,
      kind,
      x,
      y,
      ...(copy?.attrs ? { attrs: { ...copy.attrs } } : {}),
      ...(rotate ? { rotate } : {}),
    }
    instances.push(inst)
    // Land in a hole if there is one under it, otherwise on the free grid — except
    // for a rotated copy, whose hole offsets would need rotating about the
    // element's box, which we only know once it has mounted. It keeps the position
    // it was pasted at and re-seats on the first nudge.
    if (snap && !rotate) {
      const offsets = (kindPins.get(kind) ?? []).map((p) => ({ x: p.x, y: p.y }))
      const placed = placePart(kind, offsets, inst.x, inst.y)
      inst.x = placed.x
      inst.y = placed.y
    }
    selectedIds = [inst.id]
    return inst
  }

  // ---- Zoom / pan -------------------------------------------------------
  function zoomAt(clientX: number, clientY: number, factor: number) {
    if (!canvasEl) return
    const r = canvasEl.getBoundingClientRect()
    const mx = clientX - r.left
    const my = clientY - r.top
    const cx = (mx - panX) / zoom
    const cy = (my - panY) / zoom
    const nz = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom * factor))
    panX = mx - cx * nz
    panY = my - cy * nz
    zoom = nz
  }

  function zoomIn() {
    if (!canvasEl) return
    const r = canvasEl.getBoundingClientRect()
    zoomAt(r.left + r.width / 2, r.top + r.height / 2, 1.2)
  }

  function zoomOut() {
    if (!canvasEl) return
    const r = canvasEl.getBoundingClientRect()
    zoomAt(r.left + r.width / 2, r.top + r.height / 2, 1 / 1.2)
  }

  /** Put the world origin in the middle of the canvas rather than in its corner. */
  function centreView() {
    if (!canvasEl) return
    panX = Math.round(canvasEl.clientWidth / 2)
    panY = Math.round(canvasEl.clientHeight / 2)
  }

  /**
   * Centre it once, when the canvas first exists. Guarded because an effect that
   * re-centres on every change would fight the user's own panning.
   */
  let viewCentred = false
  $effect(() => {
    if (viewCentred || !canvasEl) return
    viewCentred = true
    centreView()
  })

  function resetZoom() {
    zoom = 1
    centreView()
  }

  // ---- Pins -------------------------------------------------------------
  // The wokwi elements expose `pinInfo` (name + position in their own rendered
  // pixel space). Read it straight off the mounted elements after each change.
  let pinsVersion = $state(0)
  let pinsRetries = 0
  $effect(() => {
    void pinsVersion
    const list = instances
    const next: Record<string, PinGeom[]> = {}
    const sizes: Record<string, { w: number; h: number }> = {}
    let needRetry = false
    for (const inst of list) {
      const el = canvasEl?.querySelector(`.instance[data-id="${inst.id}"]`) as HTMLElement | null
      const comp = el?.firstElementChild as (HTMLElement & { pinInfo?: PinGeom[] }) | null
      if (comp?.pinInfo) {
        next[inst.id] = comp.pinInfo.map((p) => ({
          name: p.name,
          x: p.x,
          y: p.y,
          description: p.description,
        }))
        const w = el?.offsetWidth ?? 0
        const h = el?.offsetHeight ?? 0
        sizes[inst.id] = { w, h }
        if (!w || !h) needRetry = true
      }
    }
    pinsById = next
    sizesById = sizes
    // Lit renders asynchronously — re-measure once the elements have laid out.
    if (needRetry && pinsRetries++ < 120) requestAnimationFrame(() => pinsVersion++)
  })

  // A pin's position in canvas space, accounting for the part's rotation
  // (which is applied about the part's centre).
  function pinCanvasPoint(inst: Instance, pin: PinGeom): Point {
    const rot = inst.rotate ?? 0
    if (!rot) return { x: inst.x + pin.x, y: inst.y + pin.y }
    // Size only — never a state-dependent lookup. See baseKey.
    const { w, h } = partSizeOf(inst)
    const rad = (rot * Math.PI) / 180
    const c = Math.cos(rad)
    const s = Math.sin(rad)
    const dx = pin.x - w / 2
    const dy = pin.y - h / 2
    return { x: inst.x + w / 2 + dx * c - dy * s, y: inst.y + h / 2 + dx * s + dy * c }
  }

  function pinPoint(ref: PinRef): Point | null {
    const inst = instances.find((i) => i.id === ref.instanceId)
    const pin = pinsById[ref.instanceId]?.find((p) => p.name === ref.pin)
    if (!inst || !pin) return null
    return pinCanvasPoint(inst, pin)
  }

  // The pin nearest a canvas-space point, within a forgiving radius.
  /**
   * The pin nearest a point, optionally restricted to one part. The restriction
   * matters: with a component sitting on a breadboard, the nearest pin to your
   * press is nearly always a hole *underneath*, so an unscoped search grabs the
   * board and you can't pick up the part you're pointing at.
   */
  function nearestPin(cx: number, cy: number, radius: number, only?: string): PinRef | null {
    let best: PinRef | null = null
    let bestDist = radius * radius
    for (const inst of instances) {
      if (only && inst.id !== only) continue
      for (const p of pinsById[inst.id] ?? []) {
        const pt = pinCanvasPoint(inst, p)
        const dx = cx - pt.x
        const dy = cy - pt.y
        const d = dx * dx + dy * dy
        if (d <= bestDist) {
          bestDist = d
          best = { instanceId: inst.id, pin: p.name }
        }
      }
    }
    return best
  }

  // Screen-space helper (radius given in screen px, so it feels the same at any zoom).
  function pinAtClient(clientX: number, clientY: number, radius = 26): PinRef | null {
    const p = toCanvas(clientX, clientY)
    return nearestPin(p.x, p.y, radius / zoom)
  }

  function samePin(a: PinRef, b: PinRef) {
    return a.instanceId === b.instanceId && a.pin === b.pin
  }


  function wirePoints(w: Wire): Point[] {
    const a = reroutingPoint(w, 'a') ?? pinPoint(w.a)
    const b = reroutingPoint(w, 'b') ?? pinPoint(w.b)
    if (!a || !b) return []
    return [a, ...w.waypoints, b]
  }

  // The pin a wire being drawn would connect to right now (for the snap ring).
  let wiringTarget = $derived.by(() => {
    if (!wiring) return null
    return nearestPin(cursor.x, cursor.y, 26 / zoom)
  })

  // The live path of the wire being drawn, from its pin to wherever we are.
  let wiringPoints = $derived.by(() => {
    if (!wiring) return [] as Point[]
    const from = pinPoint(wiring.from)
    if (!from) return [] as Point[]
    const end = (wiringTarget && pinPoint(wiringTarget)) || cursor
    return livePoints(from, wiring.waypoints, end)
  })

  function reroutingPoint(w: Wire, end: 'a' | 'b'): Point | null {
    if (rerouting && rerouting.wireId === w.id && rerouting.end === end) {
      return { x: rerouting.x, y: rerouting.y }
    }
    return null
  }

  function dedupeWires() {
    const seen = new Set<string>()
    wires = wires.filter((w) => {
      const ka = `${w.a.instanceId}:${w.a.pin}`
      const kb = `${w.b.instanceId}:${w.b.pin}`
      const key = ka < kb ? `${ka}|${kb}` : `${kb}|${ka}`
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
  }

  // ---- Wiring: click a pin, click the destination (or empty space to bend) --
  function handlePinPress(e: PointerEvent, ref: PinRef) {
    if (e.button !== 0) return
    // While a sketch runs the circuit is frozen — no wiring either.
    if (running) return
    e.stopPropagation()
    e.preventDefault()
    if (wiring) {
      finishWire(ref)
    } else {
      const start = toCanvas(e.clientX, e.clientY)
      cursor = { x: snapToGrid(start.x), y: snapToGrid(start.y) }
      wiring = { from: ref, waypoints: [] }
      // The part stays selected (see armPinPress) — the strip showing what you're
      // wiring to is more useful than an empty one.
    }
  }

  function finishWire(target: PinRef) {
    if (!wiring) return
    const { from, waypoints } = wiring
    wiring = null
    if (samePin(from, target)) return
    const existing = wires.find(
      (w) => (samePin(w.a, from) && samePin(w.b, target)) || (samePin(w.a, target) && samePin(w.b, from)),
    )
    // Clicking the same two pins again takes the wire away, the way Wokwi does.
    // It used to find the duplicate and then do nothing, which reads as the app
    // ignoring the click.
    if (existing) {
      commit()
      wires = wires.filter((w) => w.id !== existing.id)
      selectedWireId = null
      return
    }
    commit()
    wires.push({ id: `w${++wireSeq}`, a: from, b: target, waypoints })
  }

  // Distance from a point to a line segment.
  function distToSegment(p: Point, a: Point, b: Point): number {
    const dx = b.x - a.x
    const dy = b.y - a.y
    const len2 = dx * dx + dy * dy
    let t = len2 ? ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2 : 0
    t = Math.max(0, Math.min(1, t))
    return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy))
  }

  // Clicking a wire selects it; dragging from its middle reshapes it. The route
  // is materialised into explicit waypoints and the grabbed segment slides along
  // its perpendicular — endpoints that are pins stay put.
  function startWireDrag(e: PointerEvent, wireId: string) {
    if (running) return
    e.stopPropagation()
    e.preventDefault()
    selectedWireId = wireId
    selectedIds = []
    if (e.button !== 0) return
    const w = wires.find((x) => x.id === wireId)
    if (!w) return
    const a = pinPoint(w.a)
    const b = pinPoint(w.b)
    if (!a || !b) return
    const line = expandPath([a, ...w.waypoints, b])
    if (line.length < 2) return
    const start = toCanvas(e.clientX, e.clientY)
    let seg = -1
    let bestDist = Infinity
    for (let i = 0; i < line.length - 1; i++) {
      const d = distToSegment(start, line[i], line[i + 1])
      if (d < bestDist) {
        bestDist = d
        seg = i
      }
    }
    if (seg < 0 || bestDist > 24 / zoom) return
    const vertical = line[seg].x === line[seg + 1].x
    let started = false

    const applyMove = frameThrottle((ev: PointerEvent) => {
      const cur = toCanvas(ev.clientX, ev.clientY)
      const delta = vertical ? cur.x - start.x : cur.y - start.y
      if (Math.abs(delta) < 3) return
      if (!started) {
        commit()
        w.waypoints = line.slice(1, -1).map((p) => ({ ...p }))
        started = true
      }
      for (const j of [seg, seg + 1]) {
        if (j <= 0 || j >= line.length - 1) continue // a pin endpoint — fixed
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
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  function startEndpointDrag(e: PointerEvent, wireId: string, end: 'a' | 'b') {
    if (e.button !== 0) return
    if (running) return
    e.stopPropagation()
    e.preventDefault()
    const w = wires.find((x) => x.id === wireId)
    if (!w) return
    selectedWireId = wireId
    const p = toCanvas(e.clientX, e.clientY)
    rerouting = { wireId, end, x: p.x, y: p.y }

    const applyMove = frameThrottle((ev: PointerEvent) => {
      const q = toCanvas(ev.clientX, ev.clientY)
      rerouting = { wireId, end, x: q.x, y: q.y }
    })
    const onMove = (ev: PointerEvent) => applyMove(ev)
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      const target = pinAtClient(ev.clientX, ev.clientY)
      if (target) {
        const other = end === 'a' ? w.b : w.a
        if (!samePin(target, other)) {
          commit()
          if (end === 'a') w.a = target
          else w.b = target
          dedupeWires()
        }
      }
      rerouting = null
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  // ---- Part dragging ----------------------------------------------------
  function startPaletteDrag(e: PointerEvent, def: PartDef) {
    if (e.button !== 0) return
    if (running) return
    e.preventDefault()
    dragging = { def, x: e.clientX, y: e.clientY }

    const onMove = (ev: PointerEvent) => {
      dragging = { def, x: ev.clientX, y: ev.clientY }
    }
    const onUp = (ev: PointerEvent) => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      if (overCanvas(ev.clientX, ev.clientY)) {
        const p = toCanvas(ev.clientX, ev.clientY)
        addInstance(def.kind, p.x - 16, p.y - 16)
      }
      dragging = null
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  /**
   * A press near a pin is *armed*, not committed. Move and it becomes a drag of
   * the part; let go without moving and it's a pin press (start/finish a wire).
   *
   * This is what makes a breadboard grabbable: its holes are everywhere, so any
   * press at all is "near a pin", and committing on press left no way to move it
   * except by its edges.
   */
  function armPinPress(e: PointerEvent, ref: PinRef) {
    e.stopPropagation()
    e.preventDefault()
    // Pressing a pin selects its part, so the properties strip follows what
    // you're working on — and so clicking a small part (whose body is mostly
    // within reach of its legs) doesn't leave you with no selection at all.
    // Pressing a pin of an already-selected part keeps the selection. Collapsing it here
    // made a multi-selection impossible to drag: most of a small part is within reach of
    // its own legs, so the grab went through this path and threw the rest away.
    if (!selectedIds.includes(ref.instanceId)) selectedIds = [ref.instanceId]
    selectedWireId = null
    const startX = e.clientX
    const startY = e.clientY
    let armed = true
    const cleanup = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
    }
    const onMove = (ev: PointerEvent) => {
      if (Math.hypot(ev.clientX - startX, ev.clientY - startY) <= 4) return
      armed = false
      cleanup()
      // It was a drag after all — hand over, starting from where the pointer is
      // now, so the part doesn't jump.
      const inst = instances.find((i) => i.id === ref.instanceId)
      if (inst) beginPartDrag(ev, inst)
    }
    const onUp = () => {
      cleanup()
      if (armed) handlePinPress(e, ref)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }

  function startMove(e: PointerEvent, inst: Instance) {
    if (e.button !== 0) return
    const p = toCanvas(e.clientX, e.clientY)
    const pin = nearestPin(p.x, p.y, 18 / zoom, inst.id)
    if (pin) {
      armPinPress(e, pin)
      return
    }
    if (wiring) {
      // A wire is in progress, so a press on a part's *body* is a corner, not a
      // drag. Routing around a component used to be impossible: every press on one
      // moved it instead. A press near one of its pins still finishes the wire —
      // that is the branch above.
      e.stopPropagation()
      e.preventDefault()
      addBend()
      return
    }
    e.stopPropagation()
    e.preventDefault()
    beginPartDrag(e, inst)
  }

  function beginPartDrag(e: PointerEvent, inst: Instance) {
    const p = toCanvas(e.clientX, e.clientY)
    // Pressing a part inside the selection drags all of it; pressing one outside makes
    // that the selection, which is what every editor does.
    if (!selectedIds.includes(inst.id)) selectedIds = [inst.id]
    selectedWireId = null

    // A pushbutton is momentary: pressing it holds it down while the pointer is
    // on it. (If the press turns into a drag, it's cancelled below.)
    if (inst.kind === 'pushbutton') pressedButtons[inst.id] = true
    if (running) {
      // While a sketch runs the circuit is frozen — parts don't move. A
      // pushbutton still presses, since that's how you drive the running sim.
      const release = () => {
        window.removeEventListener('pointerup', release)
        window.removeEventListener('pointercancel', release)
        if (inst.kind === 'pushbutton') pressedButtons[inst.id] = false
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
    /** The selection's positions when the drag began, so the group keeps its shape. */
    let group: { inst: Instance; x: number; y: number }[] = []
    let anchorX = inst.x
    let anchorY = inst.y
    const applyMove = frameThrottle((ev: PointerEvent) => {
      const q = toCanvas(ev.clientX, ev.clientY)
      if (!dragged && Math.hypot(q.x - startX, q.y - startY) > 4) {
        dragged = true
        // Dragging isn't pressing — release a button we pressed on the way in.
        if (inst.kind === 'pushbutton') pressedButtons[inst.id] = false
      }
      if (!dragged) return
      if (!movedOnce) {
        commit()
        movedOnce = true
        // From here the *canvas* shows the movement. The invisible DOM only
        // exists for hit-testing, and moving it makes WebKitGTK repaint the whole
        // circuit layer every frame — which is what made dragging a breadboard
        // crawl. Freeze it at where the drag began; it catches up on release.
        dragAnchor = { id: inst.id, x: inst.x, y: inst.y }
        // Everything selected moves by the anchor's delta. Only the anchor snaps — a
        // group dragged across a breadboard should keep its shape rather than have
        // each part independently jump to a hole.
        anchorX = inst.x
        anchorY = inst.y
        group = selectedIds
          .map((id) => instances.find((i) => i.id === id))
          .filter((i): i is Instance => !!i)
          .map((i) => ({ inst: i, x: i.x, y: i.y }))
      }
      // Holes first, from the raw pointer position; the free grid only if no hole
      // is in reach. (See placePart — this order is the bug fix.)
      const placed = placePart(inst.kind, pinOffsets(inst), q.x - offsetX, q.y - offsetY)
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
      dragAnchor = null
      if (inst.kind === 'pushbutton') pressedButtons[inst.id] = false
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    // Letting go off-window still has to release the button.
    window.addEventListener('pointercancel', onUp)
  }

  // Highlight the pin under the pointer (CSS :hover can't reach canvas pixels).
  function onCanvasHover(e: PointerEvent) {
    if (!canvasEl) return
    const p = toCanvas(e.clientX, e.clientY)
    // Only the part under the pointer — otherwise a board's holes win and the
    // tooltip (and the marker) describe the board rather than the part you're on.
    const under = ((e.target as Element | null)?.closest('.instance') as HTMLElement | null)
      ?.dataset.id
    const pin = nearestPin(p.x, p.y, 10 / zoom, under)
    const id = pin ? `${pin.instanceId}:${pin.pin}` : null
    const cur = hoveredPin ? `${hoveredPin.instanceId}:${hoveredPin.pin}` : null
    if (id !== cur) {
      hoveredPin = pin
      // Only on change, so a drag doesn't churn the tooltip every pixel.
      hoverPos = pin ? { x: e.clientX, y: e.clientY } : null
    }
  }

  /**
   * Commit the *bend* of the current leg, so the next leg starts from the corner
   * rather than the free end — that is what lets a wire fold back over itself
   * instead of locking the whole L. It is why clicking empty space mid-wire drops
   * a corner, and it works over a part too now.
   */
  function addBend() {
    if (!wiring) return
    const from = pinPoint(wiring.from)
    const last = wiring.waypoints.length ? wiring.waypoints[wiring.waypoints.length - 1] : from
    if (!last) return
    const bend = legBend(wiring.waypoints.length, last, cursor)
    wiring = { ...wiring, waypoints: [...wiring.waypoints, bend ?? cursor] }
  }

  function onCanvasPointerDown(e: PointerEvent) {
    const el = e.target as HTMLElement | null
    if (el?.closest('.instance') || el?.closest('.wire__hit')) return
    // Right-drag draws a selection box. It comes after the line above so that
    // right-clicking *on* a part still opens the context menu.
    if (e.button === 2) {
      startMarquee(e)
      return
    }
    if (wiring) {
      addBend()
      return
    }
    // Drag empty space to pan; a plain click clears the selection.
    if (e.button !== 0 && e.button !== 1) return
    e.preventDefault()
    const startX = e.clientX
    const startY = e.clientY
    const startPanX = panX
    const startPanY = panY
    let moved = false
    const applyPan = frameThrottle((ev: PointerEvent) => {
      panX = startPanX + (ev.clientX - startX)
      panY = startPanY + (ev.clientY - startY)
    })
    const onMove = (ev: PointerEvent) => {
      if (Math.abs(ev.clientX - startX) + Math.abs(ev.clientY - startY) > 4) moved = true
      applyPan(ev)
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      if (!moved) {
        selectedIds = []
        selectedWireId = null
      }
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
  }

  // Keep the drawn wire following the pointer, even between clicks.
  $effect(() => {
    const applyMove = frameThrottle((ev: PointerEvent) => {
      const p = toCanvas(ev.clientX, ev.clientY)
      cursor = { x: snapToGrid(p.x), y: snapToGrid(p.y) }
    })
    const onMove = (ev: PointerEvent) => {
      if (wiring) applyMove(ev)
    }
    window.addEventListener('pointermove', onMove)
    return () => window.removeEventListener('pointermove', onMove)
  })

  // Wheel zooms toward the pointer.
  $effect(() => {
    const el = canvasEl
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      zoomAt(e.clientX, e.clientY, e.deltaY < 0 ? 1.1 : 1 / 1.1)
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  })

  // Registered once. Reads of selections happen when a key is pressed, not
  // while this effect runs — so it isn't re-attached on change.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      if (e.ctrlKey || e.metaKey) {
        const k = e.key.toLowerCase()
        if (k === '=' || k === '+') zoomIn()
        else if (k === '-' || k === '_') zoomOut()
        else if (k === '0') resetZoom()
        else if (k === 'n' && inTauri) void newProject()
        else if (k === 's' && e.shiftKey && inTauri) void onSaveAs()
        else if (k === 's' && inTauri) void onSave()
        else if (k === 'o' && inTauri) void onOpen()
        else if (k === 'z') {
          if (running) return
          if (e.shiftKey) redo()
          else undo()
        } else if (k === 'y') {
          if (running) return
          redo()
        } else if (k === 'a') {
          selectAll()
        } else if (k === 'c') copySelection()
        else if (k === 'x') {
          // Circuit edits wait until the simulation is stopped.
          if (running) return
          cutSelection()
        } else if (k === 'v') {
          if (running) return
          paste()
        } else if (k === 'd') {
          if (running) return
          duplicateSelection()
        } else return
        e.preventDefault()
        return
      }
      if (!running && selectedIds.length && (e.key === 'r' || e.key === 'R')) {
        // Shift+R turns it right around; plain R steps 90°.
        rotatePart(e.shiftKey ? 2 : 1)
        e.preventDefault()
        return
      }
      // Arrow keys nudge the selection by one grid step. An undo step each, which the
      // old version did not have at all — nudging was the one place you could move
      // something and not be able to take it back.
      const nudging =
        e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown'
      if (!running && selectedIds.length && nudging) {
        commit()
        for (const id of selectedIds) {
          const inst = instances.find((i) => i.id === id)
          if (!inst) continue
          if (e.key === 'ArrowLeft') inst.x -= WIRE_GRID
          else if (e.key === 'ArrowRight') inst.x += WIRE_GRID
          else if (e.key === 'ArrowUp') inst.y -= WIRE_GRID
          else inst.y += WIRE_GRID
        }
        e.preventDefault()
        return
      }
      if (e.key === 'Escape') {
        wiring = null
        selectedIds = []
        selectedWireId = null
        return
      }
      if (!running && (e.key === 'Delete' || e.key === 'Backspace')) {
        if (selectedWireId || selectedIds.length) {
          deleteSelection()
          e.preventDefault()
        }
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  // ---- Save / load (diagram.json) ----------------------------------------


  function buildDiagram() {
    return buildDiagramFile({
      parts: instances.map((inst) => ({
        id: inst.id,
        kind: inst.kind,
        x: inst.x,
        y: inst.y,
        rotate: inst.rotate,
        attrs: attrsOf(inst),
      })),
      wires: wires.map((w) => ({ a: w.a, b: w.b, colour: w.colour, waypoints: w.waypoints })),
      sketch,
      // The one thing only the app can answer: where a pin actually sits.
      pinPoint,
    })
  }

  function loadDiagram(text: string) {
    // Parsing and validation live in the format module, where they can be tested. A
    // file it cannot parse throws before anything here has been touched.
    const doc = parseDiagram(text)
    instances = doc.parts
    wires = []
    history = []
    future = []
    clipboard = null
    selectedIds = []
    selectedWireId = null
    pendingWires = doc.wires
    if (doc.sketch !== null) sketch = doc.sketch
    syncedSketch = sketch
    dirty = false
  }

  const SAVE_THEN_NEW = 'Save and start new'
  const DISCARD_CHANGES = 'Discard changes'

  /**
   * Start a blank circuit.
   *
   * The circuit is the one thing here that cannot be regenerated, so this asks before
   * throwing it away — and asks with three answers, because "no" and "wait" are not the
   * same thing. Discarding is available; losing work to a mis-click is not.
   */
  async function newProject() {
    if (dirty) {
      // Nothing to ask without a native dialog.
      if (!inTauri) return resetToBlank()
      const answer = await messageDialog('This circuit has changes that have not been saved yet.', {
        title: 'Start a new circuit?',
        kind: 'warning',
        buttons: { yes: SAVE_THEN_NEW, no: DISCARD_CHANGES, cancel: 'Cancel' },
      })
      if (answer !== SAVE_THEN_NEW && answer !== DISCARD_CHANGES) return
      if (answer === SAVE_THEN_NEW) {
        await onSave()
        // Backed out of the file picker: the work is still unsaved, so keep it.
        if (dirty) return
      }
    }
    // Only once the decision is made — cancelling must not stop a running sketch.
    if (running) await stopSim()
    resetToBlank()
  }

  function resetToBlank() {
    instances = []
    wires = []
    pendingWires = null
    history = []
    future = []
    clipboard = null
    selectedIds = []
    selectedWireId = null
    sketch = DEFAULT_SKETCH
    syncedSketch = DEFAULT_SKETCH
    dirty = false
    // A new circuit has no home, so the first Save asks. Without this it would write
    // over whatever had been open before.
    savePath = null
    avrPins = {}
    pin13 = false
    serial = ''
    buildError = ''
  }

  /** Write the circuit to a path, and make that path this project's home. */
  async function writeTo(path: string) {
    const json = JSON.stringify(buildDiagram(), null, 2)
    await invoke('write_text_file', { path, contents: json })
    savePath = path
    // In step with the disk again, so File -> New stops asking about it.
    syncedSketch = sketch
    dirty = false
    noteSaved()
  }

  /**
   * Save writes straight to wherever this circuit lives. The picker is for *choosing* a
   * home, not for confirming one — asking again every time is what makes saving feel
   * like an export rather than the thing you do constantly. With no home yet, this is
   * a Save as.
   *
   * Deliberately an unconfirmed overwrite, the way every editor behaves, which is what
   * makes Ctrl+S worth pressing. Save as… is how you move or rename it.
   */
  async function onSave() {
    const path = savePath
    if (!path) return onSaveAs()
    try {
      await writeTo(path)
    } catch (err) {
      console.error('save failed', err)
    }
  }

  /**
   * Where a first Save should offer to put the file.
   *
   * Absolute, and outside the app. A bare "diagram.json" resolves against the working
   * directory — which for a dev build is `src-tauri/` — so the app used to offer to save
   * into its own source tree, and once overwrote a tracked file that way.
   */
  async function defaultSavePath(): Promise<string> {
    if (!inTauri) return 'diagram.json'
    try {
      const dir = await documentDir()
      return `${dir.replace(/\/+$/, '')}/diagram.json`
    } catch {
      return 'diagram.json'
    }
  }

  /** Always asks, even when there is a home — this is how the home moves. */
  async function onSaveAs() {
    try {
      const path = await saveDialog({
        defaultPath: savePath ?? (await defaultSavePath()),
        filters: [{ name: 'Wokwi diagram', extensions: ['json'] }],
      })
      if (!path) return
      await writeTo(path)
    } catch (err) {
      console.error('save failed', err)
    }
  }

  /**
   * Load a shipped circuit. It goes through the same loader as File → Open, because
   * a preset *is* a diagram.json — and like Open it replaces the whole circuit, so it
   * cannot happen under a live simulation.
   */
  async function loadPreset(preset: Preset) {
    if (running) await stopSim()
    loadDiagram(preset.text)
  }

  async function onOpen() {
    // Loading a diagram replaces the whole circuit, so it can't happen under a
    // live simulation — stop first rather than silently ignoring the click.
    if (running) await stopSim()
    try {
      const picked = await openDialog({
        multiple: false,
        filters: [{ name: 'Wokwi diagram', extensions: ['json'] }],
      })
      if (!picked || typeof picked !== 'string') return
      const text = await invoke<string>('read_text_file', { path: picked })
      loadDiagram(text)
      // Only after it loaded: a file that fails to parse must not become the path
      // the next Save writes over.
      savePath = picked
    } catch (err) {
      console.error('open failed', err)
    }
  }

  // Decode loaded wire routes once the parts (and their pins) have rendered.
  $effect(() => {
    if (!pendingWires) return
    const created: Wire[] = []
    for (const p of pendingWires) {
      const a = pinPoint(p.a)
      const b = pinPoint(p.b)
      if (!a || !b) return // pins not ready yet — wait for them
      created.push({
        id: `w${++wireSeq}`,
        a: p.a,
        b: p.b,
        waypoints: p.route.length ? decodeRoute(a, p.route) : [],
        colour: p.colour,
      })
    }
    wires = created
    pendingWires = null
    // The file is now fully on the canvas, so frame it. Deferred a frame because the
    // instances have only just been inserted and their sizes aren't measured yet.
    requestAnimationFrame(() => fitView())
  })

  // ---- Undo / redo -------------------------------------------------------
  interface Snapshot {
    instances: Instance[]
    wires: Wire[]
  }

  let history = $state<Snapshot[]>([])
  let future = $state<Snapshot[]>([])

  function snapshot(): Snapshot {
    return {
      instances: instances.map((i) => ({ ...i })),
      wires: wires.map((w) => ({
        ...w,
        a: { ...w.a },
        b: { ...w.b },
        waypoints: w.waypoints.map((p) => ({ ...p })),
      })),
    }
  }

  // Record the current state, so the mutation that follows can be undone.
  function commit() {
    history = [...history, snapshot()].slice(-200)
    future = []
    dirty = true
  }

  function applySnapshot(s: Snapshot) {
    instances = s.instances
    wires = s.wires
    selectedIds = []
    selectedWireId = null
    wiring = null
  }

  function undo() {
    if (!history.length) return
    future = [...future, snapshot()]
    const prev = history[history.length - 1]
    history = history.slice(0, -1)
    applySnapshot(prev)
  }

  function redo() {
    if (!future.length) return
    history = [...history, snapshot()]
    const next = future[future.length - 1]
    future = future.slice(0, -1)
    applySnapshot(next)
  }

  // ---- Clipboard ---------------------------------------------------------
  /** One part on the clipboard, offset from the top-left of the copy it came from. */
  interface ClipboardPart {
    kind: PartKind
    dx: number
    dy: number
    /** A copy carries how a part *looks*, not just what it is. */
    attrs?: Record<string, string>
    rotate?: number
  }

  let clipboard = $state<{ x: number; y: number; parts: ClipboardPart[] } | null>(null)

  function deleteSelection() {
    if (selectedWireId) {
      commit()
      wires = wires.filter((w) => w.id !== selectedWireId)
      selectedWireId = null
    } else if (selectedIds.length) {
      commit()
      // A wire hanging off a part that is going goes with it, or it is left dangling.
      const going = new Set(selectedIds)
      wires = wires.filter((w) => !going.has(w.a.instanceId) && !going.has(w.b.instanceId))
      instances = instances.filter((i) => !going.has(i.id))
      selectedIds = []
    }
  }

  /**
   * Everything on the canvas.
   *
   * Wires are left out deliberately: they are selected one at a time, so "all of them" is
   * not a thing the rest of the app could act on.
   */
  function selectAll() {
    if (!instances.length) return
    selectedIds = instances.map((i) => i.id)
    selectedWireId = null
  }

  function copySelection() {
    const parts = instances.filter((i) => selectedIds.includes(i.id))
    if (!parts.length) return
    // Stored relative to the top-left of the copy, so a group keeps its shape wherever
    // it is pasted. Rotation *and* attributes come along too: without them a copied LED
    // came back upright and red, and a copied resistor lost its value — the app
    // forgetting what you had just set.
    const x = Math.min(...parts.map((p) => p.x))
    const y = Math.min(...parts.map((p) => p.y))
    clipboard = {
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

  function cutSelection() {
    copySelection()
    deleteSelection()
  }

  function paste() {
    const clip = clipboard
    if (!clip) return
    // Each paste steps along, so pasting twice does not stack two copies in one spot.
    const x = clip.x + WIRE_GRID
    const y = clip.y + WIRE_GRID
    const pasted: string[] = []
    for (const part of clip.parts) {
      // The offsets are deliberately not re-snapped to holes: a group should land in the
      // shape it was copied, not have each part independently jump to the nearest one.
      const made = addInstance(part.kind, x + part.dx, y + part.dy, part, false)
      if (made) pasted.push(made.id)
    }
    // What you just pasted is what you are working on.
    if (pasted.length) selectedIds = pasted
    clipboard = { ...clip, x, y }
  }

  function duplicateSelection() {
    if (!selectedIds.length) return
    copySelection()
    paste()
  }

  // ---- Wires: colour, fit, clear -----------------------------------------
  // `quarterTurns` is 1 for the R shortcut / toolbar, 2 for "Rotate 180°".
  function rotatePart(quarterTurns = 1) {
    const turning = instances.filter((i) => selectedIds.includes(i.id))
    if (!turning.length) return
    // One snapshot for the whole turn, so undo puts them all back together rather than
    // stepping through them one at a time.
    commit()
    for (const inst of turning) rotateOne(inst, quarterTurns)
  }

  function rotateOne(inst: Instance, quarterTurns: number) {
    inst.rotate = ((inst.rotate ?? 0) + 90 * quarterTurns) % 360
    // Rotating moves the legs, so put the part back down where they still land in
    // holes. On a breadboard you don't spin a part on the spot — you pull it out
    // and push it back in. Without this, rotating a seated part left its legs
    // between holes, which is what a rotated pushbutton looked like.
    if (!isBreadboard(inst.kind)) {
      const seated = snapToHoles(
        { x: inst.x, y: inst.y },
        pinOffsets(inst),
        boardHolePoints(),
        SNAP,
      )
      // Off the board, or nothing in reach: leave it exactly where it was.
      if (seated) {
        inst.x = seated.x
        inst.y = seated.y
      }
    }
  }

  function setWireColour(name: string) {
    const w = selectedWireId ? wires.find((x) => x.id === selectedWireId) : null
    if (!w || w.colour === name) return
    commit()
    w.colour = name
  }

  function fitView() {
    if (!canvasEl || !instances.length) return
    const r = canvasEl.getBoundingClientRect()
    const pad = 48
    let minX = Infinity
    let minY = Infinity
    let maxX = -Infinity
    let maxY = -Infinity
    for (const inst of instances) {
      const el = canvasEl.querySelector(`.instance[data-id="${inst.id}"]`) as HTMLElement | null
      const w = el?.offsetWidth ?? 0
      const h = el?.offsetHeight ?? 0
      minX = Math.min(minX, inst.x)
      minY = Math.min(minY, inst.y)
      maxX = Math.max(maxX, inst.x + w)
      maxY = Math.max(maxY, inst.y + h)
    }
    const bw = Math.max(1, maxX - minX)
    const bh = Math.max(1, maxY - minY)
    const z = Math.min(
      ZOOM_MAX,
      Math.max(ZOOM_MIN, Math.min((r.width - pad * 2) / bw, (r.height - pad * 2) / bh)),
    )
    zoom = z
    panX = (r.width - bw * z) / 2 - minX * z
    panY = (r.height - bh * z) / 2 - minY * z
  }

  function clearCanvas() {
    if (running) return
    if (!instances.length && !wires.length) return
    commit()
    instances = []
    wires = []
    selectedIds = []
    selectedWireId = null
    wiring = null
  }

  // ---- Menu bar ----------------------------------------------------------
  // The home for commands that have none of their own: undo/redo, cut/copy/
  // paste, rotate, clear. Everything here also has a keyboard shortcut, and the
  // primary actions (Run, zoom, wiring, the palette) stay visible as buttons —
  // nothing *essential* lives only in here.
  type MenuId = 'file' | 'edit' | 'view' | 'run'
  let openMenu = $state<MenuId | null>(null)
  const hasSelection = $derived(selectedIds.length > 0 || !!selectedWireId)

  function toggleMenu(id: MenuId) {
    openMenu = openMenu === id ? null : id
  }

  // With a menu open, sliding across the titles switches menus (menu-bar feel).
  function hoverMenu(id: MenuId) {
    if (openMenu) openMenu = id
  }

  function menuPick(fn: () => void) {
    openMenu = null
    fn()
  }

  // Any press outside, Esc, or losing the window closes the open menu.
  $effect(() => {
    if (!openMenu) return
    const dismiss = () => {
      openMenu = null
    }
    const close = (e: Event) => {
      const t = e.target as Element | null
      if (t?.closest?.('.menu, .menubar')) return
      dismiss()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') dismiss()
    }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', dismiss)
    return () => {
      window.removeEventListener('pointerdown', close)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', dismiss)
    }
  })

  // What the pin under the pointer is. A single LED or the Uno is guessable; a
  // 4-leg button or a sensor isn't, so this is what makes them usable.
  function pinTooltip(ref: PinRef): { part: string; name: string; note: string } | null {
    const inst = instances.find((i) => i.id === ref.instanceId)
    if (!inst) return null
    return {
      part: partByKind(inst.kind).label,
      name: ref.pin,
      note: pinsById[ref.instanceId]?.find((p) => p.name === ref.pin)?.description ?? '',
    }
  }

  // ---- Right-click menu --------------------------------------------------
  // An in-app menu, not a native one: it wears the app's tokens, and every
  // command here also has a keyboard shortcut — nothing is menu-only.
  let menu = $state<{ x: number; y: number; partId?: string; wireId?: string } | null>(null)

  const MENU_W = 184
  const MENU_H = 260

  function openContextMenu(e: MouseEvent) {
    // A right-drag is selecting, not asking for a menu.
    if (marquee) {
      e.preventDefault()
      return
    }
    // Right-clicking a part outside the selection means that part — otherwise the menu
    // would act on parts you cannot see it acting on.
    const hit = (e.target as Element | null)?.closest('.instance') as HTMLElement | null
    const hitId = hit?.dataset.id
    if (hitId && !selectedIds.includes(hitId)) selectedIds = [hitId]
    const el = e.target as Element | null
    const partEl = el?.closest('.instance') as HTMLElement | null
    const wireEl = el?.closest('.wire__hit') as Element | null
    const partId = partEl?.dataset.id
    const wireId = wireEl?.getAttribute('data-wire') ?? undefined
    if (!partId && !wireId) {
      menu = null
      return
    }
    e.preventDefault()
    // The guard above already narrowed the selection to the clicked part when it was not
    // part of it. It must not be narrowed again here, or right-clicking any part of a
    // multi-selection would throw the rest away and the menu could never act on a group.
    // Right-clicking a *wire* is not about parts at all, so that does clear them.
    if (!partId) selectedIds = []
    selectedWireId = wireId ?? null
    menu = {
      x: Math.max(0, Math.min(e.clientX, window.innerWidth - MENU_W)),
      y: Math.max(0, Math.min(e.clientY, window.innerHeight - MENU_H)),
      partId,
      wireId,
    }
  }

  function runMenu(fn: () => void) {
    menu = null
    fn()
  }

  /**
   * Whether every selected part is a microcontroller.
   *
   * The context menu offers only what *all* of the selection can do, so "Edit code"
   * appears only when there is nothing in the selection it would not apply to.
   */
  function menuIsMcu(): boolean {
    if (!selectedIds.length) return false
    return selectedIds.every((id) => instances.some((i) => i.id === id && i.kind === 'uno'))
  }

  // "Edit code" — raise the Sketch panel and put the caret in it. When each
  // microcontroller gets its own sketch, this is also where the tab switches.
  function editSketch() {
    menu = null
    leftCollapsed = false
    // The editor may need to mount (or expand) first, hence the retries.
    let tries = 0
    const focus = () => {
      const content = document.querySelector('.cm-content') as HTMLElement | null
      if (content) {
        content.focus()
        content.scrollIntoView({ block: 'nearest' })
      } else if (++tries <= 10) {
        requestAnimationFrame(focus)
      }
    }
    requestAnimationFrame(focus)
  }

  // Any press outside, Esc, or losing the window closes the menu.
  $effect(() => {
    if (!menu) return
    const dismiss = () => {
      menu = null
    }
    const close = (e: Event) => {
      // Presses *inside* the menu must reach the item, not dismiss it.
      const t = e.target as Element | null
      if (t?.closest?.('.menu')) return
      dismiss()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') dismiss()
    }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', dismiss)
    return () => {
      window.removeEventListener('pointerdown', close)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', dismiss)
    }
  })

  // ---- Canvas renderer ---------------------------------------------------
  // The circuit is drawn into a <canvas> at device resolution (crisp); the DOM
  // stays invisible and handles hit-testing. See src/lib/circuit-render.ts.
  let palette = $state<Palette>(readPalette())
  const partImages = new Map<string, PartSprite>()
  const pendingParts = new Set<string>()
  let partImagesVersion = $state(0)

  function wireColour(name?: string): string {
    return name ? (WIRE_HEX.get(name) ?? name) : palette.wire
  }

  /** A part's attributes: the catalogue's defaults, with its own on top. */
  function attrsOf(inst: Instance): Record<string, string> {
    return { ...partByKind(inst.kind).attrs, ...inst.attrs }
  }

  /** Change one property of a part (undoable). */
  /**
   * The resistance field is the one property you *type* rather than pick, so it
   * keeps the text as it is being written and the unit it is being written in.
   * Ohms stay the stored truth; the unit is only how you read and write them.
   */
  let resText = $state('')
  let resUnit = $state(1)

  // Re-read the controls when a different part is selected. Keyed on the selection
  // alone — the value is read without tracking — so this cannot re-trigger itself
  // while the field is being typed in.
  $effect(() => {
    const id = selectedId
    if (!id) return
    const inst = instances.find((i) => i.id === id)
    if (!inst) return
    untrack(() => {
      const field = (partByKind(inst.kind).fields ?? []).find((f) => f.control.kind === 'resistance')
      if (!field || field.control.kind !== 'resistance') return
      const ohms = Number(attrsOf(inst)[field.name] ?? field.control.min)
      const tidy = formatResistance(ohms)
      resText = tidy.text
      resUnit = tidy.factor
    })
  })

  /** A resistance written in the unit currently chosen. */
  function inUnit(ohms: number): string {
    const written = ohms / resUnit
    // Below a thousandth of the chosen unit, rounding to three decimals would show a
    // zero that is not true. Show it in full instead — 2.2e-10 reads oddly but is
    // honest, and it types back correctly.
    if (Math.abs(written) < 0.001) return String(written)
    return String(Math.round(written * 1000) / 1000)
  }

  /** Switching unit is a change of view, not of value: 4700 Ω reads "4700" in Ω. */
  function pickUnit(factor: number) {
    if (!selectedPart) return
    resUnit = factor
    const ohms = Number(attrsOf(selectedPart).value ?? 0)
    if (ohms > 0) {
      resUnit = factor
      resText = String(Math.round((ohms / factor) * 1000) / 1000)
    }
  }

  function commitResistance(field: PartField, text: string) {
    if (!selectedPart) return
    const control = field.control
    if (control.kind !== 'resistance') return
    const ohms = parseResistance(text, resUnit, control.min, control.max)
    if (ohms === null) {
      // Not a number: put back what is actually stored, rather than guessing at it.
      resText = inUnit(Number(attrsOf(selectedPart)[field.name] ?? control.min))
      return
    }
    setAttr(selectedPart, field.name, String(ohms))
    // Show what was stored, in the unit being typed in, so a value that had to be
    // clamped is visible rather than silently different from what was typed.
    resText = inUnit(ohms)
  }

  function setAttr(inst: Instance, name: string, value: string) {
    if (attrsOf(inst)[name] === value) return
    commit()
    inst.attrs = { ...inst.attrs, [name]: value }
  }

  /**
   * The key a part's sprite is cached under when only its *drawing* matters —
   * what the simulation changes, not how big the part is.
   *
   * This split matters more than it looks. `pinCanvasPoint` runs inside the
   * netlist effect's dependency read, and a *rotated* part needed its sprite size
   * — which meant reading `partKey`, which reads `ledStates`. The effect then
   * assigns `ledStates`, so it re-triggered itself forever: a busy loop that
   * started the moment anything was rotated, never stopped, and got worse the
   * more there was on the canvas. Sizes are keyed without state, so the rotation
   * path can't reach any.
   */
  function baseKey(inst: Instance): string {
    return `${partByKind(inst.kind).tag}|${JSON.stringify(attrsOf(inst))}`
  }

  function partKey(inst: Instance): string {
    const chip = inst.kind === 'uno' && pin13 ? 1 : 0
    const lit = inst.kind === 'led' ? LED_ORDER[ledStates[inst.id] ?? 'off'] : 0
    const down = inst.kind === 'pushbutton' && pressedButtons[inst.id] ? 1 : 0
    return `${baseKey(inst)}|${chip}${lit}${down}`
  }

  /** Rasterised size per `baseKey` — a plain Map, so reading it tracks nothing. */
  const partSizes = new Map<string, { w: number; h: number }>()

  /** The box a part rotates about. Deliberately free of simulation state. */
  function partSizeOf(inst: Instance): { w: number; h: number } {
    const cached = partSizes.get(baseKey(inst))
    if (cached) return cached
    const dom = sizesById[inst.id]
    return { w: dom?.w ?? 0, h: dom?.h ?? 0 }
  }

  const partRetries = new Map<string, number>()

  // The palette shows each part's *real* drawing, rasterised exactly like the
  // canvas is (see lib/part-art.ts). The hand-drawn line icons are only a
  // fallback for the frame or two before the thumbnail is ready.
  let partsListEl = $state<HTMLElement | null>(null)
  let partArt = $state<Record<string, { href: string; width: number; height: number }>>({})
  const artPending = new Set<PartKind>()

  // Each kind's pin offsets, taken from the palette's hidden elements. That's how
  // a part can be dropped straight into a hole before it has ever been mounted.
  const kindPins = new Map<PartKind, PinGeom[]>()
  // A part within a hole's pitch of one snaps into it; anything further is free.
  const SNAP_REACH = BREADBOARD_PITCH * 1.1
  /** How far a part may move to find a placement, and what counts as seated. */
  const SNAP = { reach: SNAP_REACH, seat: BREADBOARD_HOLE_R }

  /** A part's pin offsets from its own origin, rotation included. */
  function pinOffsets(inst: Instance): Point[] {
    const pins = pinsById[inst.id]
    if (!pins?.length) return []
    return pins.map((p) => {
      const at = pinCanvasPoint(inst, p)
      return { x: at.x - inst.x, y: at.y - inst.y }
    })
  }

  /** Every breadboard hole, in canvas space. */
  function boardHolePoints(): Point[] {
    const holes: Point[] = []
    for (const board of instances) {
      if (!isBreadboard(board.kind)) continue
      for (const hole of pinsById[board.id] ?? []) holes.push(pinCanvasPoint(board, hole))
    }
    return holes
  }

  /**
   * Where a part should sit: in a hole if one is within reach, otherwise on the
   * free grid. The holes are tried **first**, from the raw pointer position.
   *
   * That order is the whole point. Quantising to the 12px free grid first moved
   * the part up to 6px away before anyone asked about holes, which is further
   * than the reach — so some columns simply could not be used, and which ones
   * failed moved whenever either grid changed.
   */
  function placePart(kind: PartKind, offsets: Point[], rawX: number, rawY: number) {
    const raw = { x: rawX, y: rawY }
    // A board never snaps into another board: it has hundreds of holes of its own.
    const onHoles = isBreadboard(kind)
      ? null
      : snapToHoles(raw, offsets, boardHolePoints(), SNAP)
    if (onHoles) return onHoles
    return { x: snapToGrid(raw.x), y: snapToGrid(raw.y) }
  }

  function buildPaletteArt(attempt = 0) {
    const tiles = Array.from(partsListEl?.querySelectorAll<HTMLElement>('.part[data-kind]') ?? [])
    let waitingForLayout = false
    for (const tile of tiles) {
      const kind = tile.dataset.kind as PartKind | undefined
      if (!kind || partArt[kind] || artPending.has(kind)) continue
      const host = tile.querySelector('.part__art > *:first-child') as
        | (HTMLElement & { pinInfo?: PinGeom[] })
        | null
      // Remember this kind's pin offsets, for snapping straight into a hole.
      if (host?.pinInfo?.length && !kindPins.has(kind)) {
        kindPins.set(
          kind,
          host.pinInfo.map((p) => ({ name: p.name, x: p.x, y: p.y })),
        )
      }
      let markup: string | null = null
      try {
        markup = host ? shadowSvgMarkup(host) : null
      } catch {
        markup = null
      }
      if (!markup) {
        // Not laid out yet — try again next frame.
        waitingForLayout = true
        continue
      }
      artPending.add(kind)
      svgMarkupToImage(markup)
        .then((art) => {
          artPending.delete(kind)
          // The object URL is deliberately kept alive: the <img> shows it.
          partArt = {
            ...partArt,
            [kind]: { href: art.href, width: art.width, height: art.height },
          }
        })
        .catch(() => artPending.delete(kind))
    }
    if (waitingForLayout && attempt < 30) {
      requestAnimationFrame(() => buildPaletteArt(attempt + 1))
    }
  }

  // Also re-run when the panel reopens, in case the tiles weren't mounted yet.
  $effect(() => {
    void rightCollapsed
    requestAnimationFrame(() => buildPaletteArt())
  })

  // Rasterise a part's element (its shadow-root contents) into an image we blit.
  // `waited` makes the Lit wait below one-shot: `updateComplete` is a *reused*
  // promise that still exists once settled, so re-reading it after awaiting it
  // would spin forever — freezing the app the first time a part is rasterised.
  function rasterizePart(inst: Instance, waited = false) {
    const key = partKey(inst)
    if (partImages.has(key) || pendingParts.has(key)) return
    const host = canvasEl?.querySelector(
      `.instance[data-id="${inst.id}"] > *:first-child`,
    ) as HTMLElement | null
    // Lit re-renders asynchronously — wait once for it before snapshotting, or
    // we capture (and cache) the *previous* drawing. This matters when the
    // drawing depends on a property we just changed: the LED's `value`, the
    // button's `pressed`, the Uno's `led13`.
    if (!waited) {
      const ready = (host as unknown as { updateComplete?: Promise<unknown> } | null)
        ?.updateComplete
      if (ready && typeof ready.then === 'function') {
        pendingParts.add(key)
        const again = () => {
          pendingParts.delete(key)
          rasterizePart(inst, true)
        }
        ready.then(again, again)
        return
      }
    }
    // The state can move on while we wait (a press that turns into a drag).
    // Never file a snapshot under a key it no longer matches.
    if (partKey(inst) !== key) return

    const w = host?.offsetWidth ?? 0
    const h = host?.offsetHeight ?? 0
    if (!host || !host.shadowRoot || !w || !h) {
      // Not laid out yet — retry on the next frame until it is.
      const tries = (partRetries.get(key) ?? 0) + 1
      if (tries <= 120) {
        partRetries.set(key, tries)
        requestAnimationFrame(() => (partImagesVersion += 1))
      }
      return
    }
    let markup: string | null
    try {
      markup = shadowSvgMarkup(host)
    } catch (err) {
      renderError = `${key}: ${String(err)}`
      return
    }
    if (!markup) return
    pendingParts.add(key)
    svgMarkupToImage(markup, { width: w, height: h })
      .then((art) => {
        // The image already holds the pixels; the URL only existed to load it.
        URL.revokeObjectURL(art.href)
        partImages.set(key, { image: art.image, w: art.width, h: art.height })
        // The size, without the simulation state the key above carries.
        partSizes.set(baseKey(inst), { w: art.width, h: art.height })
        pendingParts.delete(key)
        renderError = ''
        partImagesVersion++
      })
      .catch((err) => {
        pendingParts.delete(key)
        renderError = `${key}: ${String(err)}`
      })
  }

  /**
   * A miswired part gets a badge on the canvas, and the properties strip spells it
   * out once the part is selected. A bare dot would explain nothing on its own.
   */
  function drawWarning(
    ctx: CanvasRenderingContext2D,
    t: ReturnType<typeof makeTransform>,
    inst: Instance,
    palette: Palette,
    zoom: number,
  ) {
    const size = sizesById[inst.id] ?? partImages.get(partKey(inst))
    const at = t.toScreen({ x: inst.x + (size?.w ?? 0) - 2, y: inst.y + 2 })
    drawDisc(ctx, at, 6 * zoom, palette.accent, palette.bg)
    ctx.save()
    ctx.fillStyle = palette.bg
    ctx.font = `bold ${7 * zoom}px sans-serif`
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText('!', at.x, at.y)
    ctx.restore()
  }

  function drawCircuit() {
    const c = renderCanvas
    if (!c || !canvasEl) return
    const ctx = c.getContext('2d')
    if (!ctx) return
    const dpr = window.devicePixelRatio || 1
    const rect = canvasEl.getBoundingClientRect()
    const view = {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      dpr,
      zoom,
      panX,
      panY,
    }
    if (c.width !== view.width * dpr || c.height !== view.height * dpr) {
      c.width = view.width * dpr
      c.height = view.height * dpr
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    clear(ctx, view, palette)
    drawGrid(ctx, view, palette, zoom >= 0.67)
    const t = makeTransform(view)

    // Each part is drawn with its own pins, so a part on top hides the pins of
    // the parts beneath it; wires go over everything (like Wokwi).
    for (const inst of renderOrder) {
      const sprite = partImages.get(partKey(inst))
      if (sprite) {
        const p = t.toScreen({ x: inst.x, y: inst.y })
        const pw = sprite.w * zoom
        const ph = sprite.h * zoom
        ctx.save()
        if (inst.rotate) {
          ctx.translate(p.x + pw / 2, p.y + ph / 2)
          ctx.rotate((inst.rotate * Math.PI) / 180)
          ctx.drawImage(sprite.image, -pw / 2, -ph / 2, pw, ph)
        } else {
          ctx.drawImage(sprite.image, p.x, p.y, pw, ph)
        }
        ctx.restore()
      } else {
        rasterizePart(inst)
      }
      // A marker only for the pin under the pointer: marking every pin reads as
      // static on a breadboard and is clutter on anything else.
      for (const pin of pinsById[inst.id] ?? []) {
        const hovered = hoveredPin?.instanceId === inst.id && hoveredPin.pin === pin.name
        if (!hovered) continue
        drawSquare(
          ctx,
          t.toScreen(pinCanvasPoint(inst, pin)),
          (hovered ? 10 : 7) * zoom,
          hovered ? palette.accent : palette.wire,
          palette.bg,
        )
      }
      if (warnings[inst.id]) drawWarning(ctx, t, inst, palette, zoom)
    }

    for (const w of wires) {
      const a = pinPoint(w.a)
      const b = pinPoint(w.b)
      if (!a || !b) continue
      const pts = expandPath([a, ...w.waypoints, b]).map(t.toScreen)
      drawPolyline(ctx, pts, wireColour(w.colour), (selectedWireId === w.id ? 4 : 2.5) * zoom)
    }

    // A ring for every selected part, not only the one the strip is editing.
    for (const id of selectedIds) {
      const inst = instances.find((i) => i.id === id)
      if (inst) {
        const p = t.toScreen({ x: inst.x, y: inst.y })
        const sprite = partImages.get(partKey(inst))
        const host = canvasEl.querySelector(`.instance[data-id="${inst.id}"]`) as HTMLElement | null
        const sw = (sprite?.w ?? host?.offsetWidth ?? 0) * zoom
        const sh = (sprite?.h ?? host?.offsetHeight ?? 0) * zoom
        ctx.save()
        ctx.translate(p.x + sw / 2, p.y + sh / 2)
        if (inst.rotate) ctx.rotate((inst.rotate * Math.PI) / 180)
        ctx.strokeStyle = palette.accent
        ctx.lineWidth = 2 * zoom
        ctx.strokeRect(-sw / 2 - 3 * zoom, -sh / 2 - 3 * zoom, sw + 6 * zoom, sh + 6 * zoom)
        ctx.restore()
      }
    }

    if (wiring) {
      drawPolyline(ctx, expandPath(wiringPoints).map(t.toScreen), palette.wire, 2.5 * zoom, [4, 4])
      const snap = wiringTarget ? pinPoint(wiringTarget) : null
      if (snap) drawDisc(ctx, t.toScreen(snap), 7 * zoom, '', palette.accent)
    }

    if (selectedWireId) {
      const w = wires.find((x) => x.id === selectedWireId)
      if (w) {
        for (const ref of [w.a, w.b]) {
          const p = pinPoint(ref)
          if (p) drawDisc(ctx, t.toScreen(p), 5 * zoom, palette.accent, palette.bg)
        }
      }
    }
  }

  // Redraw whenever anything the canvas shows changes.
  $effect(() => {
    void partImagesVersion
    void palette
    for (const inst of instances) {
      void inst.x
      void inst.y
      void inst.rotate
    }
    for (const w of wires) {
      void w.waypoints
      void w.colour
    }
    for (const key in pinsById) void pinsById[key]
    for (const key in sizesById) void sizesById[key]
    void selectedId
    void selectedIds
    void marquee
    void selectedWireId
    void hoveredPin
    void wiring
    void cursor
    void rerouting
    drawCircuit()
  })

  // Keep the canvas palette in step with the theme.
  $effect(() => {
    void theme
    palette = readPalette()
  })

  // ---- Simulation --------------------------------------------------------
  /**
   * What the simulation drives on a part's element, if anything. Only the
   * *property name* is per-kind, and that is behaviour rather than registration,
   * so it lives here instead of being scattered through the template.
   */
  function liveProp(inst: Instance): { name: string; value: unknown }[] | null {
    if (inst.kind === 'uno') return [{ name: 'led13', value: pin13 }]
    if (inst.kind === 'led') {
      const state = ledStates[inst.id] ?? 'off'
      // `brightness` is how the element shows a dim LED; `value` is on/off, and it
      // has to be false when a burnt LED is dark or the element would still glow.
      return [
        { name: 'brightness', value: LED_BRIGHTNESS[state] },
        { name: 'value', value: state === 'lit' || state === 'dim' },
      ]
    }
    if (inst.kind === 'pushbutton') return [{ name: 'pressed', value: !!pressedButtons[inst.id] }]
    return null
  }

  /** How brightly each state shows, and a digit so variants cache separately. */
  const LED_BRIGHTNESS: Record<LedState, number> = { off: 0, dim: 0.4, lit: 1, burnt: 0 }
  const LED_ORDER: Record<LedState, number> = { off: 0, dim: 1, lit: 2, burnt: 3 }

  /** One action for every part: push that state into whichever element it is. */
  function liveState(node: HTMLElement, state: { name: string; value: unknown }[] | null) {
    const apply = (s: { name: string; value: unknown }[] | null) => {
      for (const item of s ?? []) {
        ;(node as unknown as Record<string, unknown>)[item.name] = item.value
      }
    }
    apply(state)
    return { update: apply }
  }

  const lastDrives = new Map<string, boolean>()

  // When the circuit or the chip's pins change: re-solve the netlist, light the
  // LEDs, and push any button-driven pins back into the simulator.
  $effect(() => {
    const parts = instances.map((i) => ({
      id: i.id,
      kind: i.kind,
      attrs: attrsOf(i),
      // Canvas-space pins, so a leg resting in a breadboard hole can connect.
      pins: (pinsById[i.id] ?? []).map((p) => ({
        name: p.name,
        ...pinCanvasPoint(i, p),
      })),
    }))
    const refs = wires.map((w) => ({ a: { ...w.a }, b: { ...w.b } }))
    // Handed over by reference rather than spread, and that matters: reading
    // `pressedButtons[id]` is tracked even for an id that is not in the object yet,
    // which is what carries a press from the canvas into the simulator. Spreading
    // reads only the keys it already has, so a button that had never been pressed
    // was invisible to this effect and no pin was ever driven.
    const result = evaluateCircuit(parts, refs, pressedButtons, avrPins)
    ledStates = result.leds
    warnings = result.warnings
    if (!inTauri || !running) {
      lastDrives.clear()
      return
    }
    for (const d of result.drives) {
      const k = `${d.port}${d.pin}`
      if (lastDrives.get(k) === d.high) continue
      lastDrives.set(k, d.high)
      // Not fire-and-forget: a rejected command used to vanish silently, which is
      // exactly how "the button does nothing" hid for this long.
      invoke('set_input', { port: d.port.charCodeAt(0), pin: d.pin, high: d.high }).catch((err) =>
        console.error('set_input failed', err),
      )
    }
  })

  async function runSim() {
    buildError = ''
    serial = ''
    pin13 = false
    // A stopped chip drives nothing — clear the levels it last reported, or an
    // external LED would stay lit at whatever the sketch left it on.
    avrPins = {}
    // A fresh run never inherits a half-drawn wire or a held button.
    wiring = null
    rerouting = null
    pressedButtons = {}
    try {
      const fw = await invoke<{ elf: string }>('compile_sketch', { sketch })
      await invoke('start_sim', { elf: fw.elf, bounce: switchBounce })
      running = true
    } catch (err) {
      buildError = String(err)
      running = false
    }
    showSerial = true
  }

  async function stopSim() {
    try {
      await invoke('stop_sim')
    } catch {
      /* ignore */
    }
    running = false
    pin13 = false
    avrPins = {}
    pressedButtons = {}
  }

  // Apply an edited sketch without leaving the run: recompile and restart.
  async function restartSim() {
    await stopSim()
    await runSim()
  }

  /**
   * Hand focus to the canvas.
   *
   * Blurring the editor is not enough on every engine: WebKitGTK can leave a
   * contenteditable holding focus, so keys carry on going into the code and nothing
   * you press reaches the circuit — the shortcuts look dead even while you are
   * dragging a part. Making the canvas the active element is unambiguous, and it is
   * what the canvas is for: it is the `role="application"` the keys act on.
   */
  function focusCanvas() {
    canvasEl?.focus({ preventScroll: true })
  }

  // Pressing anywhere outside the code editor moves focus off it, so the canvas
  // keybinds (Delete, R, Ctrl+Z…) work straight after editing.
  $effect(() => {
    const onDown = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null
      if (target?.closest('.cm-editor')) return
      // Something that takes focus itself — a control, a menu item — is left to the
      // browser; we would only be stealing it back.
      if (target?.closest('input, textarea, select, button, [contenteditable]')) return
      const active = document.activeElement as HTMLElement | null
      if (
        active &&
        active !== document.body &&
        (active.isContentEditable || active.tagName === 'INPUT' || active.tagName === 'TEXTAREA')
      ) {
        active.blur()
      }
      focusCanvas()
    }
    document.addEventListener('pointerdown', onDown, true)
    return () => document.removeEventListener('pointerdown', onDown, true)
  })

  // Stream pin changes and serial bytes from the engine (set up once).
  $effect(() => {
    if (!inTauri) return
    let alive = true
    const unlisten: (() => void)[] = []
    listen<{ port: number; pin: number; high: boolean }>('sim:pin', (e) => {
      avrPins[`${String.fromCharCode(e.payload.port)}${e.payload.pin}`] = e.payload.high
      if (e.payload.port === 66 && e.payload.pin === 5) pin13 = e.payload.high
    }).then((u) => alive && unlisten.push(u))
    listen<number>('sim:serial', (e) => {
      // Append, then trim only when over the cap. Slicing on every byte is O(n²) — at
      // 115200 baud that rebuilds a 20 000-character string a thousand times a second,
      // which is enough to stutter the UI.
      serial += String.fromCharCode(e.payload)
      if (serial.length > 20000) serial = serial.slice(-20000)
    }).then((u) => alive && unlisten.push(u))
    return () => {
      alive = false
      unlisten.forEach((u) => u())
    }
  })
</script>

<div class="app">
  <header class="topbar">
    <span class="brand">Croquis</span>

    <nav class="menubar" aria-label="Main menu">
      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'file'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'file'}
          onclick={() => toggleMenu('file')}
          onpointerenter={() => hoverMenu('file')}
        >
          File
        </button>
        {#if openMenu === 'file'}
          <div class="menu menu--anchored">
            <button class="menu__item" onclick={() => menuPick(newProject)}>
              <span>New</span><span class="menu__key">Ctrl+N</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" disabled={!inTauri} onclick={() => menuPick(onOpen)}>
              <span>Open…</span><span class="menu__key">Ctrl+O</span>
            </button>
            <button class="menu__item" disabled={!inTauri} onclick={() => menuPick(onSave)}>
              <span>Save</span><span class="menu__key">Ctrl+S</span>
            </button>
            <button class="menu__item" disabled={!inTauri} onclick={() => menuPick(onSaveAs)}>
              <span>Save as…</span><span class="menu__key">Ctrl+Shift+S</span>
            </button>
            <div class="menu__rule"></div>
            <!-- A labelled section rather than a submenu: one click, and a stack of
                 menus is the thing this app keeps out. These work without Tauri too,
                 since they are compiled in rather than read from disk. -->
            <div class="menu__note">Presets</div>
            {#each PRESETS as preset (preset.id)}
              <button class="menu__item" onclick={() => menuPick(() => loadPreset(preset))}>
                <span>{preset.label}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'edit'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'edit'}
          onclick={() => toggleMenu('edit')}
          onpointerenter={() => hoverMenu('edit')}
        >
          Edit
        </button>
        {#if openMenu === 'edit'}
          <div class="menu menu--anchored">
            <button class="menu__item" disabled={running || !history.length} onclick={() => menuPick(undo)}>
              <span>Undo</span><span class="menu__key">Ctrl+Z</span>
            </button>
            <button class="menu__item" disabled={running || !future.length} onclick={() => menuPick(redo)}>
              <span>Redo</span><span class="menu__key">Ctrl+Shift+Z</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" disabled={running || !hasSelection} onclick={() => menuPick(cutSelection)}>
              <span>Cut</span><span class="menu__key">Ctrl+X</span>
            </button>
            <button class="menu__item" disabled={!hasSelection} onclick={() => menuPick(copySelection)}>
              <span>Copy</span><span class="menu__key">Ctrl+C</span>
            </button>
            <button class="menu__item" disabled={running || !clipboard} onclick={() => menuPick(paste)}>
              <span>Paste</span><span class="menu__key">Ctrl+V</span>
            </button>
            <button
              class="menu__item"
              disabled={running || !instances.length}
              onclick={() => menuPick(selectAll)}
            >
              <span>Select all</span><span class="menu__key">Ctrl+A</span>
            </button>
            <button
              class="menu__item"
              disabled={running || !hasSelection}
              onclick={() => menuPick(duplicateSelection)}
            >
              <span>Duplicate</span><span class="menu__key">Ctrl+D</span>
            </button>
            <div class="menu__rule"></div>
            <button
              class="menu__item"
              disabled={running || !hasSelection}
              onclick={() => menuPick(() => rotatePart(1))}
            >
              <span>Rotate 90°</span><span class="menu__key">R</span>
            </button>
            <button
              class="menu__item"
              disabled={running || !hasSelection}
              onclick={() => menuPick(() => rotatePart(2))}
            >
              <span>Rotate 180°</span><span class="menu__key">Shift+R</span>
            </button>
            <div class="menu__rule"></div>
            <button
              class="menu__item menu__item--danger"
              disabled={running || !hasSelection}
              onclick={() => menuPick(deleteSelection)}
            >
              <span>Delete</span><span class="menu__key">Del</span>
            </button>
          </div>
        {/if}
      </div>

      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'view'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'view'}
          onclick={() => toggleMenu('view')}
          onpointerenter={() => hoverMenu('view')}
        >
          View
        </button>
        {#if openMenu === 'view'}
          <div class="menu menu--anchored">
            <button
              class="menu__item"
              onclick={() => menuPick(() => (theme = theme === 'dark' ? 'light' : 'dark'))}
            >
              <span>{theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" onclick={() => menuPick(fitView)}>
              <span>Fit view to circuit</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(zoomIn)}>
              <span>Zoom in</span><span class="menu__key">Ctrl +</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(zoomOut)}>
              <span>Zoom out</span><span class="menu__key">Ctrl -</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(resetZoom)}>
              <span>Reset zoom</span><span class="menu__key">Ctrl 0</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" onclick={() => menuPick(() => (leftCollapsed = !leftCollapsed))}>
              <span>{leftCollapsed ? 'Show' : 'Hide'} Sketch panel</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(() => (rightCollapsed = !rightCollapsed))}>
              <span>{rightCollapsed ? 'Show' : 'Hide'} Components panel</span>
            </button>
          </div>
        {/if}
      </div>

      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'run'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'run'}
          onclick={() => toggleMenu('run')}
          onpointerenter={() => hoverMenu('run')}
        >
          Run
        </button>
        {#if openMenu === 'run'}
          <div class="menu menu--anchored">
            <button
              class="menu__item"
              disabled={!inTauri || running}
              onclick={() => menuPick(runSim)}
            >
              <span>Run</span>
            </button>
            <button class="menu__item" disabled={!running} onclick={() => menuPick(restartSim)}>
              <span>Restart</span>
            </button>
            <button class="menu__item" disabled={!running} onclick={() => menuPick(stopSim)}>
              <span>Stop</span>
            </button>
            <div class="menu__rule"></div>
            <!-- A setting rather than an action, which is why it sits below the rule.
                 Takes effect on the next Run, since the contacts' behaviour is fixed
                 when the simulation starts. -->
            <button
              class="menu__item"
              role="menuitemcheckbox"
              aria-checked={switchBounce}
              onclick={() => menuPick(() => (switchBounce = !switchBounce))}
            >
              <span>Switch bounce</span>
              <span class="menu__key">{switchBounce ? 'On' : 'Off'}</span>
            </button>
          </div>
        {/if}
      </div>
    </nav>
  </header>

  <main class="workspace">
    <section
      class="panel panel--code"
      class:is-collapsed={leftCollapsed}
      style="flex:0 0 {leftCollapsed ? 0 : leftWidth}px"
    >
      <header class="panel__head">
        <span class="panel__title">Sketch</span>
        <button
          class="icon-btn"
          title="Collapse the sketch panel"
          aria-label="Collapse the sketch panel"
          onclick={() => (leftCollapsed = true)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html CHEVRON_LEFT}</svg>
        </button>
      </header>
      <div class="panel__body panel__body--code">
        <CodeEditor bind:value={sketch} />
      </div>
    </section>

    {#if leftCollapsed}
      <aside class="rail rail--left">
        <button
          class="rail__btn"
          title="Sketch"
          aria-label="Expand the sketch panel"
          onclick={() => (leftCollapsed = false)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html ICON_SKETCH}</svg>
        </button>
      </aside>
    {:else}
      <div
        class="splitter"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the sketch panel"
        onpointerdown={(e) => startResize(e, 'left')}
      ></div>
    {/if}

    <section class="panel panel--canvas">
      <header class="panel__head">
        <span class="panel__title">Circuit</span>
        {#if selectedWireId}
          <div class="wire-colours">
            {#each WIRE_COLORS as c (c.name)}
              <button
                class="wire-colour"
                class:wire-colour--active={wires.find((w) => w.id === selectedWireId)?.colour === c.name}
                style="--swatch: {c.hex}"
                title={c.name}
                aria-label="Wire colour: {c.name}"
                onclick={() => setWireColour(c.name)}
              ></button>
            {/each}
          </div>
        {/if}
        <button class="icon-btn" title="Toggle the serial monitor" aria-label="Toggle the serial monitor" onclick={() => (showSerial = !showSerial)}>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 6h16M4 10h16M4 14h10" />
          </svg>
        </button>
        {#if selectedId}
          <button class="icon-btn" title="Rotate 90° (R)" aria-label="Rotate part" onclick={() => rotatePart(1)}>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v4h-4" />
            </svg>
          </button>
        {/if}
        <button class="icon-btn" title="Fit view to circuit" aria-label="Fit view to circuit" onclick={fitView}>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
          </svg>
        </button>
        <button
          class="icon-btn"
          disabled={running}
          title={running ? 'Stop the simulation to clear the circuit' : 'Clear the canvas'}
          aria-label="Clear the canvas"
          onclick={clearCanvas}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M5 7h14M9 7V5h6v2M7 7l1 12h8l1-12" />
          </svg>
        </button>
        <div class="zoom">
          <button class="icon-btn" title="Zoom out (Ctrl -)" aria-label="Zoom out" onclick={zoomOut}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 12h12" /></svg>
          </button>
          <button class="zoom__level" title="Reset zoom to 100% (Ctrl 0)" onclick={resetZoom}>
            {Math.round(zoom * 100)}%
          </button>
          <button class="icon-btn" title="Zoom in (Ctrl +)" aria-label="Zoom in" onclick={zoomIn}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 6v12M6 12h12" /></svg>
          </button>
        </div>
        <span class="spacer"></span>
        {#if running}
          <button class="btn" title="Recompile and restart with the current sketch" onclick={restartSim}>
            Restart
          </button>
          <button class="btn" onclick={stopSim}>Stop</button>
        {:else}
          <button
            class="btn btn--primary"
            disabled={!inTauri}
            title={inTauri ? 'Compile and run' : 'Available in the desktop app'}
            onclick={runSim}
          >
            Run
          </button>
        {/if}
      </header>
      <!-- The selected part's properties, docked under the circuit bar. Always
           present (so the canvas never jumps) and generic: the catalogue says
           what's editable. -->
      <div class="props">
        {#if selectedPart}
          {@const def = partByKind(selectedPart.kind)}
          <span class="props__name">{def.label}</span>
          {#if warnings[selectedPart.id]}
            <span class="props__warn">{warnings[selectedPart.id]}</span>
          {/if}
          {@const attrs = attrsOf(selectedPart)}
          {#each def.fields ?? [] as field (`${selectedPart.id}:${field.name}`)}
            {@const id = `prop-${selectedPart.id}-${field.name}`}
            <span class="props__field">
              <label class="props__label" for={id}>{field.label}</label>
              {#if field.control.kind === 'select'}
                {@const current = field.control.options.find((o) => o.value === attrs[field.name])}
                <span class="props__control">
                  {#if current?.swatch}
                    <span class="props__swatch" style="background:{current.swatch}"></span>
                  {/if}
                  <select
                    {id}
                    class="props__input props__select"
                    value={attrs[field.name] ?? ''}
                    disabled={running}
                    onchange={(e) => setAttr(selectedPart, field.name, e.currentTarget.value)}
                  >
                    {#each field.control.options as opt (opt.value)}
                      <option value={opt.value}>{opt.label}</option>
                    {/each}
                  </select>
                </span>
              {:else}
                <span class="props__control">
                  <input
                    {id}
                    class="props__input props__number"
                    type="text"
                    inputmode="decimal"
                    spellcheck="false"
                    bind:value={resText}
                    disabled={running}
                    onchange={() => commitResistance(field, resText)}
                  />
                  <select
                    class="props__input props__select props__select--unit"
                    aria-label={`${field.label} unit`}
                    value={String(resUnit)}
                    disabled={running}
                    onchange={(e) => pickUnit(Number(e.currentTarget.value))}
                  >
                    {#each field.control.units as unit (unit.factor)}
                      <option value={String(unit.factor)}>{unit.label}</option>
                    {/each}
                  </select>
                </span>
              {/if}
            </span>
          {/each}
        {:else}
          <span class="props__hint">Select a part to change its properties.</span>
        {/if}
      </div>

      {#if running}
        <div class="canvas-note">Simulation running — the circuit is frozen. Stop to edit it.</div>
      {/if}
      {#if renderError}
        <div class="render-error">{renderError}</div>
      {/if}
      <div
        class="panel__body panel__body--canvas"
        class:is-wiring={!!wiring || !!rerouting}
        role="application"
        aria-label="Circuit canvas"
        tabindex="-1"
        bind:this={canvasEl}
        onpointerdown={onCanvasPointerDown}
        onpointermove={onCanvasHover}
        oncontextmenu={openContextMenu}
      >
        <canvas class="circuit-canvas" bind:this={renderCanvas}></canvas>
        {#if savedNote}
          <div class="saved-note" role="status">Saved</div>
        {/if}
        {#if marquee}
          <div
            class="marquee"
            style="left:{marquee.left}px;top:{marquee.top}px;width:{marquee.width}px;height:{marquee.height}px"
          ></div>
        {/if}
        <div
          class="canvas-content"
          style="transform: {zoom === 1 && panX === 0 && panY === 0
            ? 'none'
            : `translate(${panX}px, ${panY}px) scale(${zoom})`}"
        >
          {#if zoom >= 0.34}
            <svg class="grid" aria-hidden="true">
              <defs>
                <pattern id="grid-coarse" width="24" height="24" patternUnits="userSpaceOnUse">
                  <path
                    d="M24 0H0V24"
                    fill="none"
                    style="stroke: var(--border); stroke-width: 1; opacity: 0.3"
                  />
                </pattern>
                <pattern id="grid-fine" width="12" height="12" patternUnits="userSpaceOnUse">
                  <path
                    d="M12 0H0V12"
                    fill="none"
                    style="stroke: var(--border); stroke-width: 1; opacity: 0.13"
                  />
                </pattern>
              </defs>
              <rect width="100%" height="100%" fill="url(#grid-coarse)" />
              {#if zoom >= 0.67}
                <rect width="100%" height="100%" fill="url(#grid-fine)" />
              {/if}
            </svg>
          {/if}

        <!-- Wire hit-targets live under the parts, so they never block pins. -->
        <svg class="wires" aria-hidden="true">
          {#each wires as w (w.id)}
            {@const pts = wirePoints(w)}
            {#if pts.length >= 2}
              <path
                class="wire__hit"
                data-wire={w.id}
                role="button"
                aria-label="Select or reshape wire"
                tabindex="-1"
                d={routePath(pts)}
                onpointerdown={(e) => startWireDrag(e, w.id)}
              />
            {/if}
          {/each}
        </svg>

        <div class="canvas-layer">
          {#each renderOrder as inst (inst.id)}
            {@const def = partByKind(inst.kind)}
            <div
              class="instance"
              class:instance--selected={selectedIds.includes(inst.id)}
              data-id={inst.id}
              role="button"
              aria-label={def.label}
              tabindex="0"
              style="transform: translate({domX(inst)}px, {domY(inst)}px) rotate({inst.rotate ?? 0}deg)"
              onpointerdown={(e) => startMove(e, inst)}
            >
              <!-- One element for every part, named by the catalogue — so adding
                   a component is a catalogue entry, not a branch here. -->
              <svelte:element this={def.tag} use:liveState={liveProp(inst)} {...attrsOf(inst)}
              ></svelte:element>

              <!-- A breadboard's 400 holes are hit-tested from data, not DOM. -->
              {#if !def.skipDomPins}
                {#each pinsById[inst.id] ?? [] as p (p.name)}
                  <div
                    class="pin"
                    data-pin={p.name}
                    aria-hidden="true"
                    title={p.name}
                    style="left:{p.x}px; top:{p.y}px"
                  ></div>
                {/each}
              {/if}
            </div>
          {/each}
        </div>

        <!-- Visible wires (and the live one, snap ring, and endpoint handles)
             render above the parts, like Wokwi. -->
        <svg class="wires" aria-hidden="true">
          {#each wires as w (w.id)}
            {@const pts = wirePoints(w)}
            {#if pts.length >= 2}
              <path
                class="wire__line"
                class:wire__line--selected={selectedWireId === w.id}
                style="stroke: {wireHex(w.colour)}"
                d={routePath(pts)}
              />
            {/if}
          {/each}

          {#if wiring}
            {@const d = routePath(wiringPoints)}
            {#if d}
              <path class="wire__line wire__line--preview" d={d} />
            {/if}
            {@const snap = wiringTarget ? pinPoint(wiringTarget) : null}
            {#if snap}
              <circle class="wire__snap" cx={snap.x} cy={snap.y} r="7" />
            {/if}
          {/if}

          {#if selectedWireId}
            {@const w = wires.find((x) => x.id === selectedWireId)}
            {#if w}
              {@const a = pinPoint(w.a)}
              {@const b = pinPoint(w.b)}
              {#if a}
                <circle
                  class="wire__handle"
                  role="button"
                  aria-label="Wire endpoint"
                  tabindex="-1"
                  cx={a.x}
                  cy={a.y}
                  r="5"
                  onpointerdown={(e) => startEndpointDrag(e, w.id, 'a')}
                />
              {/if}
              {#if b}
                <circle
                  class="wire__handle"
                  role="button"
                  aria-label="Wire endpoint"
                  tabindex="-1"
                  cx={b.x}
                  cy={b.y}
                  r="5"
                  onpointerdown={(e) => startEndpointDrag(e, w.id, 'b')}
                />
              {/if}
            {/if}
          {/if}
        </svg>

        </div>

        {#if instances.length === 0}
          <p class="placeholder placeholder--center">Drag a component to start.</p>
        {/if}
      </div>

      {#if showSerial}
        <div class="serial">
          <header class="panel__head">
            <span class="panel__title">Serial Monitor</span>
            <button
              class="icon-btn"
              title="Close the serial monitor"
              aria-label="Close the serial monitor"
              onclick={() => (showSerial = false)}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
            </button>
          </header>
          {#if buildError}
            <pre class="serial__out serial__out--error">{buildError}</pre>
          {:else}
            <pre class="serial__out">{serial || (running ? '' : 'No output.')}</pre>
          {/if}
        </div>
      {/if}
    </section>

    {#if rightCollapsed}
      <aside class="rail rail--right">
        <button
          class="rail__btn"
          title="Components"
          aria-label="Expand the components panel"
          onclick={() => (rightCollapsed = false)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html ICON_COMPONENTS}</svg>
        </button>
      </aside>
    {:else}
      <div
        class="splitter"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the components panel"
        onpointerdown={(e) => startResize(e, 'right')}
      ></div>
    {/if}

    <section
      class="panel panel--components"
      class:is-collapsed={rightCollapsed}
      style="flex:0 0 {rightCollapsed ? 0 : rightWidth}px"
    >
      <header class="panel__head">
        <span class="panel__title">Components</span>
        <button
          class="icon-btn"
          title="Collapse the components panel"
          aria-label="Collapse the components panel"
          onclick={() => (rightCollapsed = true)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html CHEVRON_RIGHT}</svg>
        </button>
      </header>
      <div class="panel__body">
        <ul class="parts" bind:this={partsListEl}>
          {#each PARTS as part (part.kind)}
            <li
              class="part"
              class:is-locked={running}
              data-kind={part.kind}
              title={running ? 'Stop the simulation to add parts' : part.label}
              onpointerdown={(e) => startPaletteDrag(e, part)}
            >
              {#if partArt[part.kind]}
                <img
                  class="part__thumb"
                  src={partArt[part.kind].href}
                  alt=""
                  draggable="false"
                />
              {:else}
                <svg class="part__icon" viewBox="0 0 24 24" aria-hidden="true">{@html part.icon}</svg>
              {/if}
              <!-- The real element, hidden: it's what the thumbnail is drawn from. -->
              <span class="part__art" aria-hidden="true">
                <svelte:element this={part.tag} {...part.attrs}></svelte:element>
              </span>
              <span class="part__label">{part.label}</span>
            </li>
          {/each}
        </ul>
      </div>
    </section>
  </main>

  {#if dragging}
    <div class="drag-ghost" style="left:{dragging.x}px; top:{dragging.y}px">
      <svg viewBox="0 0 24 24" aria-hidden="true">{@html dragging.def.icon}</svg>
    </div>
  {/if}

  {#if menu}
    <div
      class="menu"
      style="left:{menu.x}px; top:{menu.y}px"
    >
      {#if menu.partId}
        <button class="menu__item" disabled={running} onclick={() => runMenu(copySelection)}>
          Copy
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(cutSelection)}>
          Cut
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(duplicateSelection)}>
          Duplicate
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(() => rotatePart(1))}>
          Rotate 90°
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(() => rotatePart(2))}>
          Rotate 180°
        </button>
        {#if menuIsMcu()}
          <div class="menu__rule"></div>
          <button class="menu__item" onclick={editSketch}>Edit code</button>
        {/if}
        <div class="menu__rule"></div>
        <button
          class="menu__item menu__item--danger"
          disabled={running}
          onclick={() => runMenu(deleteSelection)}
        >
          Delete
        </button>
      {:else}
        <button
          class="menu__item menu__item--danger"
          disabled={running}
          onclick={() => runMenu(deleteSelection)}
        >
          Delete wire
        </button>
      {/if}
      {#if running}
        <div class="menu__note">Circuit frozen while running</div>
      {/if}
    </div>
  {/if}

  {#if hoveredPin && hoverPos}
    {@const tip = pinTooltip(hoveredPin)}
    {#if tip}
      <div
        class="pin-tip"
        style="left:{Math.min(hoverPos.x + 14, window.innerWidth - 240)}px; top:{hoverPos.y + 16}px"
      >
        <span class="pin-tip__part">{tip.part}</span>
        <span class="pin-tip__name">{tip.name}</span>
        {#if tip.note}<span class="pin-tip__note">{tip.note}</span>{/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .app {
    display: grid;
    grid-template-rows: auto 1fr;
    height: 100vh;
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 40px;
    padding: 0 var(--space-4);
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .brand {
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .spacer {
    flex: 1;
  }

  .workspace {
    display: flex;
    min-height: 0;
  }

  .panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }

  /* A collapsed panel keeps its content mounted (so the editor keeps its
     history) but clips it to nothing; the rail beside it takes over. */
  .panel.is-collapsed {
    overflow: hidden;
  }

  .panel--canvas {
    flex: 1 1 auto;
    background: var(--bg);
  }

  .panel__head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 32px;
    padding: 0 var(--space-2) 0 var(--space-3);
    font-size: var(--fs-ui-dense);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
  }

  .panel__title {
    flex: 1;
  }

  .panel__body {
    flex: 1;
    min-height: 0;
    padding: var(--space-3);
    overflow: auto;
  }

  .panel__body--code {
    padding: 0;
    overflow: hidden;
  }

  .panel__body--canvas {
    position: relative;
    display: grid;
    place-items: center;
    padding: 0;
    overflow: hidden;
  }

  /* The circuit's DOM lives in here for hit-testing, but the visible pixels come
     from the canvas — so the DOM itself is kept transparent. */
  .canvas-content {
    position: absolute;
    inset: 0;
    transform-origin: 0 0;
    opacity: 0;
  }

  /* The visible circuit, drawn at full device resolution. */
  .circuit-canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  /* The selected part's properties, docked under the circuit bar. */
  .props {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex: none;
    min-height: 30px;
    padding: 3px var(--space-3);
    font-size: var(--fs-ui-dense);
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }

  .props__name {
    color: var(--text);
  }

  /* A part that's wired wrong says so, in the strip, in the warning colour. */
  .props__warn {
    color: var(--warning);
  }

  .props__field {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .props__control {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  /* Both controls share one look: raised surface, hairline border, no shadow. */
  .props__input {
    font-family: inherit;
    font-size: 11px;
    padding: 2px 6px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
  }

  .props__input:disabled {
    color: var(--text-muted);
    cursor: not-allowed;
  }

  /* The platform arrow is replaced by a caret we can colour, so the control reads
     the same in both themes instead of depending on the GTK widget's styling. */
  .props__select {
    appearance: none;
    padding-right: 19px;
    background-image:
      linear-gradient(45deg, transparent 50%, currentColor 50%),
      linear-gradient(135deg, currentColor 50%, transparent 50%);
    background-position:
      right 8px top 55%,
      right 4px top 55%;
    background-size:
      4px 4px,
      4px 4px;
    background-repeat: no-repeat;
  }

  .props__select--unit {
    padding-right: 16px;
    background-position:
      right 6px top 55%,
      right 2px top 55%;
  }

  .props__number {
    width: 5.5em;
    font-family: var(--font-mono);
  }

  .props__swatch {
    width: 9px;
    height: 9px;
    border: 1px solid var(--border);
  }

  .props__hint {
    color: var(--text-muted);
  }

  /* The circuit is frozen while a sketch runs — say so, don't leave it a mystery. */
  /* The right-drag selection box. In the DOM rather than on the canvas because it is
     screen-space and has no circuit geometry to speak of. Hollow, so it reads as a
     selection rather than as a shape. */
  .marquee {
    position: absolute;
    z-index: 2;
    border: 1px solid var(--accent);
    pointer-events: none;
  }

  /* Floated rather than laid out, so it cannot shift the circuit when it comes and
     goes — and with no transition on purpose: WebKitGTK drops to low-resolution tiles
     while any animation runs, which blurs the whole canvas. It pops, then it is gone. */
  .saved-note {
    position: absolute;
    left: var(--space-3);
    bottom: var(--space-3);
    z-index: 3;
    padding: 3px var(--space-3);
    font-size: 11px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    pointer-events: none;
  }

  .canvas-note {
    flex: none;
    padding: 4px var(--space-3);
    font-size: 11px;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* The right-click menu: in-app, tokens only, and floated with a border
     rather than a shadow — the app has no shadows anywhere. */
  .menu {
    position: fixed;
    z-index: 120;
    min-width: 176px;
    padding: var(--space-1) 0;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-panel);
  }

  /* A dropdown anchored under its menu-bar title. */
  .menu--anchored {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 216px;
  }

  /* The menu bar: only commands that have no other home live in here. */
  .menubar {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: var(--space-3);
  }

  .menubar__item {
    position: relative;
  }

  .menubar__btn {
    font-family: inherit;
    font-size: var(--fs-ui-dense);
    padding: 3px var(--space-2);
    color: var(--text);
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .menubar__btn:hover,
  .menubar__btn.is-open {
    background: var(--surface-raised);
    border-color: var(--border);
  }

  .menu__item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    color: var(--text);
    background: none;
    border: 0;
    cursor: pointer;
  }

  .menu__item:hover:not(:disabled) {
    background: var(--surface);
  }

  .menu__item:disabled {
    color: var(--text-muted);
    opacity: 0.5;
    cursor: not-allowed;
  }

  .menu__item--danger:hover:not(:disabled) {
    color: var(--error);
  }

  .menu__rule {
    height: 1px;
    margin: var(--space-1) 0;
    background: var(--border);
  }

  .menu__note {
    padding: var(--space-1) var(--space-3) var(--space-2);
    font-size: 11px;
    color: var(--text-muted);
  }

  /* The keyboard shortcut, shown the way a menu bar normally shows it. */
  .menu__key {
    font-size: 11px;
    color: var(--text-muted);
  }

  /* The pin under the pointer: which pin it is, and what it means where the
     element tells us. Most of the value is on multi-pin parts. */
  .pin-tip {
    position: fixed;
    z-index: 130;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: 3px var(--space-2);
    font-size: 11px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    pointer-events: none;
    white-space: nowrap;
  }

  .pin-tip__part {
    color: var(--text-muted);
  }

  .pin-tip__name {
    font-family: var(--font-mono);
  }

  .pin-tip__note {
    color: var(--text-muted);
  }

  /* A part that couldn't be rasterised — shows why. */
  .render-error {
    flex: none;
    padding: 4px var(--space-3);
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--error);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Draggable column divider — a thin line with a forgiving hit area. */
  .splitter {
    flex: 0 0 6px;
    position: relative;
    cursor: col-resize;
    touch-action: none;
  }

  .splitter::before {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    transform: translateX(-50%);
    background: var(--border);
    transition: background var(--dur-micro) var(--ease);
  }

  .splitter:hover::before {
    background: var(--accent);
  }

  /* Collapsed panel rail. */
  .rail {
    flex: 0 0 40px;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: var(--space-2);
    background: var(--surface);
  }

  .rail--left {
    border-right: 1px solid var(--border);
  }

  .rail--right {
    border-left: 1px solid var(--border);
  }

  .rail__btn {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    color: var(--text-muted);
    background: transparent;
    border: 0;
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .rail__btn:hover {
    color: var(--text);
    background: var(--surface-raised);
  }

  .rail__btn svg {
    width: 20px;
    height: 20px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .icon-btn {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    color: var(--text-muted);
    background: transparent;
    border: 0;
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .icon-btn:hover {
    color: var(--text);
    background: var(--surface-raised);
  }

  .icon-btn svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .zoom {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .wire-colours {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-right: var(--space-1);
  }

  .wire-colour {
    width: 12px;
    height: 12px;
    padding: 0;
    background: var(--swatch);
    border: 1px solid var(--border);
    border-radius: 2px;
    cursor: pointer;
  }

  .wire-colour--active {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .zoom__level {
    min-width: 46px;
    padding: 2px 4px;
    font: inherit;
    font-size: var(--fs-ui-dense);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    background: transparent;
    border: 0;
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .zoom__level:hover {
    color: var(--text);
    background: var(--surface-raised);
  }

  /* Faint orthogonal grid. Drawn as SVG patterns rather than tiled CSS
     gradients: CSS background tiles get rasterised with rounding that
     accumulates across the surface (the fine grid visibly drifts away from the
     top-left). SVG patterns are exact geometry, so the grids stay locked. */
  .grid {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  /* Wire layers. Explicit size matters: an unsized <svg> collapses to 300x150
     and clips anything drawn outside it. */
  .wires {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .wire__hit {
    fill: none;
    stroke: transparent;
    stroke-width: 14;
    pointer-events: stroke;
    cursor: pointer;
  }

  .wire__line {
    fill: none;
    stroke: var(--wire);
    stroke-width: 2.5;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .wire__line--selected {
    stroke-width: 4;
  }

  .wire__line--preview {
    stroke-dasharray: 4 4;
    opacity: 0.85;
  }

  .wire__handle {
    fill: var(--accent);
    stroke: var(--bg);
    stroke-width: 1.5;
    pointer-events: all;
    cursor: grab;
  }

  .wire__snap {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    pointer-events: none;
  }

  /* Positioned layer holding placed parts. Passes clicks through empty space. */
  .canvas-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .instance {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 50% 50%;
    pointer-events: auto;
    cursor: grab;
    touch-action: none;
  }

  /* The elements exist only to be measured and hit-tested — the visible circuit
     is the canvas above them. `visibility: hidden` keeps them out of the *paint*
     (a breadboard is ~860 nodes, and dragging one re-rasterised all of it every
     frame) while keeping the layout size the rasteriser reads. */
  .instance > *:first-child {
    visibility: hidden;
  }

  .instance--selected {
    outline: 2px solid var(--accent);
    outline-offset: 3px;
  }

  /* Pins: small square markers, like Wokwi — visible on the part, brighter on
     hover. The wire always snaps to the pin's centre, never where you clicked. */
  .pin {
    position: absolute;
    width: 7px;
    height: 7px;
    margin: -3.5px 0 0 -3.5px;
    background: var(--wire);
    border: 1px solid var(--bg);
    border-radius: 0;
    opacity: 0.5;
    cursor: crosshair;
  }

  .is-wiring .pin {
    opacity: 1;
  }

  .pin:hover {
    transform: scale(1.5);
    background: var(--accent);
  }

  /* Keep the hit area close to the pin's own size, so neighbouring pins (9.5px
     apart on the Uno) don't overlap. Pressing near a pin still snaps to the
     nearest one, so it stays easy to start a wire. */
  .pin::before {
    content: '';
    position: absolute;
    inset: -1px;
  }

  .drag-ghost {
    position: fixed;
    width: 24px;
    height: 24px;
    transform: translate(-50%, -50%);
    pointer-events: none;
    color: var(--accent);
    z-index: 100;
  }

  .drag-ghost svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .placeholder {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-ui-dense);
  }

  .placeholder--center {
    position: relative;
  }

  /* The serial monitor: a docked strip under the canvas (never a popup). */
  .serial {
    display: flex;
    flex-direction: column;
    height: 170px;
    flex: none;
    background: var(--surface);
    border-top: 1px solid var(--border);
  }

  .serial__out {
    flex: 1;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    overflow: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-code);
    line-height: var(--lh-code);
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text);
  }

  .serial__out--error {
    color: var(--error);
  }

  /* Parts: icon over label, in a two-up grid. */
  .parts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .part.is-locked {
    cursor: not-allowed;
    opacity: 0.45;
  }

  .part {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-2);
    text-align: center;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
    cursor: grab;
    touch-action: none;
    transition:
      border-color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .part:hover {
    border-color: var(--text-muted);
  }

  .part__icon {
    width: 24px;
    height: 24px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    color: var(--text-muted);
    transition: color var(--dur-micro) var(--ease);
  }

  .part:hover .part__icon {
    color: var(--text);
  }

  /* The real drawing, scaled into a consistent box. */
  .part__thumb {
    max-width: 100%;
    height: 40px;
    object-fit: contain;
  }

  /* The real element behind each thumbnail: laid out (so it can be rasterised)
     but invisible. It fills the tile so a part that sizes itself from its
     container still lays out — `wokwi-resistor` does, and collapsed to 0px in a
     zero-sized box. `overflow: hidden` keeps a wide part (a breadboard is 627px)
     from spilling into the panel. */
  .part__art {
    position: absolute;
    inset: 0;
    overflow: hidden;
    opacity: 0;
    pointer-events: none;
  }

  .part__label {
    font-size: var(--fs-ui-dense);
    line-height: 1.2;
    color: var(--text);
  }

  .btn {
    font: inherit;
    font-size: var(--fs-ui-dense);
    /* The panel headers upper-case their titles; buttons shouldn't inherit that. */
    text-transform: none;
    padding: 4px 10px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      border-color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .btn:hover:not([disabled]) {
    border-color: var(--text-muted);
  }

  .btn--primary {
    color: var(--accent-contrast);
    background: var(--accent);
    border-color: var(--accent);
  }

  .btn[disabled] {
    opacity: 0.5;
    cursor: default;
  }
</style>
