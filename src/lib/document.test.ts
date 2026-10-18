import { describe, expect, it } from 'vitest'
import {
  attrsOf,
  clipboardOf,
  dedupeWires,
  historyPush,
  renderOrderOf,
  snapshotOf,
  type Instance,
  type Snapshot,
  type Wire,
} from './document'

const part = (over: Partial<Instance> = {}): Instance => ({
  id: 'p1',
  kind: 'led',
  x: 10,
  y: 20,
  ...over,
})

const wire = (id: string, a: string, b: string): Wire => ({
  id,
  a: { instanceId: a, pin: '1' },
  b: { instanceId: b, pin: '2' },
  waypoints: [],
})

describe('attrsOf', () => {
  it('puts the part own attributes over the catalogue defaults', () => {
    expect(attrsOf(part())).toMatchObject({ color: 'red' })
    expect(attrsOf(part({ attrs: { color: 'blue' } }))).toMatchObject({ color: 'blue' })
  })
})

describe('snapshotOf', () => {
  it('copies parts, wires and waypoints so later edits cannot reach back', () => {
    const inst = part({ attrs: { color: 'blue' } })
    const w = wire('w1', 'p1', 'p2')
    w.waypoints = [{ x: 1, y: 2 }]
    const snap = snapshotOf([inst], [w])

    inst.x = 999
    inst.attrs!.color = 'green'
    w.waypoints[0].x = 50

    expect(snap.instances[0]).toMatchObject({ x: 10, attrs: { color: 'blue' } })
    expect(snap.wires[0].waypoints[0]).toEqual({ x: 1, y: 2 })
  })
})

describe('historyPush', () => {
  it('keeps the newest 200 snapshots', () => {
    let history: Snapshot[] = []
    for (let i = 0; i < 205; i++) history = historyPush(history, { instances: [], wires: [] })
    expect(history).toHaveLength(200)
  })
})

describe('dedupeWires', () => {
  it('drops a pair already present, whichever way round it is', () => {
    const reversed: Wire = {
      id: 'w2',
      a: { instanceId: 'b', pin: '2' },
      b: { instanceId: 'a', pin: '1' },
      waypoints: [],
    }
    const kept = dedupeWires([wire('w1', 'a', 'b'), reversed, wire('w3', 'a', 'c')])
    expect(kept.map((w) => w.id)).toEqual(['w1', 'w3'])
  })
})

describe('renderOrderOf', () => {
  it('draws breadboards behind everything else, keeping placement order inside each group', () => {
    const order = renderOrderOf([
      part({ id: 'led', kind: 'led' }),
      part({ id: 'board', kind: 'breadboard' }),
      part({ id: 'res', kind: 'resistor' }),
      part({ id: 'full', kind: 'breadboard-full' }),
    ])
    expect(order.map((i) => i.id)).toEqual(['board', 'full', 'led', 'res'])
  })
})

describe('clipboardOf', () => {
  it('holds the group relative to its own top-left, with rotation and attributes', () => {
    const clip = clipboardOf(
      [
        part({ id: 'a', x: 100, y: 50, attrs: { color: 'blue' }, rotate: 90 }),
        part({ id: 'b', kind: 'resistor', x: 140, y: 90, attrs: { value: '1000' } }),
      ],
      ['a', 'b'],
    )
    expect(clip).not.toBeNull()
    expect(clip!.x).toBe(100)
    expect(clip!.y).toBe(50)
    expect(clip!.parts).toEqual([
      { kind: 'led', dx: 0, dy: 0, attrs: { color: 'blue' }, rotate: 90 },
      { kind: 'resistor', dx: 40, dy: 40, attrs: { value: '1000' }, rotate: 0 },
    ])
  })

  it('is empty when nothing is selected', () => {
    expect(clipboardOf([part()], [])).toBeNull()
  })
})
