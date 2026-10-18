import { describe, expect, it } from 'vitest'
import { View } from './view.svelte'

const rect = (width = 800, height = 600): DOMRect =>
  ({
    x: 0,
    y: 0,
    left: 0,
    top: 0,
    width,
    height,
    right: width,
    bottom: height,
    toJSON: () => ({}),
  }) as DOMRect

describe('View', () => {
  it('maps a screen point through the pan and zoom', () => {
    const view = new View()
    view.zoom = 2
    view.panTo(100, 50)
    expect(view.toCanvas(140, 90, rect())).toEqual({ x: 20, y: 20 })
  })

  it('zooms toward the pointer, keeping that point still', () => {
    const view = new View()
    view.centre(rect())
    const before = view.toCanvas(600, 200, rect())
    view.zoomAt(600, 200, 2, rect())
    const after = view.toCanvas(600, 200, rect())
    expect(view.zoom).toBe(2)
    expect(after.x).toBeCloseTo(before.x)
    expect(after.y).toBeCloseTo(before.y)
  })

  it('clamps the zoom to its limits', () => {
    const view = new View()
    view.zoomAt(400, 300, 100, rect())
    expect(view.zoom).toBe(4)
    view.zoomAt(400, 300, 0.001, rect())
    expect(view.zoom).toBe(0.25)
  })

  it('centres the world origin, and reset restores zoom and centre', () => {
    const view = new View()
    view.centre(rect(1000, 700))
    expect([view.panX, view.panY]).toEqual([500, 350])
    view.zoom = 3
    view.reset(rect(1000, 700))
    expect(view.zoom).toBe(1)
    expect([view.panX, view.panY]).toEqual([500, 350])
  })

  it('frames every part with a margin inside the canvas', () => {
    const view = new View()
    const instances = [
      { id: 'a', kind: 'led' as const, x: 100, y: 50 },
      { id: 'b', kind: 'resistor' as const, x: 400, y: 300 },
    ]
    view.fit(instances, { a: { w: 200, h: 100 }, b: { w: 100, h: 40 } }, rect(800, 600))
    for (const inst of instances) {
      const size = inst.id === 'a' ? { w: 200, h: 100 } : { w: 100, h: 40 }
      const left = view.panX + inst.x * view.zoom
      const top = view.panY + inst.y * view.zoom
      expect(left).toBeGreaterThanOrEqual(47)
      expect(top).toBeGreaterThanOrEqual(47)
      expect(left + size.w * view.zoom).toBeLessThanOrEqual(753)
      expect(top + size.h * view.zoom).toBeLessThanOrEqual(553)
    }
  })

  it('does nothing when there is nothing to frame', () => {
    const view = new View()
    view.zoom = 2
    view.fit([], {}, rect())
    expect(view.zoom).toBe(2)
  })
})
