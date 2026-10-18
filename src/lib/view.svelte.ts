/**
 * The canvas viewport: zoom, pan, and the transformations between screen and
 * circuit coordinates.
 *
 * Pure geometry plus reactive `$state`, with the canvas rectangle passed in — so
 * it does not need to know about a DOM node, and the maths can be reasoned about
 * (and driven) on its own.
 */
import type { Instance, Point } from './document'

export const ZOOM_MIN = 0.25
export const ZOOM_MAX = 4

export class View {
  zoom = $state(1)
  panX = $state(0)
  panY = $state(0)
  /** Set once, when the canvas first exists, so panning is never fought. */
  centred = $state(false)

  toCanvas(clientX: number, clientY: number, rect: DOMRect): Point {
    return {
      x: (clientX - rect.left - this.panX) / this.zoom,
      y: (clientY - rect.top - this.panY) / this.zoom,
    }
  }

  overCanvas(clientX: number, clientY: number, rect: DOMRect): boolean {
    return clientX >= rect.left && clientX <= rect.right && clientY >= rect.top && clientY <= rect.bottom
  }

  panTo(x: number, y: number) {
    this.panX = x
    this.panY = y
  }

  /** Zoom toward a screen point, keeping that point under the pointer. */
  zoomAt(clientX: number, clientY: number, factor: number, rect: DOMRect) {
    const mx = clientX - rect.left
    const my = clientY - rect.top
    const cx = (mx - this.panX) / this.zoom
    const cy = (my - this.panY) / this.zoom
    const nz = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, this.zoom * factor))
    this.panX = mx - cx * nz
    this.panY = my - cy * nz
    this.zoom = nz
  }

  zoomIn(rect: DOMRect) {
    this.zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, 1.2, rect)
  }

  zoomOut(rect: DOMRect) {
    this.zoomAt(rect.left + rect.width / 2, rect.top + rect.height / 2, 1 / 1.2, rect)
  }

  /** Put the world origin in the middle of the canvas rather than in its corner. */
  centre(rect: DOMRect) {
    this.panX = Math.round(rect.width / 2)
    this.panY = Math.round(rect.height / 2)
  }

  reset(rect: DOMRect) {
    this.zoom = 1
    this.centre(rect)
  }

  /** Frame every part, with a margin, without exceeding the zoom limits. */
  fit(instances: Instance[], sizes: Record<string, { w: number; h: number }>, rect: DOMRect) {
    if (!instances.length) return
    const pad = 48
    let minX = Infinity
    let minY = Infinity
    let maxX = -Infinity
    let maxY = -Infinity
    for (const inst of instances) {
      const size = sizes[inst.id] ?? { w: 0, h: 0 }
      minX = Math.min(minX, inst.x)
      minY = Math.min(minY, inst.y)
      maxX = Math.max(maxX, inst.x + size.w)
      maxY = Math.max(maxY, inst.y + size.h)
    }
    const bw = Math.max(1, maxX - minX)
    const bh = Math.max(1, maxY - minY)
    const zoom = Math.min(
      ZOOM_MAX,
      Math.max(ZOOM_MIN, Math.min((rect.width - pad * 2) / bw, (rect.height - pad * 2) / bh)),
    )
    this.zoom = zoom
    this.panX = (rect.width - bw * zoom) / 2 - minX * zoom
    this.panY = (rect.height - bh * zoom) / 2 - minY * zoom
  }
}
