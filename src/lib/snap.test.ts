import { describe, expect, it } from 'vitest'
import { BREADBOARDS, BREADBOARD_HOLE_R, BREADBOARD_PITCH } from './breadboard'
import { snapToHoles, type Point } from './snap'

const half = BREADBOARDS.half
/** Every strip hole, both banks, in canvas space. */
const holes = half.pins.filter((p) => /^[a-j]\d+$/.test(p.name)).map((p) => ({ x: p.x, y: p.y }))
const snapOptions = { reach: BREADBOARD_PITCH * 1.1, seat: BREADBOARD_HOLE_R }

/**
 * The insertable parts' legs, in the elements' own pixel space — taken from their
 * `pinInfo`. Note they share no grid: 10, 58.8 and 67.
 */
const parts = [
  { name: 'LED', offsets: [{ x: 25, y: 42 }, { x: 15, y: 42 }] },
  { name: 'resistor', offsets: [{ x: 0, y: 5.65 }, { x: 58.8, y: 5.65 }] },
  {
    name: 'pushbutton',
    offsets: [{ x: 0, y: 13 }, { x: 0, y: 32 }, { x: 67, y: 13 }, { x: 67, y: 32 }],
  },
]

/** Shift a part's offsets so it extends right and down from its top-left leg. */
function normalised(offsets: Point[]): { offsets: Point[]; anchor: Point } {
  const minX = Math.min(...offsets.map((o) => o.x))
  const minY = Math.min(...offsets.map((o) => o.y))
  const out = offsets.map((o) => ({ x: o.x - minX, y: o.y - minY }))
  return { offsets: out, anchor: out.find((o) => o.x === 0 && o.y === 0) ?? out[0] }
}

/** How far the worst leg of a snapped part is from the nearest hole. */
function worstMiss(snapped: Point, offsets: Point[]): number {
  return Math.max(
    ...offsets.map((o) =>
      Math.min(...holes.map((h) => Math.hypot(h.x - (snapped.x + o.x), h.y - (snapped.y + o.y)))),
    ),
  )
}

describe('snapping a part into the board', () => {
  /**
   * Rotation is rigid, so the *spacings* between a part's legs are what matter,
   * not the centre it turns about. Rotating the offsets about the origin gives the
   * same geometry the app computes, which is why this can be tested here.
   */
  const turns = [
    { name: 'unrotated', turn: (p: Point) => p },
    { name: 'rotated 90°', turn: (p: Point) => ({ x: -p.y, y: p.x }) },
    { name: 'rotated 180°', turn: (p: Point) => ({ x: -p.x, y: -p.y }) },
  ]

  it('seats every leg, from every column, however sloppily the part is dropped', () => {
    // Two bugs in one property. The first: quantising to the free grid *before*
    // looking for holes made some columns unusable, and which ones moved whenever
    // either grid changed. The second: at a 10px pitch a pushbutton's 67px span
    // missed the nearest hole by 3px, outside the 2.1px hole — the one that
    // visibly did not fit, in both orientations.
    for (const part of parts) {
      for (const turn of turns) {
        const { offsets, anchor } = normalised(part.offsets.map(turn.turn))
        const spanCols = Math.round(Math.max(...offsets.map((o) => o.x)) / BREADBOARD_PITCH)
        for (let col = 2; col <= half.columns - spanCols; col++) {
          const target = half.pins.find((p) => p.name === `a${col}`)
          if (!target) throw new Error(`no hole a${col}`)
          for (const dx of [-4, -2, 0, 2, 4]) {
            for (const dy of [-3, 0, 3]) {
              const raw = { x: target.x - anchor.x + dx, y: target.y - anchor.y + dy }
              const snapped = snapToHoles(raw, offsets, holes, snapOptions)
              expect(snapped, `${part.name} ${turn.name} at column ${col}`).not.toBeNull()
              expect(
                worstMiss(snapped!, offsets),
                `${part.name} ${turn.name} at column ${col}`,
              ).toBeLessThanOrEqual(BREADBOARD_HOLE_R)
            }
          }
        }
      }
    }
  })

  it('seats both sides of a two-row part, wherever it sits down the board', () => {
    // The trench is a gap with no holes, and it is 19.14px tall — the same as a
    // pushbutton's two sides are apart. A placement a couple of rows above it put
    // the lower side *in* the trench, where no hole exists: one side seated, the
    // other out. That is what aligning to the nearest single leg produced, and why
    // the button "didn't sit correctly at all, rotated or not".
    const button = parts.find((p) => p.name === 'pushbutton')
    if (!button) throw new Error('pushbutton missing')
    const { offsets } = normalised(button.offsets)
    const spanCols = Math.round(Math.max(...offsets.map((o) => o.x)) / BREADBOARD_PITCH)
    // Every row that leaves room for the part below it, in either bank.
    for (const row of ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h']) {
      for (let col = 2; col <= half.columns - spanCols; col++) {
        const target = half.pins.find((p) => p.name === `${row}${col}`)
        if (!target) throw new Error(`no hole ${row}${col}`)
        for (const dx of [-3, 0, 3]) {
          for (const dy of [-3, 0, 3]) {
            const raw = { x: target.x + dx, y: target.y + dy }
            const snapped = snapToHoles(raw, offsets, holes, snapOptions)
            expect(snapped, `button dropped at ${row}${col}`).not.toBeNull()
            expect(
              worstMiss(snapped!, offsets),
              `button dropped at ${row}${col}`,
            ).toBeLessThanOrEqual(BREADBOARD_HOLE_R)
          }
        }
      }
    }
  })

  it('snaps at negative canvas coordinates', () => {
    // The view is centred on the world origin, so a board dropped there sits at
    // negative coordinates. Clamping the raw position to zero made the origin
    // behave like a wall and pulled parts onto the board from its left.
    const shift = -1000
    const shifted = holes.map((h) => ({ x: h.x + shift, y: h.y + shift }))
    const target = shifted[Math.floor(shifted.length / 2)]
    const raw = { x: target.x - 25, y: target.y - 42 }
    const snapped = snapToHoles(raw, parts[0].offsets, shifted, snapOptions)
    expect(snapped).not.toBeNull()
    const missed = Math.max(
      ...parts[0].offsets.map((o) =>
        Math.min(
          ...shifted.map((h) =>
            Math.hypot(h.x - (snapped!.x + o.x), h.y - (snapped!.y + o.y)),
          ),
        ),
      ),
    )
    expect(missed).toBeLessThanOrEqual(BREADBOARD_HOLE_R)
  })

  it('leaves a part alone when no hole is in reach', () => {
    expect(snapToHoles({ x: 5000, y: 5000 }, parts[0].offsets, holes, snapOptions)).toBeNull()
  })

  it('does not move a part just to seat one leg', () => {
    // A part whose legs cannot all reach holes must stay where it was dropped (the
    // caller falls back to the grid). Seating the nearest single leg was the
    // "LED jumps to a random hole" behaviour.
    const wide = [
      { x: 0, y: 0 },
      { x: 100, y: 0 },
    ]
    const target = holes[0]
    for (const dx of [-4, 0, 4]) {
      for (const dy of [-3, 0, 3]) {
        expect(
          snapToHoles({ x: target.x + dx, y: target.y + dy }, wide, holes, snapOptions),
        ).toBeNull()
      }
    }
  })

  it('does nothing without pins or holes to snap to', () => {
    expect(snapToHoles({ x: 0, y: 0 }, [], holes, snapOptions)).toBeNull()
    expect(snapToHoles({ x: 0, y: 0 }, parts[0].offsets, [], snapOptions)).toBeNull()
  })
})
