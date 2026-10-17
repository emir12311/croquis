import { describe, expect, it } from 'vitest'
import { BREADBOARDS, BREADBOARD_HOLE_R, BREADBOARD_PITCH, breadboardPinNode } from './breadboard'

const half = BREADBOARDS.half
const hole = (name: string) => {
  const pin = half.pins.find((p) => p.name === name)
  if (!pin) throw new Error(`no hole named ${name}`)
  return pin
}

describe('the board is on a grid all its parts can sit on', () => {
  /*
   * The elements share no grid: a wokwi LED's legs are 10px apart, a resistor's
   * 58.8, a pushbutton's 67 across and 19 down. "Exact for everything" doesn't
   * exist, so the real requirement is that every leg lands **inside** a hole
   * (radius 2.1px) rather than beside it. The pitch is picked to satisfy the
   * worst of them — the pushbutton, which is the one that visibly didn't fit at
   * 10px, missing by 3px in both orientations.
   */
  const parts = [
    { name: 'LED', spans: [10] },
    { name: 'resistor', spans: [58.8] },
    { name: 'pushbutton', spans: [67, 19] },
  ]
  /** How far a span misses the nearest hole, at this pitch. */
  const miss = (span: number) =>
    Math.abs(span - Math.round(span / BREADBOARD_PITCH) * BREADBOARD_PITCH)

  it('is spaced so every insertable part seats', () => {
    for (const part of parts) {
      for (const span of part.spans) {
        expect(
          miss(span),
          `${part.name} span ${span}px misses by ${miss(span).toFixed(2)}px`,
        ).toBeLessThanOrEqual(BREADBOARD_HOLE_R)
      }
    }
  })

  it('makes the pushbutton exact, since it was the worst fit', () => {
    expect(miss(67)).toBeLessThan(1e-9)
  })

  it('spaces holes and rows by the pitch', () => {
    expect(hole('a10').x - hole('a9').x).toBeCloseTo(BREADBOARD_PITCH, 9)
    expect(hole('b1').y - hole('a1').y).toBeCloseTo(BREADBOARD_PITCH, 9)
  })

  it('puts the rows either side of the trench exactly two pitches apart', () => {
    // That is what lets a two-row part straddle it: a pushbutton's sides are 19px
    // apart, which is this gap. At the original 13px trench the gap was 22.6px, so
    // both of its sides fell in one strip — the same net, shorted, dead.
    expect(hole('f1').y - hole('e1').y).toBeCloseTo(2 * BREADBOARD_PITCH, 9)
    expect(Math.abs(hole('f1').y - hole('e1').y - 19)).toBeLessThanOrEqual(BREADBOARD_HOLE_R)
  })

  it('sizes both boards from the same generator', () => {
    expect(BREADBOARDS.half.columns).toBe(30)
    expect(BREADBOARDS.full.columns).toBe(63)
    expect(BREADBOARDS.half.pins).toHaveLength(400) // 300 strip + 100 rail
    expect(BREADBOARDS.full.pins).toHaveLength(830) // 630 strip + 200 rail
    expect(BREADBOARDS.full.width).toBeGreaterThan(BREADBOARDS.half.width)
  })
})

describe('holes that are electrically one point', () => {
  it('groups a strip, above and below the trench separately', () => {
    expect(breadboardPinNode('a1')).toBe('top1')
    expect(breadboardPinNode('e1')).toBe('top1') // the same five-hole strip
    expect(breadboardPinNode('f1')).toBe('bottom1') // the trench separates them
    expect(breadboardPinNode('j30')).toBe('bottom30')
  })

  it('groups each rail segment, split on a full board', () => {
    expect(breadboardPinNode('tp3')).toBe('railtp')
    expect(breadboardPinNode('tn25')).toBe('railtn')
    // A full board's rails break in the middle, so the halves are separate nets.
    expect(breadboardPinNode('tpA1')).toBe('railtpA')
    expect(breadboardPinNode('tpB1')).toBe('railtpB')
  })

  it('leaves anything else alone', () => {
    expect(breadboardPinNode('weird')).toBe('weird')
  })
})
