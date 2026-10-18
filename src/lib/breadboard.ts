/**
 * Breadboards.
 *
 * Wokwi's element library doesn't ship one, and the detailed third-party artwork
 * (Fritzing) is CC-BY-SA — which can't be baked into a AGPL-3.0 binary. So they're
 * generated from a spec instead. That suits a breadboard unusually well: its
 * drawing and its pin map are the *same regular grid*, so one spec produces both
 * and the two can't drift apart. (If they did, a wire would sit half a hole off
 * the copper it's supposed to touch.)
 *
 * The pitch is 67/7 ≈ 9.5714px — 2.54mm at 96dpi, and the same grid the
 * `wokwi-elements` parts are drawn on — so a component's legs land on our holes.
 *
 * Two sizes: a half board (30 columns, 400 tie points) and the full board
 * (63 columns, 830 tie points, with the power rails split in the middle the way
 * a real one is).
 */

/**
 * The hole spacing — chosen from the *worst-fitting* insertable part, which is
 * the pushbutton: its legs are 67px apart, and seven pitches of 67/7 makes that
 * exact. The others land within the hole radius rather than exactly on it:
 * the LED at 10px is 0.43px off, a resistor at 58.8px is 1.37px off.
 *
 * The three elements do not share a grid — a wokwi LED is drawn on 10px, a
 * resistor on 58.8, a pushbutton on 67 — so "exact for everything" does not exist
 * and the target is that every leg lands **inside** a hole (radius 2.1px). At the
 * old 10px pitch the button was 3px out, which is outside the hole: that is the
 * one that looked like it didn't fit, in both orientations.
 */
const PITCH = 67 / 7
/** Exported so the snap logic and the netlist's hole tolerance agree with the art. */
export const BREADBOARD_PITCH = PITCH

const ROWS_PER_SIDE = 5
const ROWS_TOP = ['a', 'b', 'c', 'd', 'e']
const ROWS_BOTTOM = ['f', 'g', 'h', 'i', 'j']
/** Holes in each power-rail segment. A full board splits every rail line in two. */
const RAIL_HOLES = 25
/** Columns skipped where a full board's rails break in the middle. */
const RAIL_BREAK = 3

// Board dressing, in the same pixel space as the holes.
const PAD = 16
const RAIL_GAP = PITCH
/**
 * The centre trench. One pitch wide, which makes the two rows either side of it
 * exactly *two* pitches apart — and that is what lets a two-row part (a
 * pushbutton, whose legs are 19px apart) straddle the trench, one side in each
 * bank. With the old 13px trench it was three-and-a-bit, so a button's legs
 * landed in one strip and its two sides were the same net: shorted, dead.
 */
const TRENCH = PITCH
const HOLE_R = 2.1
/** Exported so tests can assert that a part's legs land *inside* a hole. */
export const BREADBOARD_HOLE_R = HOLE_R
const BODY = '#f2f1ec'
const BODY_EDGE = '#c9c6bc'
const TRENCH_FILL = '#e4e2d9'
const HOLE = '#3a3a3c'
const PLUS = '#d0483f'
const MINUS = '#3f6fd0'
const INK = '#8d8a80'

/**
 * Which holes are electrically one point: the five holes in a strip, or a whole
 * power-rail segment. Two holes sharing a node name land in one net — that *is*
 * a breadboard's connectivity, which is why the netlist needs nothing else.
 * (A full board's rails break in the middle, hence the per-segment letter.)
 */
export function breadboardPinNode(pin: string): string {
  const strip = /^([a-j])(\d+)$/.exec(pin)
  if (strip) {
    // Columns a–e sit above the trench, f–j below; the two banks don't connect.
    const bank = strip[1] <= 'e' ? 'top' : 'bottom'
    return `${bank}${strip[2]}`
  }
  const rail = /^([tb])([pn])([A-Z])?\d+$/.exec(pin)
  if (rail) return `rail${rail[1]}${rail[2]}${rail[3] ?? ''}`
  return pin
}

/**
 * A pin, positioned in the element's own pixel space.
 *
 * `description` is copied from the element's `pinInfo` when it offers one (the
 * LED's "Anode", say) and is what the hover tooltip shows.
 */
export interface BreadboardPin {
  name: string
  x: number
  y: number
  description?: string
}

export interface BuiltBreadboard {
  tag: string
  columns: number
  width: number
  height: number
  pins: BreadboardPin[]
  svg: string
}

export interface BreadboardSpec {
  /** Element tag / `diagram.json` part type. */
  tag: string
  columns: number
  /** Power rails are split into this many segments per line. */
  railSegments: number
}

function build(spec: BreadboardSpec): BuiltBreadboard {
  const { columns, railSegments } = spec
  const pins: BreadboardPin[] = []
  const colX = (c: number) => PAD + (c - 1) * PITCH

  const railTop = PAD + 6
  const labelTop = railTop + RAIL_GAP + 11
  const gridTop = labelTop + 6
  const rowYTop = (i: number) => gridTop + i * PITCH
  const trenchTop = rowYTop(ROWS_PER_SIDE - 1) + PITCH * 0.5
  const rowYBottom = (i: number) => trenchTop + TRENCH + PITCH * 0.5 + i * PITCH

  // Signal rows: a–e above the trench, f–j below. The descriptions are what the
  // hover tooltip says, so a hole explains which strip it belongs to.
  for (let r = 0; r < ROWS_PER_SIDE; r++) {
    for (let c = 1; c <= columns; c++) {
      pins.push({
        name: `${ROWS_TOP[r]}${c}`,
        x: colX(c),
        y: rowYTop(r),
        description: `strip ${c} · upper`,
      })
      pins.push({
        name: `${ROWS_BOTTOM[r]}${c}`,
        x: colX(c),
        y: rowYBottom(r),
        description: `strip ${c} · lower`,
      })
    }
  }

  // Power rails, aligned to the column holes and stopping short of the edges.
  const span = RAIL_HOLES * railSegments + RAIL_BREAK * (railSegments - 1)
  const startCol = Math.floor((columns - span) / 2) + 1
  const segStart = (s: number) => startCol + s * (RAIL_HOLES + RAIL_BREAK)
  const railBottom = rowYBottom(ROWS_PER_SIDE - 1) + 15
  const railRows: { name: string; y: number }[] = [
    { name: 'tp', y: railTop },
    { name: 'tn', y: railTop + RAIL_GAP },
    { name: 'bp', y: railBottom },
    { name: 'bn', y: railBottom + RAIL_GAP },
  ]
  // One stripe per rail *segment* (a full board's rails have a break in them).
  const stripes: { from: number; to: number; y: number; colour: string }[] = []
  for (const row of railRows) {
    for (let s = 0; s < railSegments; s++) {
      const suffix = railSegments > 1 ? String.fromCharCode(65 + s) : ''
      const first = colX(segStart(s))
      const last = colX(segStart(s) + RAIL_HOLES - 1)
      stripes.push({
        from: first - PITCH * 0.6,
        to: last + PITCH * 0.6,
        y: row.y,
        colour: row.name.endsWith('p') ? PLUS : MINUS,
      })
      for (let j = 1; j <= RAIL_HOLES; j++) {
        pins.push({
          name: `${row.name}${suffix}${j}`,
          x: colX(segStart(s) + j - 1),
          y: row.y,
          description:
            `${row.name.endsWith('p') ? '+' : '−'} rail` + (suffix ? ` · segment ${suffix}` : ''),
        })
      }
    }
  }

  const width = PAD * 2 + (columns - 1) * PITCH
  const height = Math.max(...pins.map((p) => p.y)) + PAD

  const hole = (p: BreadboardPin) =>
    `<circle cx="${p.x.toFixed(1)}" cy="${p.y.toFixed(1)}" r="${HOLE_R}" fill="${HOLE}"/>`
  const stripe = (s: (typeof stripes)[number]) =>
    `<line x1="${s.from.toFixed(1)}" y1="${s.y}" x2="${s.to.toFixed(1)}" y2="${s.y}" stroke="${s.colour}" stroke-width="${(PITCH * 0.34).toFixed(1)}" stroke-linecap="round"/>`
  const label = (x: number, y: number, text: string) =>
    `<text x="${x.toFixed(1)}" y="${y.toFixed(1)}" font-family="sans-serif" font-size="7" fill="${INK}" text-anchor="middle">${text}</text>`

  const at = (name: string) => pins.find((p) => p.name === name)!
  // Column numbers: 1, every 5, and the last column — unless it would crowd the
  // label before it (a 63-column board would otherwise print "60" and "63" on
  // top of one another).
  const wanted = [1]
  for (let c = 5; c <= columns; c += 5) wanted.push(c)
  if (columns % 5 !== 0) wanted.push(columns)
  let labels = ''
  wanted
    .filter((c, i) => i === 0 || c - wanted[i - 1] > 3)
    .forEach((c) => {
      labels += label(colX(c), at('a1').y - 5, String(c))
    })
  for (const row of [...ROWS_TOP, ...ROWS_BOTTOM]) {
    labels += label(PAD - 9, at(`${row}1`).y + 2.5, row)
  }

  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
  <rect x="0.5" y="0.5" width="${width - 1}" height="${height - 1}" rx="5" fill="${BODY}" stroke="${BODY_EDGE}"/>
  <rect x="${PAD - 6}" y="${(at('e1').y + PITCH * 0.5).toFixed(1)}" width="${((columns - 1) * PITCH + 12).toFixed(1)}" height="${TRENCH}" fill="${TRENCH_FILL}"/>
  ${stripes.map(stripe).join('')}
  ${labels}
  ${pins.map(hole).join('')}
</svg>`

  return { tag: spec.tag, columns, width, height, pins, svg }
}

/**
 * Our own elements (not `wokwi-*` ones — no collision if Wokwi ever ships a
 * breadboard). They only have to do what the rasteriser expects: a shadow root
 * holding an `<svg>`, a laid-out size, and a `pinInfo` of `{name, x, y}`.
 */
function define(board: BuiltBreadboard) {
  if (typeof customElements === 'undefined' || customElements.get(board.tag)) return
  customElements.define(
    board.tag,
    class extends HTMLElement {
      readonly pinInfo: BreadboardPin[] = board.pins
      connectedCallback() {
        if (this.shadowRoot) return
        const root = this.attachShadow({ mode: 'open' })
        root.innerHTML = `<style>:host{display:block;width:${board.width}px;height:${board.height}px}svg{display:block}</style>${board.svg}`
      }
    },
  )
}

export const BREADBOARDS = {
  half: build({ tag: 'sim-breadboard', columns: 30, railSegments: 1 }),
  full: build({ tag: 'sim-breadboard-full', columns: 63, railSegments: 2 }),
}

for (const board of Object.values(BREADBOARDS)) define(board)
