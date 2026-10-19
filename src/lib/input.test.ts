import { describe, expect, it } from 'vitest'
import { knobValue } from './input.svelte'

/**
 * The knob maths is the element's own `rotateHandler`, so these are the values
 * the element itself would produce for an upright part. They are also the ones
 * the live drive run lands on.
 */
const centre = { x: 100, y: 100 }
const at = (dx: number, dy: number, rotate = 0) =>
  knobValue(centre, { x: centre.x + dx, y: centre.y + dy }, rotate)

describe('knobValue', () => {
  it('puts the middle of the track straight up', () => {
    expect(at(0, -10)).toBe(512)
  })

  it('turns clockwise through the GND-to-VCC sweep', () => {
    expect(at(10, 0)).toBe(853)
    expect(at(0, 10)).toBe(1023)
    expect(at(-10, 0)).toBe(171)
  })

  it('clamps at the end stops', () => {
    expect(at(1, 0.1)).toBeGreaterThanOrEqual(0)
    expect(at(1, 0.1)).toBeLessThanOrEqual(1023)
    expect(at(-1, -0.1)).toBeGreaterThanOrEqual(0)
    expect(at(-1, -0.1)).toBeLessThanOrEqual(1023)
  })

  it('ignores a pointer at the centre', () => {
    expect(at(0, 0)).toBeNull()
  })

  it('undoes the part rotation before reading the angle', () => {
    // A part turned 90° clockwise: the screen's right is the element's up.
    expect(at(10, 0, 90)).toBe(512)
    expect(at(0, -10, 90)).toBe(171)
  })
})
