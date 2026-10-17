/**
 * Snapping a part into a breadboard's holes.
 *
 * Kept here, out of the component, for one reason: the bugs in this were all in
 * the *geometry*, and none of it was testable while it sat inside the drag handler.
 *
 * Two rules matter:
 *
 * 1. The input is the **raw** position. Quantising to the free grid first moves a
 *    part up to half a grid step away, which can put every hole out of reach — so
 *    some columns were unusable, and which ones moved when either grid changed.
 * 2. A placement only counts if **every** leg is in a hole. Aligning the nearest
 *    single leg is not enough: a pushbutton's sides are 19px apart and the gap
 *    across the trench is 19.14px, so a placement two rows too high puts its lower
 *    side *in the trench*, where there is no hole — one side seated, one side out.
 */

export interface Point {
  x: number
  y: number
}

export interface SnapOptions {
  /** How far a part is willing to move to find a placement. */
  reach: number
  /** How close a leg has to be to a hole to count as being in it. */
  seat: number
}

/**
 * Where a part should sit so every one of its legs is in a hole, or `null` when
 * nothing that good is within reach — the caller's cue to fall back to the free
 * grid.
 *
 * `offsets` are the part's pins relative to its own origin (rotation included),
 * `holes` the board's holes, both in canvas space.
 */
export function snapToHoles(
  raw: Point,
  offsets: Point[],
  holes: Point[],
  options: SnapOptions,
): Point | null {
  if (!offsets.length || !holes.length) return null
  const { reach, seat } = options
  const reachSq = reach * reach
  const seatSq = seat * seat
  // A part can't sit off the top-left of the canvas, so work from a clamped base.
  const raw0 = { x: Math.max(0, raw.x), y: Math.max(0, raw.y) }

  // Bucket the holes so "is there one under this leg?" is a few compares. It gets
  // asked once per leg per candidate placement, and a board has hundreds of holes.
  const cell = seat * 2
  const byCell = new Map<string, Point[]>()
  for (const hole of holes) {
    const k = `${Math.floor(hole.x / cell)},${Math.floor(hole.y / cell)}`
    const list = byCell.get(k)
    if (list) list.push(hole)
    else byCell.set(k, [hole])
  }
  const isSeated = (p: Point) => {
    const cx = Math.floor(p.x / cell)
    const cy = Math.floor(p.y / cell)
    for (let i = -1; i <= 1; i++) {
      for (let j = -1; j <= 1; j++) {
        for (const hole of byCell.get(`${cx + i},${cy + j}`) ?? []) {
          const dx = hole.x - p.x
          const dy = hole.y - p.y
          if (dx * dx + dy * dy <= seatSq) return true
        }
      }
    }
    return false
  }
  const allSeated = (pos: Point) =>
    offsets.every((o) => isSeated({ x: pos.x + o.x, y: pos.y + o.y }))

  let bestSeated: { x: number; y: number; d: number } | null = null
  let bestPartial: { x: number; y: number; d: number } | null = null
  for (const hole of holes) {
    for (const offset of offsets) {
      const dx = hole.x - (raw0.x + offset.x)
      const dy = hole.y - (raw0.y + offset.y)
      const d = dx * dx + dy * dy
      if (d > reachSq) continue
      const pos = { x: raw0.x + dx, y: raw0.y + dy }
      if (pos.x < 0 || pos.y < 0) continue
      if (!bestPartial || d < bestPartial.d) bestPartial = { ...pos, d }
      if (!allSeated(pos)) continue
      // Already exactly where it belongs: nothing beats that, and no point looking.
      if (d < 0.0001) return pos
      if (!bestSeated || d < bestSeated.d) bestSeated = { ...pos, d }
    }
  }
  // Fully seated wherever possible; a single leg in a hole beats the free grid.
  const best = bestSeated ?? bestPartial
  return best ? { x: best.x, y: best.y } : null
}
