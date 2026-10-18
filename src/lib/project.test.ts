import { describe, expect, it } from 'vitest'
import { Project, type ProjectDeps } from './project.svelte'
import type { Point } from './document'

/** No holes, no snapping: placement is where the caller asked for it. */
function makeProject(overrides: Partial<ProjectDeps> = {}) {
  let editable = true
  const deps: ProjectDeps = {
    canEdit: () => editable,
    place: (_kind, x, y) => ({ x, y }),
    pinOffsets: () => [],
    boardHolePoints: () => [],
    ...overrides,
  }
  return {
    project: new Project(deps),
    setEditable: (value: boolean) => {
      editable = value
    },
  }
}

const wire = (id: string, a: string, b: string) => ({
  id,
  a: { instanceId: a, pin: '1' },
  b: { instanceId: b, pin: '2' },
  waypoints: [],
})

describe('Project history', () => {
  it('records a step per edit and undoes/redoes it', () => {
    const { project } = makeProject()
    const led = project.add('led', 10, 20)!
    expect(project.instances).toHaveLength(1)
    expect(project.dirty).toBe(true)
    expect(project.history).toHaveLength(1)

    expect(project.undo()).toBe(true)
    expect(project.instances).toHaveLength(0)
    expect(project.redo()).toBe(true)
    expect(project.instances[0].id).toBe(led.id)
  })

  it('is dirty again after undoing a saved document', () => {
    // The regression this class exists to make testable: Save cleared the flag,
    // then Ctrl+Z moved away from the file and File -> New discarded it silently.
    const { project } = makeProject()
    project.add('led', 10, 20)
    project.markSaved()
    expect(project.dirty).toBe(false)

    project.undo()
    expect(project.instances).toHaveLength(0)
    expect(project.dirty).toBe(true)
  })

  it('loads a circuit with a clean history and no clipboard', () => {
    const { project } = makeProject()
    project.add('led', 10, 20)
    project.copySelection(project.instances.map((i) => i.id))
    project.load(
      [{ id: 'x', kind: 'led', x: 0, y: 0 }],
      [],
    )
    expect(project.instances).toHaveLength(1)
    expect(project.history).toHaveLength(0)
    expect(project.future).toHaveLength(0)
    expect(project.clipboard).toBeNull()
    expect(project.dirty).toBe(false)
  })
})

describe('Project clipboard', () => {
  it('pastes a group as one undo step, keeping its shape', () => {
    const { project } = makeProject()
    project.add('led', 100, 100)
    project.add('resistor', 140, 140)
    const ids = project.instances.map((i) => i.id)
    project.copySelection(ids)
    const steps = project.history.length

    const pasted = project.paste()
    expect(pasted).toHaveLength(2)
    expect(project.instances).toHaveLength(4)
    expect(project.history).toHaveLength(steps + 1)
    expect(project.instances[3].x - project.instances[2].x).toBe(40)
    expect(project.instances[3].y - project.instances[2].y).toBe(40)

    project.undo()
    expect(project.instances).toHaveLength(2)
  })

  it('duplicates the selection through copy and paste', () => {
    const { project } = makeProject()
    const led = project.add('led', 0, 0)!
    const made = project.duplicateSelection([led.id])
    expect(made).toHaveLength(1)
    expect(project.instances).toHaveLength(2)
  })
})

describe('Project editing', () => {
  it('deleting a part takes the wires hanging off it', () => {
    const { project } = makeProject()
    const uno = project.add('uno', 0, 0)!
    const led = project.add('led', 100, 0)!
    project.addWire(wire('w1', uno.id, led.id))
    project.addWire(wire('w2', led.id, 'elsewhere'))
    project.deleteSelection([led.id], null)
    expect(project.instances.map((i) => i.id)).toEqual([uno.id])
    expect(project.wires).toHaveLength(0)
  })

  it('clicking the same pins again removes that wire', () => {
    const { project } = makeProject()
    const a = { instanceId: 'a', pin: '1' }
    const b = { instanceId: 'b', pin: '2' }
    expect(project.toggleWire(a, b, () => wire('w1', 'a', 'b'))).toBeNull()
    expect(project.wires).toHaveLength(1)
    expect(project.toggleWire(b, a, () => wire('w2', 'b', 'a'))).toMatchObject({ id: 'w1' })
    expect(project.wires).toHaveLength(0)
  })

  it('rotates the whole selection as one undo step', () => {
    const { project } = makeProject()
    project.add('led', 0, 0)
    project.add('resistor', 100, 0)
    const ids = project.instances.map((i) => i.id)
    const steps = project.history.length
    project.rotate(ids, 1)
    expect(project.instances.map((i) => i.rotate)).toEqual([90, 90])
    expect(project.history).toHaveLength(steps + 1)
    project.undo()
    expect(project.instances.map((i) => i.rotate)).toEqual([undefined, undefined])
  })

  it('nudges the selection by whole grid steps, once', () => {
    const { project } = makeProject()
    project.add('led', 0, 0)
    project.add('resistor', 100, 0)
    const ids = project.instances.map((i) => i.id)
    const steps = project.history.length
    project.nudge(ids, 12, -12)
    expect(project.instances.map((i) => [i.x, i.y])).toEqual([[12, -12], [112, -12]])
    expect(project.history).toHaveLength(steps + 1)
  })

  it('changes a part attribute once, and ignores an identical value', () => {
    const { project } = makeProject()
    const led = project.add('led', 0, 0)!
    const steps = project.history.length
    project.setAttr(led, 'color', 'blue')
    expect(led.attrs).toMatchObject({ color: 'blue' })
    expect(project.history).toHaveLength(steps + 1)
    project.setAttr(led, 'color', 'blue')
    expect(project.history).toHaveLength(steps + 1)
  })
})

describe('Project while a simulation runs', () => {
  it('refuses to place parts or paste', () => {
    const { project, setEditable } = makeProject()
    project.add('led', 0, 0)
    project.copySelection(project.instances.map((i) => i.id))
    setEditable(false)
    expect(project.add('led', 10, 10)).toBeUndefined()
    expect(project.paste()).toEqual([])
    expect(project.instances).toHaveLength(1)
  })
})

describe('placement', () => {
  it('asks the injected geometry where a dropped part goes', () => {
    const spotted = new Map<string, Point>()
    const { project } = makeProject({
      place: (kind, x, y) => {
        spotted.set(kind, { x, y })
        return { x: x + 5, y: y + 7 }
      },
    })
    const led = project.add('led', 10, 20)!
    expect(spotted.get('led')).toEqual({ x: 10, y: 20 })
    expect([led.x, led.y]).toEqual([15, 27])
  })
})
