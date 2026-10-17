/**
 * Orthogonal wire routing — the geometry behind "never a diagonal".
 *
 * Every route in the app is a polyline whose segments are horizontal or vertical, and
 * this is the only place that decides where the bends go. It is pure and lives here
 * rather than in the component so the promise can be tested rather than asserted: see
 * `route.test.ts`, which checks that no input produces a diagonal.
 */

export type Point = { x: number; y: number }

/**
 * Which corner to turn through, alternating by leg index so a route zig-zags rather
 * than stacking every bend the same way. Deriving the corner from the index (instead
 * of storing it) is what lets a wire be saved as mere relative moves.
 */
export function elbowFor(i: number, a: Point, b: Point): Point {
  return i % 2 === 0 ? { x: a.x, y: b.y } : { x: b.x, y: a.y }
}

/** The one bend of the current (live) leg, or null if the two points already line up on an axis. */
export function legBend(i: number, last: Point, end: Point): Point | null {
  if (last.x === end.x || last.y === end.y) return null
  return elbowFor(i, last, end)
}

/** The wire so far, plus the live leg ending at `end`. */
export function livePoints(from: Point, waypoints: Point[], end: Point): Point[] {
  const last = waypoints.length ? waypoints[waypoints.length - 1] : from
  const bend = legBend(waypoints.length, last, end)
  return bend ? [from, ...waypoints, bend, end] : [from, ...waypoints, end]
}

/** The full orthogonal polyline through `pts`, inserting a bend between any two points
 *  that don't already share a row or a column. */
export function expandPath(pts: Point[]): Point[] {
  if (pts.length < 2) return []
  const out: Point[] = [pts[0]]
  for (let i = 0; i < pts.length - 1; i++) {
    const a = pts[i]
    const b = pts[i + 1]
    if (a.x === b.x || a.y === b.y) {
      out.push(b)
    } else {
      const c = elbowFor(i, a, b)
      out.push(c, b)
    }
  }
  return out
}

/** Route through a list of points, always at right angles, never a diagonal. */
export function routePath(pts: Point[]): string {
  const line = expandPath(pts)
  if (line.length < 2) return ''
  return 'M ' + line.map((p) => `${p.x} ${p.y}`).join(' L ')
}

/**
 * Wokwi's route mini-language: relative "v<px>"/"h<px>" moves from a pin. Storing moves
 * rather than absolute corners means a saved wire stays sensible when its part moves.
 */
export function encodeRoute(a: Point, b: Point, waypoints: Point[]): string[] {
  const line = expandPath([a, ...waypoints, b])
  const round = (n: number) => Math.round(n * 10) / 10
  const out: string[] = []
  for (let i = 0; i < line.length - 1; i++) {
    const dx = line[i + 1].x - line[i].x
    const dy = line[i + 1].y - line[i].y
    if (dx === 0 && dy === 0) continue
    if (dx === 0) out.push(`v${round(dy)}`)
    else if (dy === 0) out.push(`h${round(dx)}`)
  }
  return out
}

export function decodeRoute(from: Point, route: string[]): Point[] {
  const pts: Point[] = []
  let x = from.x
  let y = from.y
  for (const ins of route) {
    const m = /^([vh])(-?[0-9.]+)$/.exec(ins)
    if (!m) continue
    const n = Number(m[2])
    if (m[1] === 'v') y += n
    else x += n
    pts.push({ x, y })
  }
  pts.pop() // the last move lands on the target pin; the rest are waypoints
  return pts
}
