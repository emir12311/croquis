import { describe, expect, it } from 'vitest'
import { decodeRoute, encodeRoute, expandPath, routePath, type Point } from './route'

const P = (x: number, y: number): Point => ({ x, y })
const segments = (pts: Point[]) => pts.slice(1).map((p, i) => [p.x - pts[i].x, p.y - pts[i].y])

describe('orthogonal routing', () => {
  it('never produces a diagonal, whatever it is handed', () => {
    // The design promise, as a test rather than a claim in a README.
    const cases: Point[][] = [
      [P(0, 0), P(50, 50)],
      [P(0, 0), P(10, 90), P(90, 10)],
      [P(0, 0), P(40, 40), P(80, 0), P(120, 40)],
      [P(185, 89), P(185, 205.65), P(430, 205.65)],
      [P(0, 0), P(0, 40), P(60, 40), P(60, 0)],
    ]
    for (const pts of cases) {
      for (const [dx, dy] of segments(expandPath(pts))) {
        expect(dx === 0 || dy === 0, `diagonal in ${JSON.stringify(pts)}`).toBe(true)
      }
    }
  })

  it('leaves a route that already runs on the axes alone', () => {
    const straight = [P(0, 0), P(0, 50), P(90, 50)]
    expect(expandPath(straight)).toEqual(straight)
  })

  it('adds exactly one bend between two points sharing no axis, and alternates the corner', () => {
    expect(expandPath([P(0, 0), P(50, 50)])).toEqual([P(0, 0), P(0, 50), P(50, 50)])
    // The second leg turns the other way, so a wire can fold back rather than spiral.
    expect(expandPath([P(0, 0), P(50, 50), P(90, 10)])).toEqual([
      P(0, 0),
      P(0, 50),
      P(50, 50),
      P(90, 50),
      P(90, 10),
    ])
  })

  it('writes a polyline with no diagonals in it', () => {
    expect(routePath([P(0, 0), P(50, 50)])).toBe('M 0 0 L 0 50 L 50 50')
    expect(routePath([P(5, 5)])).toBe('')
  })

  it('survives a trip through the saved mini-language', () => {
    // Routes are stored as relative moves from the pin, so a wire has to come back.
    const from = P(185, 89)
    const to = P(605, 212)
    const waypoints = [P(185, 205.6), P(488.8, 205.6)]
    const encoded = encodeRoute(from, to, waypoints)
    expect(encoded.length).toBeGreaterThan(0)
    for (const move of encoded) expect(move, move).toMatch(/^[vh]-?[0-9.]+$/)

    const expected = expandPath([from, ...waypoints, to]).slice(1, -1)
    const decoded = decodeRoute(from, encoded)
    expect(decoded).toHaveLength(expected.length)
    decoded.forEach((p, i) => {
      expect(p.x).toBeCloseTo(expected[i].x, 1)
      expect(p.y).toBeCloseTo(expected[i].y, 1)
    })
  })

  it('decodes to the points between the ends, which is the bend a plain wire needs', () => {
    // Not "the waypoints you set": the stored form is the whole expanded line, so what
    // comes back is every point between the pins. A wire straight across needs one bend.
    expect(decodeRoute(P(10, 10), encodeRoute(P(10, 10), P(90, 60), []))).toEqual([P(10, 60)])
  })
})
