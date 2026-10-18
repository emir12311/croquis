/**
 * The circuit's canvas surface: pin geometry read off the mounted elements, the
 * sprite cache, the palette thumbnails, and the drawing itself.
 *
 * The circuit is drawn into a `<canvas>` at device resolution (crisp); the DOM
 * stays invisible and handles hit-testing. Geometry is derived here because only
 * the mounted elements know where their pins are.
 */
import { isBreadboard, partByKind, type PartKind } from './parts'
import { shadowSvgMarkup, svgMarkupToImage } from './part-art'
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
} from './circuit-render'
import { snapToHoles } from './snap'
import { BREADBOARD_HOLE_R, BREADBOARD_PITCH } from './breadboard'
import { expandPath } from './route'
import {
  attrsOf,
  renderOrderOf,
  snapToGrid,
  type Instance,
  type PinRef,
  type Point,
  type Wire,
} from './document'
import { WIRE_HEX } from './wire-colours'
import type { LedState } from './netlist'

export type PinGeom = { name: string; x: number; y: number; description?: string }

/** The simulation state that changes a part's drawing. */
export interface RenderState {
  pin13: boolean
  ledStates: Record<string, LedState>
  pressedButtons: Record<string, boolean>
  warnings: Record<string, string>
}

/** Everything the renderer draws, and where the viewport is looking. */
export interface Scene {
  instances: Instance[]
  wires: Wire[]
  selectedIds: string[]
  selectedWireId: string | null
  hoveredPin: PinRef | null
  zoom: number
  panX: number
  panY: number
  wiring: { from: PinRef; waypoints: Point[] } | null
  wiringPoints: Point[]
  wiringTarget: PinRef | null
}

const LED_ORDER: Record<LedState, number> = { off: 0, dim: 1, lit: 2, burnt: 3 }
/** A part within a hole's pitch of one snaps into it; anything further is free. */
const SNAP = { reach: BREADBOARD_PITCH * 1.1, seat: BREADBOARD_HOLE_R }

export class CanvasSurface {
  canvasEl = $state<HTMLDivElement | null>(null)
  renderCanvas = $state<HTMLCanvasElement | null>(null)
  partsListEl = $state<HTMLElement | null>(null)

  pinsById = $state<Record<string, PinGeom[]>>({})
  sizesById = $state<Record<string, { w: number; h: number }>>({})
  palette = $state<Palette>(readPalette())
  renderError = $state('')
  partImagesVersion = $state(0)
  /** Bumped when the canvas element changes size, so the bitmap is redrawn. */
  sizeVersion = $state(0)
  partArt = $state<Record<string, { href: string; width: number; height: number }>>({})

  /** Each kind's pin offsets, taken from the palette's hidden elements. */
  readonly kindPins = new Map<PartKind, PinGeom[]>()

  private pinsVersion = $state(0)
  private pinsRetries = 0
  private partImages = new Map<string, PartSprite>()
  private pendingParts = new Set<string>()
  private partSizes = new Map<string, { w: number; h: number }>()
  private partRetries = new Map<string, number>()
  private artPending = new Set<PartKind>()
  private state: () => RenderState

  constructor(state: () => RenderState) {
    this.state = state
  }

  /**
   * Redraw whenever the canvas element changes size — a window resize, a panel
   * splitter, the serial monitor opening, a collapsed column. Without this the
   * bitmap keeps its old dimensions and CSS stretches it, so the circuit warps
   * until the next interaction happens to trigger a draw.
   */
  watchSize(): () => void {
    const el = this.canvasEl
    if (!el || typeof ResizeObserver === 'undefined') return () => {}
    const observer = new ResizeObserver(() => {
      this.sizeVersion++
    })
    observer.observe(el)
    return () => observer.disconnect()
  }

  // ---- measuring the mounted parts ---------------------------------------

  /** Read every part's pins and size off the DOM. Lit renders asynchronously. */
  measure(instances: Instance[]) {
    void this.pinsVersion
    const next: Record<string, PinGeom[]> = {}
    const sizes: Record<string, { w: number; h: number }> = {}
    let needRetry = false
    for (const inst of instances) {
      const el = this.canvasEl?.querySelector(`.instance[data-id="${inst.id}"]`) as HTMLElement | null
      const comp = el?.firstElementChild as (HTMLElement & { pinInfo?: PinGeom[] }) | null
      // Not mounted or not upgraded yet — try again next frame.
      if (!comp?.pinInfo) {
        needRetry = true
        continue
      }
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
    this.pinsById = next
    this.sizesById = sizes
    if (needRetry && this.pinsRetries++ < 120) {
      requestAnimationFrame(() => this.pinsVersion++)
    }
  }

  /**
   * A part's pins, measured on demand if the measuring pass has not reached it
   * yet. Dropping a part the instant it appears used to snap against an empty
   * pin list, which put it on the free grid instead of in a hole.
   *
   * Deliberately does not write reactive state: this can be called from a
   * template (`pinPoint`), where a state mutation is forbidden. The size goes
   * into the plain `partSizes` cache, which is what rotation reads.
   */
  private pinsOf(inst: Instance): PinGeom[] {
    const known = this.pinsById[inst.id]
    if (known?.length) return known
    const el = this.canvasEl?.querySelector(`.instance[data-id="${inst.id}"]`) as HTMLElement | null
    const comp = el?.firstElementChild as (HTMLElement & { pinInfo?: PinGeom[] }) | null
    const pins = comp?.pinInfo
    if (!pins?.length) return []
    const measured = pins.map((p) => ({
      name: p.name,
      x: p.x,
      y: p.y,
      description: p.description,
    }))
    if (el) {
      const size = this.sizesById[inst.id]
      if (!size?.w || !size?.h) {
        this.partSizes.set(this.baseKey(inst), { w: el.offsetWidth, h: el.offsetHeight })
      }
    }
    return measured
  }

  // ---- geometry ----------------------------------------------------------

  /** A pin's position in canvas space, accounting for the part's rotation. */
  pinCanvasPoint(inst: Instance, pin: PinGeom): Point {
    const rot = inst.rotate ?? 0
    if (!rot) return { x: inst.x + pin.x, y: inst.y + pin.y }
    // Size only — never a state-dependent lookup. See baseKey.
    const { w, h } = this.partSizeOf(inst)
    const rad = (rot * Math.PI) / 180
    const c = Math.cos(rad)
    const s = Math.sin(rad)
    const dx = pin.x - w / 2
    const dy = pin.y - h / 2
    return { x: inst.x + w / 2 + dx * c - dy * s, y: inst.y + h / 2 + dx * s + dy * c }
  }

  pinPoint(instances: Instance[], ref: PinRef): Point | null {
    const inst = instances.find((i) => i.id === ref.instanceId)
    if (!inst) return null
    const pin = this.pinsOf(inst).find((p) => p.name === ref.pin)
    if (!pin) return null
    return this.pinCanvasPoint(inst, pin)
  }

  /**
   * The pin nearest a canvas-space point, optionally restricted to one part. The
   * restriction matters: with a component sitting on a breadboard, the nearest pin
   * to your press is nearly always a hole *underneath*.
   */
  nearestPin(instances: Instance[], cx: number, cy: number, radius: number, only?: string): PinRef | null {
    let best: PinRef | null = null
    let bestDist = radius * radius
    for (const inst of instances) {
      if (only && inst.id !== only) continue
      for (const p of this.pinsOf(inst)) {
        const pt = this.pinCanvasPoint(inst, p)
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

  /** A part's pin offsets from its own origin, rotation included. */
  pinOffsets(inst: Instance): Point[] {
    const pins = this.pinsOf(inst)
    if (!pins.length) return []
    return pins.map((p) => {
      const at = this.pinCanvasPoint(inst, p)
      return { x: at.x - inst.x, y: at.y - inst.y }
    })
  }

  /** Every breadboard hole, in canvas space. Measured on demand like the rest. */
  boardHolePoints(instances: Instance[]): Point[] {
    const holes: Point[] = []
    for (const board of instances) {
      if (!isBreadboard(board.kind)) continue
      for (const hole of this.pinsOf(board)) {
        holes.push(this.pinCanvasPoint(board, hole))
      }
    }
    return holes
  }

  /**
   * A kind's pin offsets from the palette's mounted element. Measured on demand:
   * the palette's art pass is asynchronous, and a part dropped before it finished
   * must still know where its legs are.
   */
  pinOffsetsFor(kind: PartKind): Point[] {
    const cached = this.kindPins.get(kind)
    if (cached?.length) return cached.map((p) => ({ x: p.x, y: p.y }))
    const host = this.partsListEl?.querySelector(
      `.part[data-kind="${kind}"] .part__art > *:first-child`,
    ) as (HTMLElement & { pinInfo?: PinGeom[] }) | null
    const pins = host?.pinInfo
    if (!pins?.length) return []
    this.kindPins.set(
      kind,
      pins.map((p) => ({ name: p.name, x: p.x, y: p.y })),
    )
    return pins.map((p) => ({ x: p.x, y: p.y }))
  }

  /** Where a dropped part should sit: in a hole if reachable, else on the grid. */
  place(kind: PartKind, offsets: Point[], rawX: number, rawY: number, instances: Instance[]): Point {
    const raw = { x: rawX, y: rawY }
    // A board never snaps into another board: it has hundreds of holes of its own.
    const onHoles = isBreadboard(kind)
      ? null
      : snapToHoles(raw, offsets, this.boardHolePoints(instances), SNAP)
    if (onHoles) return onHoles
    return { x: snapToGrid(raw.x), y: snapToGrid(raw.y) }
  }

  /** The box a part rotates about. Deliberately free of simulation state. */
  partSizeOf(inst: Instance): { w: number; h: number } {
    const cached = this.partSizes.get(this.baseKey(inst))
    if (cached) return cached
    const dom = this.sizesById[inst.id]
    return { w: dom?.w ?? 0, h: dom?.h ?? 0 }
  }

  // ---- sprite cache ------------------------------------------------------

  /**
   * The key a part's sprite is cached under when only its *drawing* matters —
   * what the simulation changes, not how big the part is.
   *
   * This split matters: `pinCanvasPoint` runs inside the netlist effect, and a
   * rotated part needs its sprite size. If sizes were keyed by simulation state,
   * that read would re-trigger the netlist effect forever.
   */
  private baseKey(inst: Instance): string {
    return `${partByKind(inst.kind).tag}|${JSON.stringify(attrsOf(inst))}`
  }

  private partKey(inst: Instance): string {
    const state = this.state()
    const chip = inst.kind === 'uno' && state.pin13 ? 1 : 0
    const lit = inst.kind === 'led' ? LED_ORDER[state.ledStates[inst.id] ?? 'off'] : 0
    const down = inst.kind === 'pushbutton' && state.pressedButtons[inst.id] ? 1 : 0
    return `${this.baseKey(inst)}|${chip}${lit}${down}`
  }

  /** Rasterise a part's element (its shadow-root contents) into an image we blit. */
  private rasterizePart(inst: Instance, waited = false) {
    const key = this.partKey(inst)
    if (this.partImages.has(key) || this.pendingParts.has(key)) return
    const host = this.canvasEl?.querySelector(
      `.instance[data-id="${inst.id}"] > *:first-child`,
    ) as HTMLElement | null
    // Lit re-renders asynchronously — wait once before snapshotting, or we capture
    // (and cache) the *previous* drawing. `updateComplete` is a reused promise, so
    // the wait is one-shot.
    if (!waited) {
      const ready = (host as unknown as { updateComplete?: Promise<unknown> } | null)
        ?.updateComplete
      if (ready && typeof ready.then === 'function') {
        this.pendingParts.add(key)
        const again = () => {
          this.pendingParts.delete(key)
          this.rasterizePart(inst, true)
        }
        ready.then(again, again)
        return
      }
    }
    // The state can move on while we wait (a press that turns into a drag).
    if (this.partKey(inst) !== key) return

    const w = host?.offsetWidth ?? 0
    const h = host?.offsetHeight ?? 0
    if (!host || !host.shadowRoot || !w || !h) {
      const tries = (this.partRetries.get(key) ?? 0) + 1
      if (tries <= 120) {
        this.partRetries.set(key, tries)
        requestAnimationFrame(() => (this.partImagesVersion += 1))
      }
      return
    }
    let markup: string | null
    try {
      markup = shadowSvgMarkup(host)
    } catch (err) {
      this.renderError = `${key}: ${String(err)}`
      return
    }
    if (!markup) return
    this.pendingParts.add(key)
    svgMarkupToImage(markup, { width: w, height: h })
      .then((art) => {
        URL.revokeObjectURL(art.href)
        this.partImages.set(key, { image: art.image, w: art.width, h: art.height })
        this.partSizes.set(this.baseKey(inst), { w: art.width, h: art.height })
        this.pendingParts.delete(key)
        this.renderError = ''
        this.partImagesVersion++
      })
      .catch((err) => {
        this.pendingParts.delete(key)
        this.renderError = `${key}: ${String(err)}`
      })
  }

  // ---- palette thumbnails ------------------------------------------------

  /** Rasterise each palette tile's real drawing, and remember its pin offsets. */
  buildPaletteArt(attempt = 0) {
    const tiles = Array.from(this.partsListEl?.querySelectorAll<HTMLElement>('.part[data-kind]') ?? [])
    let waitingForLayout = false
    for (const tile of tiles) {
      const kind = tile.dataset.kind as PartKind | undefined
      if (!kind) continue
      const host = tile.querySelector('.part__art > *:first-child') as
        | (HTMLElement & { pinInfo?: PinGeom[] })
        | null
      // Remember this kind's pin offsets even when its thumbnail is already done.
      if (host?.pinInfo?.length && !this.kindPins.has(kind)) {
        this.kindPins.set(
          kind,
          host.pinInfo.map((p) => ({ name: p.name, x: p.x, y: p.y })),
        )
      }
      if (this.partArt[kind] || this.artPending.has(kind)) continue
      let markup: string | null = null
      try {
        markup = host ? shadowSvgMarkup(host) : null
      } catch {
        markup = null
      }
      if (!markup) {
        waitingForLayout = true
        continue
      }
      this.artPending.add(kind)
      svgMarkupToImage(markup)
        .then((art) => {
          this.artPending.delete(kind)
          // The object URL is deliberately kept alive: the <img> shows it.
          this.partArt = {
            ...this.partArt,
            [kind]: { href: art.href, width: art.width, height: art.height },
          }
        })
        .catch(() => this.artPending.delete(kind))
    }
    if (waitingForLayout && attempt < 30) {
      requestAnimationFrame(() => this.buildPaletteArt(attempt + 1))
    }
  }

  // ---- drawing -----------------------------------------------------------

  private wireColour(name?: string): string {
    return name ? (WIRE_HEX.get(name) ?? name) : this.palette.wire
  }

  /** Draw the whole circuit. The caller owns the reactive reads (`Scene`). */
  draw(scene: Scene) {
    const c = this.renderCanvas
    const host = this.canvasEl
    if (!c || !host) return
    const ctx = c.getContext('2d')
    if (!ctx) return
    const dpr = window.devicePixelRatio || 1
    const rect = host.getBoundingClientRect()
    const view = {
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      dpr,
      zoom: scene.zoom,
      panX: scene.panX,
      panY: scene.panY,
    }
    if (c.width !== view.width * dpr || c.height !== view.height * dpr) {
      c.width = view.width * dpr
      c.height = view.height * dpr
    }
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0)
    clear(ctx, view, this.palette)
    drawGrid(ctx, view, this.palette, scene.zoom >= 0.67)
    const t = makeTransform(view)
    const state = this.state()

    // Each part is drawn with its own pins, so a part on top hides the pins of
    // the parts beneath it; wires go over everything (like Wokwi).
    for (const inst of renderOrderOf(scene.instances)) {
      const sprite = this.partImages.get(this.partKey(inst))
      if (sprite) {
        const p = t.toScreen({ x: inst.x, y: inst.y })
        const pw = sprite.w * scene.zoom
        const ph = sprite.h * scene.zoom
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
        this.rasterizePart(inst)
      }
      // A marker only for the pin under the pointer: marking every pin reads as
      // static on a breadboard and is clutter on anything else.
      for (const pin of this.pinsById[inst.id] ?? []) {
        const hovered = scene.hoveredPin?.instanceId === inst.id && scene.hoveredPin.pin === pin.name
        if (!hovered) continue
        drawSquare(
          ctx,
          t.toScreen(this.pinCanvasPoint(inst, pin)),
          10 * scene.zoom,
          this.palette.accent,
          this.palette.bg,
        )
      }
      if (state.warnings[inst.id]) this.drawWarning(ctx, t, inst, scene.zoom)
    }

    for (const w of scene.wires) {
      const a = this.pinPoint(scene.instances, w.a)
      const b = this.pinPoint(scene.instances, w.b)
      if (!a || !b) continue
      const pts = expandPath([a, ...w.waypoints, b]).map(t.toScreen)
      drawPolyline(
        ctx,
        pts,
        this.wireColour(w.colour),
        (scene.selectedWireId === w.id ? 4 : 2.5) * scene.zoom,
      )
    }

    // A ring for every selected part, not only the one the strip is editing.
    for (const id of scene.selectedIds) {
      const inst = scene.instances.find((i) => i.id === id)
      if (!inst) continue
      const p = t.toScreen({ x: inst.x, y: inst.y })
      const sprite = this.partImages.get(this.partKey(inst))
      const el = host.querySelector(`.instance[data-id="${inst.id}"]`) as HTMLElement | null
      const sw = (sprite?.w ?? el?.offsetWidth ?? 0) * scene.zoom
      const sh = (sprite?.h ?? el?.offsetHeight ?? 0) * scene.zoom
      ctx.save()
      ctx.translate(p.x + sw / 2, p.y + sh / 2)
      if (inst.rotate) ctx.rotate((inst.rotate * Math.PI) / 180)
      ctx.strokeStyle = this.palette.accent
      ctx.lineWidth = 2 * scene.zoom
      ctx.strokeRect(-sw / 2 - 3 * scene.zoom, -sh / 2 - 3 * scene.zoom, sw + 6 * scene.zoom, sh + 6 * scene.zoom)
      ctx.restore()
    }

    if (scene.wiring) {
      drawPolyline(
        ctx,
        expandPath(scene.wiringPoints).map(t.toScreen),
        this.palette.wire,
        2.5 * scene.zoom,
        [4, 4],
      )
      const snap = scene.wiringTarget
        ? this.pinPoint(scene.instances, scene.wiringTarget)
        : null
      if (snap) drawDisc(ctx, t.toScreen(snap), 7 * scene.zoom, '', this.palette.accent)
    }

    if (scene.selectedWireId) {
      const w = scene.wires.find((x) => x.id === scene.selectedWireId)
      if (w) {
        for (const ref of [w.a, w.b]) {
          const p = this.pinPoint(scene.instances, ref)
          if (p) drawDisc(ctx, t.toScreen(p), 5 * scene.zoom, this.palette.accent, this.palette.bg)
        }
      }
    }
  }

  /** A miswired part gets a badge on the canvas. */
  private drawWarning(
    ctx: CanvasRenderingContext2D,
    t: ReturnType<typeof makeTransform>,
    inst: Instance,
    zoom: number,
  ) {
    const size = this.sizesById[inst.id] ?? this.partImages.get(this.partKey(inst))
    const at = t.toScreen({ x: inst.x + (size?.w ?? 0) - 2, y: inst.y + 2 })
    drawDisc(ctx, at, 6 * zoom, this.palette.accent, this.palette.bg)
    ctx.save()
    ctx.fillStyle = this.palette.bg
    ctx.font = `bold ${7 * zoom}px sans-serif`
    ctx.textAlign = 'center'
    ctx.textBaseline = 'middle'
    ctx.fillText('!', at.x, at.y)
    ctx.restore()
  }
}
