/**
 * The circuit is drawn into a <canvas> rather than as DOM/SVG. On this machine
 * WebKitGTK (with the NVIDIA driver) rasterises painted DOM at reduced
 * resolution, which blurs the parts, grid and wires. A canvas is a bitmap we
 * draw ourselves at full device resolution, so WebKit just blits it — crisp.
 *
 * The DOM still exists (invisibly) to handle hit-testing and interaction.
 */

export interface Point {
  x: number
  y: number
}

export interface View {
  width: number
  height: number
  dpr: number
  zoom: number
  panX: number
  panY: number
}

export interface PartSprite {
  image: HTMLImageElement
  /** natural size in canvas units (the element's own pixel size) */
  w: number
  h: number
}

export interface DrawItem {
  x: number
  y: number
  rotate: number
  key: string
}

/** CSS variables we need in canvas form (canvas can't use var()). */
export interface Palette {
  bg: string
  border: string
  wire: string
  accent: string
  error: string
}

export function readPalette(): Palette {
  const cs = getComputedStyle(document.documentElement)
  const v = (name: string, fallback: string) => cs.getPropertyValue(name).trim() || fallback
  return {
    bg: v('--bg', '#1a1a1c'),
    border: v('--border', '#35353a'),
    wire: v('--wire', '#6ccb7b'),
    accent: v('--accent', '#d9a441'),
    error: v('--error', '#e5484d'),
  }
}

/** World (canvas-content) coordinates → screen pixels within the canvas. */
export function makeTransform(view: View) {
  return {
    toScreen(p: Point): Point {
      return { x: p.x * view.zoom + view.panX, y: p.y * view.zoom + view.panY }
    },
  }
}

export function clear(ctx: CanvasRenderingContext2D, view: View, palette: Palette) {
  ctx.setTransform(1, 0, 0, 1, 0, 0)
  ctx.fillStyle = palette.bg
  ctx.fillRect(0, 0, view.width, view.height)
}

/** The work-surface grid, in world units so it lines up with snapping. */
export function drawGrid(
  ctx: CanvasRenderingContext2D,
  view: View,
  palette: Palette,
  fine: boolean,
) {
  const { zoom, panX, panY, width, height } = view
  const line = (size: number, alpha: number) => {
    const step = size * zoom
    if (step < 6) return
    ctx.globalAlpha = alpha
    ctx.strokeStyle = palette.border
    ctx.lineWidth = 1
    ctx.beginPath()
    for (let x = panX % step; x <= width; x += step) {
      ctx.moveTo(Math.round(x) + 0.5, 0)
      ctx.lineTo(Math.round(x) + 0.5, height)
    }
    for (let y = panY % step; y <= height; y += step) {
      ctx.moveTo(0, Math.round(y) + 0.5)
      ctx.lineTo(width, Math.round(y) + 0.5)
    }
    ctx.stroke()
    ctx.globalAlpha = 1
  }
  line(24, 0.3)
  if (fine) line(12, 0.13)
}

/** Stroke an orthogonal polyline (screen coordinates). */
export function drawPolyline(
  ctx: CanvasRenderingContext2D,
  points: Point[],
  colour: string,
  width: number,
  dash: number[] = [],
) {
  if (points.length < 2) return
  ctx.save()
  ctx.strokeStyle = colour
  ctx.lineWidth = width
  ctx.lineJoin = 'round'
  ctx.lineCap = 'round'
  ctx.setLineDash(dash)
  ctx.beginPath()
  ctx.moveTo(points[0].x, points[0].y)
  for (let i = 1; i < points.length; i++) ctx.lineTo(points[i].x, points[i].y)
  ctx.stroke()
  ctx.restore()
}

export function drawDisc(
  ctx: CanvasRenderingContext2D,
  p: Point,
  r: number,
  fill: string,
  stroke?: string,
) {
  ctx.beginPath()
  ctx.arc(p.x, p.y, r, 0, Math.PI * 2)
  if (fill) {
    ctx.fillStyle = fill
    ctx.fill()
  }
  if (stroke) {
    ctx.strokeStyle = stroke
    ctx.lineWidth = 1.5
    ctx.stroke()
  }
}

export function drawSquare(
  ctx: CanvasRenderingContext2D,
  p: Point,
  size: number,
  fill: string,
  stroke?: string,
) {
  const half = size / 2
  ctx.fillStyle = fill
  ctx.fillRect(p.x - half, p.y - half, size, size)
  if (stroke) {
    ctx.strokeStyle = stroke
    ctx.lineWidth = 1
    ctx.strokeRect(p.x - half, p.y - half, size, size)
  }
}
