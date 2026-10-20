<script lang="ts">
  import {
    PARTS,
    partByKind,
    type PartKind,
    formatResistance,
    parseResistance,
    type PartField,
  } from './lib/parts'
  import { type LedState, ledStep } from './lib/netlist'
  import {
    attrsOf,
    WIRE_GRID,
    type Instance,
    type PinRef,
    type Point,
    type Wire,
  } from './lib/document'
  import { Project } from './lib/project.svelte'
  import { FileSession, type FileBackend } from './lib/file-session.svelte'
  import { CanvasSurface, type PinGeom } from './lib/canvas.svelte'
  import { PRESETS, type Preset } from './lib/presets'
  import { buildDiagram as buildDiagramFile, parseDiagram } from './lib/diagram'
  import { decodeRoute, routePath } from './lib/route'
  import CodeEditor from './lib/CodeEditor.svelte'
  import { readPalette } from './lib/circuit-render'
  import { WIRE_COLORS, wireHex } from './lib/wire-colours'
  import { onMount, untrack } from 'svelte'
  import { View } from './lib/view.svelte'
  import { Sketch } from './lib/sketch.svelte'
  import { Sim, type SimBackend } from './lib/sim.svelte'
  import { Input } from './lib/input.svelte'
  import { parseCompileErrors, isSketchFile, type CompileError } from './lib/compile-errors'
  import { invoke } from '@tauri-apps/api/core'
  import { documentDir } from '@tauri-apps/api/path'
  import { getCurrentWindow } from '@tauri-apps/api/window'
  import { listen } from '@tauri-apps/api/event'
  import {
    open as openDialog,
    save as saveDialog,
    message as messageDialog,
  } from '@tauri-apps/plugin-dialog'

  type Theme = 'dark' | 'light'
  let theme = $state<Theme>('dark')

  $effect(() => {
    document.documentElement.dataset.theme = theme
  })

  const editor = new Sketch()

  // ---- Layout: resizable columns, panels that collapse to a rail ----
  const MIN_PANEL_W = 200
  const MAX_PANEL_W = 560

  let leftWidth = $state(320)
  let rightWidth = $state(240)
  let leftCollapsed = $state(false)
  let rightCollapsed = $state(false)

  function startResize(e: PointerEvent, side: 'left' | 'right') {
    if (e.button !== 0) return
    e.preventDefault()
    const startX = e.clientX
    const startW = side === 'left' ? leftWidth : rightWidth
    const onMove = (ev: PointerEvent) => {
      const delta = ev.clientX - startX
      const next = side === 'left' ? startW + delta : startW - delta
      const clamped = Math.min(MAX_PANEL_W, Math.max(MIN_PANEL_W, next))
      if (side === 'left') leftWidth = clamped
      else rightWidth = clamped
    }
    const onUp = () => {
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', onUp)
    }
    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', onUp)
  }

  const ICON_SKETCH = '<path d="M9 7l-5 5 5 5M15 7l5 5-5 5"/>'
  const ICON_COMPONENTS =
    '<rect x="6" y="6" width="12" height="12" rx="1"/><path d="M9 3.5v2M15 3.5v2M9 18.5v2M15 18.5v2M3.5 9h2M3.5 15h2M18.5 9h2M18.5 15h2"/>'
  const CHEVRON_LEFT = '<path d="M13.5 5.5l-6.5 6.5 6.5 6.5"/>'
  const CHEVRON_RIGHT = '<path d="M10.5 5.5l6.5 6.5-6.5 6.5"/>'

  // ---- Circuit canvas ----

  // The document owns parts, wires, history and the dirty flag (lib/project.svelte.ts).
  // These aliases are reactive and read-only here: every write goes through a Project
  // method, which is what keeps a single place responsible for an undo step.
  const project: Project = new Project({
    canEdit: () => !running,
    place: (kind: PartKind, x: number, y: number): Point =>
      surface.place(kind, surface.pinOffsetsFor(kind), x, y, instances),
    pinOffsets: (inst: Instance): Point[] => surface.pinOffsets(inst),
    boardHolePoints: (): Point[] => surface.boardHolePoints(instances),
  })

  let instances = $derived(project.instances)
  let wires = $derived(project.wires)
  let clipboard = $derived(project.clipboard)
  let history = $derived(project.history)
  let future = $derived(project.future)
  let dirty = $derived(project.dirty || editor.dirty)

  // Save/load + simulation. Only meaningful inside the Tauri app; a plain
  // browser hides these.
  const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

  // The desktop side of the file lifecycle: the commands and the native pickers.
  const fileBackend: FileBackend = {
    readText: (path) => invoke<string>('read_text_file', { path }),
    writeText: async (path, contents) => {
      await invoke('write_text_file', { path, contents })
    },
    pickOpen: async () => {
      const picked = await openDialog({
        multiple: false,
        filters: [{ name: 'Wokwi diagram', extensions: ['json'] }],
      })
      return typeof picked === 'string' ? picked : null
    },
    pickSave: async (defaultPath) => {
      const picked = await saveDialog({
        defaultPath,
        filters: [{ name: 'Wokwi diagram', extensions: ['json'] }],
      })
      return typeof picked === 'string' ? picked : null
    },
    defaultPath: async () => {
      if (!inTauri) return 'diagram.json'
      try {
        const dir = await documentDir()
        return `${dir.replace(/\/+$/, '')}/diagram.json`
      } catch {
        return 'diagram.json'
      }
    },
  }
  const files = new FileSession(fileBackend)

  // The simulation session (lib/sim.svelte.ts). Its commands are Tauri calls; a plain
  // browser still solves the netlist, it just cannot run a sketch.
  const simBackend: SimBackend = {
    isDesktop: () => inTauri,
    compile: (sketch) => invoke<{ elf: string }>('compile_sketch', { sketch }),
    start: async (elf, bounce) => {
      await invoke('start_sim', { elf, bounce })
    },
    stop: async () => {
      await invoke('stop_sim')
    },
    setInput: async (port, pin, high) => {
      await invoke('set_input', { port, pin, high })
    },
    setAnalog: async (channel, millivolts) => {
      await invoke('set_analog', { channel, millivolts })
    },
    writeSerial: async (text) => {
      await invoke('serial_input', { text })
    },
    onPin: (cb) =>
      listen<{ port: number; pin: number; high: boolean }>('sim:pin', (e) =>
        cb(e.payload.port, e.payload.pin, e.payload.high),
      ),
    onTiming: (cb) =>
      listen<{ port: number; pin: number; duty: number; highMicros: number; periodMicros: number }>(
        'sim:timing',
        (e) =>
          cb(e.payload.port, e.payload.pin, e.payload.duty, e.payload.highMicros, e.payload.periodMicros),
      ),
    onSerial: (cb) => listen<number>('sim:serial', (e) => cb(e.payload)),
  }
  const sim = new Sim(simBackend)

  let running = $derived(sim.running)
  let serial = $derived(sim.serial)
  let buildError = $derived(sim.buildError)
  let compileErrors = $derived(parseCompileErrors(buildError))
  let jump = $state<{ line: number; nonce: number } | null>(null)
  let jumpSeq = 0
  let serialInput = $state('')
  let serialNewline = $state(true)

  function sendSerial() {
    if (!serialInput) return
    void sim.sendSerial(serialNewline ? `${serialInput}\n` : serialInput)
    serialInput = ''
  }

  const SAVE_AND_CLOSE = 'Save and close'
  const DISCARD_AND_CLOSE = 'Discard and close'
  /** Set once the user has agreed, so the second close request is not re-asked. */
  let allowClose = false

  onMount(() => {
    if (!inTauri) return
    const win = getCurrentWindow()
    let disposed = false
    let unlisten: (() => void) | null = null

    win
      .onCloseRequested(async (event) => {
        if (allowClose || !dirty) return
        // Ask before the window goes away. This must happen synchronously, before
        // the first await, or the close has already been allowed.
        event.preventDefault()
        const answer = await messageDialog(
          'This circuit has changes that have not been saved yet.',
          {
            title: 'Close Croquis?',
            kind: 'warning',
            buttons: { yes: SAVE_AND_CLOSE, no: DISCARD_AND_CLOSE, cancel: 'Cancel' },
          },
        )
        if (answer !== SAVE_AND_CLOSE && answer !== DISCARD_AND_CLOSE) return
        if (answer === SAVE_AND_CLOSE) {
          await onSave()
          // Backed out of the file picker: the work is still unsaved, keep the window.
          if (dirty) return
        }
        allowClose = true
        await win.close()
      })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch((err) => console.error('close guard failed', err))

    return () => {
      disposed = true
      unlisten?.()
    }
  })

  function jumpToError(error: CompileError) {
    if (!isSketchFile(error.file)) return
    leftCollapsed = false
    jump = { line: error.line, nonce: ++jumpSeq }
  }
  let showSerial = $state(false)
  let pin13 = $derived(sim.pin13)

  /**
   * Whether a pressed switch chatters the way a real contact does. Off by default:
   * a first-run button sketch should read the way a beginner expects. Turn it on in
   * the Run menu to watch a sketch that never debounced misread a single press.
   */
  let switchBounce = $state(false)

  // The file lifecycle owns the save note and the error shown when a save or open
  // does not happen (lib/file-session.svelte.ts).
  let savedNote = $derived(files.savedNote)
  let fileError = $derived(files.error)
  // Electrical state derived from the simulation (see lib/netlist.ts).
  let avrPins = $derived(sim.avrPins)
  let ledStates = $derived(sim.ledStates)
  let ledBrightness = $derived(sim.ledBrightness)
  let rgbLeds = $derived(sim.rgbLeds)
  /** What's wrong with a part — the strip says it in words, the canvas badges it. */
  let warnings = $derived(sim.warnings)
  let pressedButtons = $derived(sim.pressedButtons)
  let sensorStates = $derived(sim.sensors)
  interface PendingWire {
    a: PinRef
    b: PinRef
    route: string[]
    colour?: string
  }
  // Wires from a loaded file, decoded once their pins have rendered.
  let pendingWires = $state<PendingWire[] | null>(null)

  // The canvas surface (lib/canvas.svelte.ts): pin geometry, sprites and drawing.
  const surface = new CanvasSurface(() => ({
    pin13,
    ledStates,
    ledBrightness,
    rgbLeds,
    pressedButtons,
    sensorStates,
    warnings,
  }))
  let canvasEl = $derived(surface.canvasEl)
  let pinsById = $derived(surface.pinsById)
  let sizesById = $derived(surface.sizesById)
  let renderError = $derived(surface.renderError)
  let partArt = $derived(surface.partArt)

  // The viewport (lib/view.svelte.ts): the canvas rectangle is passed in, so the
  // transform maths does not need to live in this component.
  const view = new View()
  let zoom = $derived(view.zoom)
  let panX = $derived(view.panX)
  let panY = $derived(view.panY)

  // Input owns selection and the gestures that change it (lib/input.svelte.ts).
  const input = new Input({
    project,
    view,
    surface,
    sim,
    canvasEl: () => surface.canvasEl,
    closeMenus: () => (openMenu = null),
  })
  let selectedIds = $derived(input.selectedIds)
  let selectedWireId = $derived(input.selectedWireId)
  const selectedId = $derived(input.selectedId)
  const selectedPart = $derived(input.selectedPart)
  let hoveredPin = $derived(input.hoveredPin)
  let hoverPos = $derived(input.hoverPos)
  let marquee = $derived(input.marquee)
  let dragging = $derived(input.dragging)
  let wiring = $derived(input.wiring)
  let rerouting = $derived(input.rerouting)
  const renderOrder = $derived(project.renderOrder)

  // ---- Zoom / pan -------------------------------------------------------
  function zoomAt(clientX: number, clientY: number, factor: number) {
    if (!canvasEl) return
    view.zoomAt(clientX, clientY, factor, canvasEl.getBoundingClientRect())
  }

  function zoomIn() {
    if (!canvasEl) return
    view.zoomIn(canvasEl.getBoundingClientRect())
  }

  function zoomOut() {
    if (!canvasEl) return
    view.zoomOut(canvasEl.getBoundingClientRect())
  }

  /** Put the world origin in the middle of the canvas rather than in its corner. */
  function centreView() {
    if (!canvasEl) return
    view.centre(canvasEl.getBoundingClientRect())
  }

  /**
   * Centre it once, when the canvas first exists. Guarded because an effect that
   * re-centres on every change would fight the user's own panning.
   */
  $effect(() => {
    if (view.centred || !canvasEl) return
    view.centred = true
    centreView()
  })

  function resetZoom() {
    if (!canvasEl) return
    view.reset(canvasEl.getBoundingClientRect())
  }

  // ---- Pins -------------------------------------------------------------
  // The mounted elements are the only thing that knows where their pins are, so
  // the surface measures them. These wrappers keep the call sites terse.
  $effect(() => surface.measure(instances))

  function pinCanvasPoint(inst: Instance, pin: PinGeom): Point {
    return surface.pinCanvasPoint(inst, pin)
  }

  function pinPoint(ref: PinRef): Point | null {
    return surface.pinPoint(instances, ref)
  }

  // ---- Part dragging ----------------------------------------------------

  // Keep the drawn wire following the pointer, even between clicks.
  $effect(() => input.trackCursor())

  // Wheel zooms toward the pointer.
  $effect(() => {
    const el = canvasEl
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      zoomAt(e.clientX, e.clientY, e.deltaY < 0 ? 1.1 : 1 / 1.1)
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  })

  // Registered once. Reads of selections happen when a key is pressed, not
  // while this effect runs — so it isn't re-attached on change.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null
      if (t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)) return
      if (e.ctrlKey || e.metaKey) {
        const k = e.key.toLowerCase()
        if (k === '=' || k === '+') zoomIn()
        else if (k === '-' || k === '_') zoomOut()
        else if (k === '0') resetZoom()
        else if (k === 'n' && inTauri) void newProject()
        else if (k === 's' && e.shiftKey && inTauri) void onSaveAs()
        else if (k === 's' && inTauri) void onSave()
        else if (k === 'o' && inTauri) void onOpen()
        else if (k === 'z') {
          if (running) return
          if (e.shiftKey) redo()
          else undo()
        } else if (k === 'y') {
          if (running) return
          redo()
        } else if (k === 'a') {
          selectAll()
        } else if (k === 'c') copySelection()
        else if (k === 'x') {
          // Circuit edits wait until the simulation is stopped.
          if (running) return
          cutSelection()
        } else if (k === 'v') {
          if (running) return
          paste()
        } else if (k === 'd') {
          if (running) return
          duplicateSelection()
        } else return
        e.preventDefault()
        return
      }
      if (!running && selectedIds.length && (e.key === 'r' || e.key === 'R')) {
        // Shift+R turns it right around; plain R steps 90°.
        rotatePart(e.shiftKey ? 2 : 1)
        e.preventDefault()
        return
      }
      // Arrow keys nudge the selection by one grid step. An undo step each, which the
      // old version did not have at all — nudging was the one place you could move
      // something and not be able to take it back.
      const nudging =
        e.key === 'ArrowLeft' || e.key === 'ArrowRight' || e.key === 'ArrowUp' || e.key === 'ArrowDown'
      if (!running && selectedIds.length && nudging) {
        const dx =
          e.key === 'ArrowLeft' ? -WIRE_GRID : e.key === 'ArrowRight' ? WIRE_GRID : 0
        const dy = e.key === 'ArrowUp' ? -WIRE_GRID : e.key === 'ArrowDown' ? WIRE_GRID : 0
        input.nudge(dx, dy)
        e.preventDefault()
        return
      }
      if (e.key === 'Escape') {
        input.cancelWire()
        input.clearSelection()
        return
      }
      if (!running && (e.key === 'Delete' || e.key === 'Backspace')) {
        if (selectedWireId || selectedIds.length) {
          input.deleteSelection()
          e.preventDefault()
        }
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  // ---- Save / load (diagram.json) ----------------------------------------


  function buildDiagram() {
    return buildDiagramFile({
      parts: instances.map((inst) => ({
        id: inst.id,
        kind: inst.kind,
        x: inst.x,
        y: inst.y,
        rotate: inst.rotate,
        attrs: attrsOf(inst),
      })),
      wires: wires.map((w) => ({ a: w.a, b: w.b, colour: w.colour, waypoints: w.waypoints })),
      sketch: editor.text,
      // The one thing only the app can answer: where a pin actually sits.
      pinPoint,
    })
  }

  function loadDiagram(text: string) {
    // Parsing and validation live in the format module, where they can be tested. A
    // file it cannot parse throws before anything here has been touched.
    const doc = parseDiagram(text)
    project.load(doc.parts, [])
    files.error = ''
    input.clearSelection()
    pendingWires = doc.wires
    editor.load(doc.sketch)
  }

  const SAVE_THEN_NEW = 'Save and start new'
  const DISCARD_CHANGES = 'Discard changes'

  /**
   * Start a blank circuit.
   *
   * The circuit is the one thing here that cannot be regenerated, so this asks before
   * throwing it away — and asks with three answers, because "no" and "wait" are not the
   * same thing. Discarding is available; losing work to a mis-click is not.
   */
  async function newProject() {
    if (dirty) {
      // Nothing to ask without a native dialog.
      if (!inTauri) return resetToBlank()
      const answer = await messageDialog('This circuit has changes that have not been saved yet.', {
        title: 'Start a new circuit?',
        kind: 'warning',
        buttons: { yes: SAVE_THEN_NEW, no: DISCARD_CHANGES, cancel: 'Cancel' },
      })
      if (answer !== SAVE_THEN_NEW && answer !== DISCARD_CHANGES) return
      if (answer === SAVE_THEN_NEW) {
        await onSave()
        // Backed out of the file picker: the work is still unsaved, so keep it.
        if (dirty) return
      }
    }
    // Only once the decision is made — cancelling must not stop a running sketch.
    if (running) await stopSim()
    resetToBlank()
  }

  function resetToBlank() {
    project.reset()
    // A new circuit has no home, so the first Save asks. Without this it would write
    // over whatever had been open before.
    files.reset()
    pendingWires = null
    input.clearSelection()
    editor.reset()
    sim.reset()
  }

  /**
   * Save writes straight to wherever this circuit lives. The picker is for *choosing* a
   * home, not for confirming one — asking again every time is what makes saving feel
   * like an export rather than the thing you do constantly. With no home yet, this is
   * a Save as.
   *
   * Deliberately an unconfirmed overwrite, the way every editor behaves, which is what
   * makes Ctrl+S worth pressing. Save as… is how you move or rename it.
   */
  async function saveCircuit(write: (json: string) => Promise<boolean>) {
    const ok = await write(JSON.stringify(buildDiagram(), null, 2))
    if (!ok) return false
    // In step with the disk again, so File -> New stops asking about it.
    project.markSaved()
    editor.markSaved()
    return true
  }

  async function onSave() {
    await saveCircuit((json) => files.save(json))
  }

  /** Always asks, even when there is a home — this is how the home moves. */
  async function onSaveAs() {
    await saveCircuit((json) => files.saveAs(json))
  }

  /**
   * Load a shipped circuit. It goes through the same loader as File → Open, because
   * a preset *is* a diagram.json — and like Open it replaces the whole circuit, so it
   * cannot happen under a live simulation.
   */
  async function loadPreset(preset: Preset) {
    if (running) await stopSim()
    loadDiagram(preset.text)
  }

  async function onOpen() {
    // Loading a diagram replaces the whole circuit, so it can't happen under a
    // live simulation — stop first rather than silently ignoring the click.
    if (running) await stopSim()
    // The path becomes the circuit's home only after `loadDiagram` accepted the file.
    await files.open((text) => loadDiagram(text))
  }

  // Decode loaded wire routes once the parts (and their pins) have rendered.
  $effect(() => {
    if (!pendingWires) return
    const created: Wire[] = []
    for (const p of pendingWires) {
      const a = pinPoint(p.a)
      const b = pinPoint(p.b)
      if (!a || !b) return // pins not ready yet — wait for them
      created.push({
        id: input.nextWireId(),
        a: p.a,
        b: p.b,
        waypoints: p.route.length ? decodeRoute(a, p.route) : [],
        colour: p.colour,
      })
    }
    project.setWires(created)
    pendingWires = null
    // The file is now fully on the canvas, so frame it. Deferred a frame because the
    // instances have only just been inserted and their sizes aren't measured yet.
    requestAnimationFrame(() => fitView())
  })

  // ---- Undo / redo -------------------------------------------------------
  function commit() {
    project.commit()
  }

  function undo() {
    if (!project.undo()) return
    input.clearSelection()
    input.cancelWire()
  }

  function redo() {
    if (!project.redo()) return
    input.clearSelection()
    input.cancelWire()
  }

  // ---- Clipboard ---------------------------------------------------------

  function deleteSelection() {
    input.deleteSelection()
  }

  /**
   * Everything on the canvas.
   *
   * Wires are left out deliberately: they are selected one at a time, so "all of them" is
   * not a thing the rest of the app could act on.
   */
  function selectAll() {
    input.selectAll()
  }

  function copySelection() {
    input.copySelection()
  }

  function cutSelection() {
    input.cutSelection()
  }

  function paste() {
    input.paste()
  }

  function duplicateSelection() {
    input.duplicateSelection()
  }

  // ---- Wires: colour, fit, clear -----------------------------------------
  // `quarterTurns` is 1 for the R shortcut / toolbar, 2 for "Rotate 180°".
  function rotatePart(quarterTurns = 1) {
    input.rotatePart(quarterTurns)
  }

  function setWireColour(name: string) {
    input.setWireColour(name)
  }

  function fitView() {
    if (!canvasEl || !instances.length) return
    view.fit(instances, sizesById, canvasEl.getBoundingClientRect())
  }

  function clearCanvas() {
    if (running) return
    if (!instances.length && !wires.length) return
    project.clear()
    input.clearSelection()
    input.cancelWire()
  }

  // ---- Menu bar ----------------------------------------------------------
  // The home for commands that have none of their own: undo/redo, cut/copy/
  // paste, rotate, clear. Everything here also has a keyboard shortcut, and the
  // primary actions (Run, zoom, wiring, the palette) stay visible as buttons —
  // nothing *essential* lives only in here.
  type MenuId = 'file' | 'edit' | 'view' | 'run'
  let openMenu = $state<MenuId | null>(null)
  const hasSelection = $derived(selectedIds.length > 0 || !!selectedWireId)

  function toggleMenu(id: MenuId) {
    openMenu = openMenu === id ? null : id
  }

  // With a menu open, sliding across the titles switches menus (menu-bar feel).
  function hoverMenu(id: MenuId) {
    if (openMenu) openMenu = id
  }

  function menuPick(fn: () => void) {
    openMenu = null
    fn()
  }

  // Any press outside, Esc, or losing the window closes the open menu.
  $effect(() => {
    if (!openMenu) return
    const dismiss = () => {
      openMenu = null
    }
    const close = (e: Event) => {
      const t = e.target as Element | null
      if (t?.closest?.('.menu, .menubar')) return
      dismiss()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') dismiss()
    }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', dismiss)
    return () => {
      window.removeEventListener('pointerdown', close)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', dismiss)
    }
  })

  // What the pin under the pointer is. A single LED or the Uno is guessable; a
  // 4-leg button or a sensor isn't, so this is what makes them usable.
  function pinTooltip(ref: PinRef): { part: string; name: string; note: string } | null {
    const inst = instances.find((i) => i.id === ref.instanceId)
    if (!inst) return null
    return {
      part: partByKind(inst.kind).label,
      name: ref.pin,
      note: pinsById[ref.instanceId]?.find((p) => p.name === ref.pin)?.description ?? '',
    }
  }

  // ---- Right-click menu --------------------------------------------------
  // An in-app menu, not a native one: it wears the app's tokens, and every
  // command here also has a keyboard shortcut — nothing is menu-only.
  let menu = $state<{ x: number; y: number; partId?: string; wireId?: string } | null>(null)

  const MENU_W = 184
  const MENU_H = 260

  function openContextMenu(e: MouseEvent) {
    // A right-drag is selecting, not asking for a menu.
    if (marquee) {
      e.preventDefault()
      return
    }
    // Right-clicking a part outside the selection means that part — otherwise the menu
    // would act on parts you cannot see it acting on.
    const hit = (e.target as Element | null)?.closest('.instance') as HTMLElement | null
    const hitId = hit?.dataset.id
    if (hitId && !selectedIds.includes(hitId)) input.selectOnly(hitId)
    const el = e.target as Element | null
    const partEl = el?.closest('.instance') as HTMLElement | null
    const wireEl = el?.closest('.wire__hit') as Element | null
    const partId = partEl?.dataset.id
    const wireId = wireEl?.getAttribute('data-wire') ?? undefined
    if (!partId && !wireId) {
      menu = null
      return
    }
    e.preventDefault()
    // The guard above already narrowed the selection to the clicked part when it was not
    // part of it. It must not be narrowed again here, or right-clicking any part of a
    // multi-selection would throw the rest away and the menu could never act on a group.
    // Right-clicking a *wire* is not about parts at all, so that does clear them.
    if (!partId) input.clearSelection()
    input.selectWire(wireId ?? null)
    menu = {
      x: Math.max(0, Math.min(e.clientX, window.innerWidth - MENU_W)),
      y: Math.max(0, Math.min(e.clientY, window.innerHeight - MENU_H)),
      partId,
      wireId,
    }
  }

  function runMenu(fn: () => void) {
    menu = null
    fn()
  }

  /**
   * Whether every selected part is a microcontroller.
   *
   * The context menu offers only what *all* of the selection can do, so "Edit code"
   * appears only when there is nothing in the selection it would not apply to.
   */
  function menuIsMcu(): boolean {
    if (!selectedIds.length) return false
    return selectedIds.every((id) => instances.some((i) => i.id === id && i.kind === 'uno'))
  }

  // "Edit code" — raise the Sketch panel and put the caret in it. When each
  // microcontroller gets its own sketch, this is also where the tab switches.
  function editSketch() {
    menu = null
    leftCollapsed = false
    // The editor may need to mount (or expand) first, hence the retries.
    let tries = 0
    const focus = () => {
      const content = document.querySelector('.cm-content') as HTMLElement | null
      if (content) {
        content.focus()
        content.scrollIntoView({ block: 'nearest' })
      } else if (++tries <= 10) {
        requestAnimationFrame(focus)
      }
    }
    requestAnimationFrame(focus)
  }

  // Any press outside, Esc, or losing the window closes the menu.
  $effect(() => {
    if (!menu) return
    const dismiss = () => {
      menu = null
    }
    const close = (e: Event) => {
      // Presses *inside* the menu must reach the item, not dismiss it.
      const t = e.target as Element | null
      if (t?.closest?.('.menu')) return
      dismiss()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') dismiss()
    }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', onKey)
    window.addEventListener('blur', dismiss)
    return () => {
      window.removeEventListener('pointerdown', close)
      window.removeEventListener('keydown', onKey)
      window.removeEventListener('blur', dismiss)
    }
  })

  // ---- Canvas renderer ---------------------------------------------------
  // The surface owns the sprite cache and the drawing (lib/canvas.svelte.ts).

  // Also re-run the palette thumbnails when the panel reopens, in case the tiles
  // weren't mounted yet.
  $effect(() => {
    void rightCollapsed
    requestAnimationFrame(() => surface.buildPaletteArt())
  })

  /** Change one property of a part (undoable). */
  /**
   * The resistance field is the one property you *type* rather than pick, so it
   * keeps the text as it is being written and the unit it is being written in.
   * Ohms stay the stored truth; the unit is only how you read and write them.
   */
  let resText = $state('')
  let resUnit = $state(1)

  // Re-read the controls when a different part is selected. Keyed on the selection
  // alone — the value is read without tracking — so this cannot re-trigger itself
  // while the field is being typed in.
  $effect(() => {
    const id = selectedId
    if (!id) return
    const inst = instances.find((i) => i.id === id)
    if (!inst) return
    untrack(() => {
      const field = (partByKind(inst.kind).fields ?? []).find((f) => f.control.kind === 'resistance')
      if (!field || field.control.kind !== 'resistance') return
      const ohms = Number(attrsOf(inst)[field.name] ?? field.control.min)
      const tidy = formatResistance(ohms)
      resText = tidy.text
      resUnit = tidy.factor
    })
  })

  /** A resistance written in the unit currently chosen. */
  function inUnit(ohms: number): string {
    const written = ohms / resUnit
    // Below a thousandth of the chosen unit, rounding to three decimals would show a
    // zero that is not true. Show it in full instead — 2.2e-10 reads oddly but is
    // honest, and it types back correctly.
    if (Math.abs(written) < 0.001) return String(written)
    return String(Math.round(written * 1000) / 1000)
  }

  /** Switching unit is a change of view, not of value: 4700 Ω reads "4700" in Ω. */
  function pickUnit(factor: number) {
    if (!selectedPart) return
    resUnit = factor
    const ohms = Number(attrsOf(selectedPart).value ?? 0)
    if (ohms > 0) {
      resUnit = factor
      resText = String(Math.round((ohms / factor) * 1000) / 1000)
    }
  }

  function commitResistance(field: PartField, text: string) {
    if (!selectedPart) return
    const control = field.control
    if (control.kind !== 'resistance') return
    const ohms = parseResistance(text, resUnit, control.min, control.max)
    if (ohms === null) {
      // Not a number: put back what is actually stored, rather than guessing at it.
      resText = inUnit(Number(attrsOf(selectedPart)[field.name] ?? control.min))
      return
    }
    setAttr(selectedPart, field.name, String(ohms))
    // Show what was stored, in the unit being typed in, so a value that had to be
    // clamped is visible rather than silently different from what was typed.
    resText = inUnit(ohms)
  }

  function setAttr(inst: Instance, name: string, value: string) {
    if (attrsOf(inst)[name] === value) return
    commit()
    inst.attrs = { ...inst.attrs, [name]: value }
  }

  /**
   * A range control (the potentiometer's position) is dragged, not typed, so it
   * applies live — the circuit re-solves on every step, which is the whole point of
   * watching an `analogRead`. A whole drag is still one undo step: the first real
   * movement commits and the rest ride along, until the pointer or key lets go.
   */
  let rangeDragging = false

  function setRange(inst: Instance, name: string, value: number) {
    const text = String(value)
    if (attrsOf(inst)[name] === text) return
    if (!rangeDragging) {
      commit()
      rangeDragging = true
    }
    inst.attrs = { ...inst.attrs, [name]: text }
  }

  function endRange() {
    rangeDragging = false
  }

  /** Change one property of a part (undoable). */

  // Redraw whenever anything the canvas shows changes.
  $effect(() => {
    void surface.partImagesVersion
    void surface.sizeVersion
    void surface.palette
    for (const inst of instances) {
      void inst.x
      void inst.y
      void inst.rotate
    }
    for (const w of wires) {
      void w.waypoints
      void w.colour
    }
    for (const key in pinsById) void pinsById[key]
    for (const key in sizesById) void sizesById[key]
    void selectedId
    void selectedIds
    void marquee
    void selectedWireId
    void hoveredPin
    void wiring
    void rerouting
    surface.draw({
      instances,
      wires,
      selectedIds,
      selectedWireId,
      hoveredPin,
      zoom,
      panX,
      panY,
      wiring,
      wiringPoints: input.wiringPoints,
      wiringTarget: input.wiringTarget,
    })
  })

  // Keep the drawn bitmap in step with the canvas's layout size.
  $effect(() => surface.watchSize())

  // Keep the canvas palette in step with the theme.
  $effect(() => {
    void theme
    surface.palette = readPalette()
  })

  // ---- Simulation --------------------------------------------------------
  /**
   * What the simulation drives on a part's element, if anything. Only the
   * *property name* is per-kind, and that is behaviour rather than registration,
   * so it lives here instead of being scattered through the template.
   */
  function liveProp(inst: Instance): { name: string; value: unknown }[] | null {
    if (inst.kind === 'uno') return [{ name: 'led13', value: pin13 }]
    if (inst.kind === 'led') {
      // `brightness` is how the element shows a dim LED; `value` is on/off. Both
      // use the same quantised step as the sprite cache, so the drawn pixels and
      // the cache key cannot disagree.
      const step = ledStep(ledBrightness[inst.id] ?? 0)
      return [
        { name: 'brightness', value: step },
        { name: 'value', value: step > 0 },
      ]
    }
    if (inst.kind === 'pushbutton') return [{ name: 'pressed', value: !!pressedButtons[inst.id] }]
    if (inst.kind === 'analog-joystick') {
      const attrs = attrsOf(inst)
      // xValue positive moves the element's knob left, so invert it: dragging
      // right (x = 100) must move the knob right.
      const x = 1 - Number(attrs.x ?? 50) / 50
      const y = Number(attrs.y ?? 50) / 50 - 1
      return [
        { name: 'xValue', value: Math.min(1, Math.max(-1, x)) },
        { name: 'yValue', value: Math.min(1, Math.max(-1, y)) },
        { name: 'pressed', value: !!pressedButtons[inst.id] },
      ]
    }
    if (inst.kind === 'rgb-led') {
      const state = rgbLeds[inst.id] ?? { r: 0, g: 0, b: 0 }
      return [
        { name: 'ledRed', value: ledStep(state.r) },
        { name: 'ledGreen', value: ledStep(state.g) },
        { name: 'ledBlue', value: ledStep(state.b) },
      ]
    }
    // A sensor module's own indicator lamps say powered and on/off — the module
    // shows its comparator result, the same condition its DO pin drives.
    if (inst.kind === 'photoresistor-sensor') {
      const state = sensorStates[inst.id]
      return [
        { name: 'ledPower', value: !!state?.powered },
        { name: 'ledDO', value: !!state?.active },
      ]
    }
    return null
  }

  /** One action for every part: push that state into whichever element it is. */
  function liveState(node: HTMLElement, state: { name: string; value: unknown }[] | null) {
    const apply = (s: { name: string; value: unknown }[] | null) => {
      for (const item of s ?? []) {
        ;(node as unknown as Record<string, unknown>)[item.name] = item.value
      }
    }
    apply(state)
    return { update: apply }
  }

  // When the circuit or the chip's pins change: re-solve the netlist, light the
  // LEDs, and push any button-driven pins back into the simulator.
  $effect(() => {
    const parts = instances.map((i) => ({
      id: i.id,
      kind: i.kind,
      attrs: attrsOf(i),
      // Canvas-space pins, so a leg resting in a breadboard hole can connect.
      pins: (pinsById[i.id] ?? []).map((p) => ({
        name: p.name,
        ...pinCanvasPoint(i, p),
      })),
    }))
    const refs = wires.map((w) => ({ a: { ...w.a }, b: { ...w.b } }))
    // Re-solve when the run state flips too: `Sim.solve` only pushes its drives and
    // analogue values while running, so start needs a solve to carry the state that
    // was already on the canvas into a freshly started simulator.
    void running
    // Handed over by reference: `Sim.solve` reads `pressedButtons[id]` even for an id
    // that is not in the object yet, which is what carries a press from the canvas
    // into the simulator. Spreading reads only the keys it already has, so a button
    // that had never been pressed was invisible and no pin was ever driven.
    sim.solve(parts, refs)
  })

  async function runSim() {
    // A fresh run never inherits a half-drawn wire or a held button.
    input.cancelWire()
    input.rerouting = null
    await sim.run(editor.text, switchBounce)
    showSerial = true
  }

  async function stopSim() {
    await sim.stop()
  }

  // Apply an edited sketch without leaving the run: recompile and restart.
  async function restartSim() {
    input.cancelWire()
    input.rerouting = null
    await sim.restart(editor.text, switchBounce)
  }

  /**
   * Hand focus to the canvas.
   *
   * Blurring the editor is not enough on every engine: WebKitGTK can leave a
   * contenteditable holding focus, so keys carry on going into the code and nothing
   * you press reaches the circuit — the shortcuts look dead even while you are
   * dragging a part. Making the canvas the active element is unambiguous, and it is
   * what the canvas is for: it is the `role="application"` the keys act on.
   */
  function focusCanvas() {
    canvasEl?.focus({ preventScroll: true })
  }

  // Pressing anywhere outside the code editor moves focus off it, so the canvas
  // keybinds (Delete, R, Ctrl+Z…) work straight after editing.
  $effect(() => {
    const onDown = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null
      if (target?.closest('.cm-editor')) return
      // Something that takes focus itself — a control, a menu item — is left to the
      // browser; we would only be stealing it back.
      if (target?.closest('input, textarea, select, button, [contenteditable]')) return
      const active = document.activeElement as HTMLElement | null
      if (
        active &&
        active !== document.body &&
        (active.isContentEditable || active.tagName === 'INPUT' || active.tagName === 'TEXTAREA')
      ) {
        active.blur()
      }
      focusCanvas()
    }
    document.addEventListener('pointerdown', onDown, true)
    return () => document.removeEventListener('pointerdown', onDown, true)
  })

  // Stream pin changes and serial bytes from the engine (set up once).
  $effect(() => sim.attach())
</script>

<div class="app">
  <header class="topbar">
    <span class="brand">Croquis</span>

    <nav class="menubar" aria-label="Main menu">
      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'file'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'file'}
          onclick={() => toggleMenu('file')}
          onpointerenter={() => hoverMenu('file')}
        >
          File
        </button>
        {#if openMenu === 'file'}
          <div class="menu menu--anchored">
            <button class="menu__item" onclick={() => menuPick(newProject)}>
              <span>New</span><span class="menu__key">Ctrl+N</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" disabled={!inTauri} onclick={() => menuPick(onOpen)}>
              <span>Open…</span><span class="menu__key">Ctrl+O</span>
            </button>
            <button class="menu__item" disabled={!inTauri} onclick={() => menuPick(onSave)}>
              <span>Save</span><span class="menu__key">Ctrl+S</span>
            </button>
            <button class="menu__item" disabled={!inTauri} onclick={() => menuPick(onSaveAs)}>
              <span>Save as…</span><span class="menu__key">Ctrl+Shift+S</span>
            </button>
            <div class="menu__rule"></div>
            <!-- A labelled section rather than a submenu: one click, and a stack of
                 menus is the thing this app keeps out. These work without Tauri too,
                 since they are compiled in rather than read from disk. -->
            <div class="menu__note">Presets</div>
            {#each PRESETS as preset (preset.id)}
              <button class="menu__item" onclick={() => menuPick(() => loadPreset(preset))}>
                <span>{preset.label}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'edit'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'edit'}
          onclick={() => toggleMenu('edit')}
          onpointerenter={() => hoverMenu('edit')}
        >
          Edit
        </button>
        {#if openMenu === 'edit'}
          <div class="menu menu--anchored">
            <button class="menu__item" disabled={running || !history.length} onclick={() => menuPick(undo)}>
              <span>Undo</span><span class="menu__key">Ctrl+Z</span>
            </button>
            <button class="menu__item" disabled={running || !future.length} onclick={() => menuPick(redo)}>
              <span>Redo</span><span class="menu__key">Ctrl+Shift+Z</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" disabled={running || !hasSelection} onclick={() => menuPick(cutSelection)}>
              <span>Cut</span><span class="menu__key">Ctrl+X</span>
            </button>
            <button class="menu__item" disabled={!hasSelection} onclick={() => menuPick(copySelection)}>
              <span>Copy</span><span class="menu__key">Ctrl+C</span>
            </button>
            <button class="menu__item" disabled={running || !clipboard} onclick={() => menuPick(paste)}>
              <span>Paste</span><span class="menu__key">Ctrl+V</span>
            </button>
            <button
              class="menu__item"
              disabled={running || !instances.length}
              onclick={() => menuPick(selectAll)}
            >
              <span>Select all</span><span class="menu__key">Ctrl+A</span>
            </button>
            <button
              class="menu__item"
              disabled={running || !hasSelection}
              onclick={() => menuPick(duplicateSelection)}
            >
              <span>Duplicate</span><span class="menu__key">Ctrl+D</span>
            </button>
            <div class="menu__rule"></div>
            <button
              class="menu__item"
              disabled={running || !hasSelection}
              onclick={() => menuPick(() => rotatePart(1))}
            >
              <span>Rotate 90°</span><span class="menu__key">R</span>
            </button>
            <button
              class="menu__item"
              disabled={running || !hasSelection}
              onclick={() => menuPick(() => rotatePart(2))}
            >
              <span>Rotate 180°</span><span class="menu__key">Shift+R</span>
            </button>
            <div class="menu__rule"></div>
            <button
              class="menu__item menu__item--danger"
              disabled={running || !hasSelection}
              onclick={() => menuPick(deleteSelection)}
            >
              <span>Delete</span><span class="menu__key">Del</span>
            </button>
          </div>
        {/if}
      </div>

      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'view'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'view'}
          onclick={() => toggleMenu('view')}
          onpointerenter={() => hoverMenu('view')}
        >
          View
        </button>
        {#if openMenu === 'view'}
          <div class="menu menu--anchored">
            <button
              class="menu__item"
              onclick={() => menuPick(() => (theme = theme === 'dark' ? 'light' : 'dark'))}
            >
              <span>{theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'}</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" onclick={() => menuPick(fitView)}>
              <span>Fit view to circuit</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(zoomIn)}>
              <span>Zoom in</span><span class="menu__key">Ctrl +</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(zoomOut)}>
              <span>Zoom out</span><span class="menu__key">Ctrl -</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(resetZoom)}>
              <span>Reset zoom</span><span class="menu__key">Ctrl 0</span>
            </button>
            <div class="menu__rule"></div>
            <button class="menu__item" onclick={() => menuPick(() => (leftCollapsed = !leftCollapsed))}>
              <span>{leftCollapsed ? 'Show' : 'Hide'} Sketch panel</span>
            </button>
            <button class="menu__item" onclick={() => menuPick(() => (rightCollapsed = !rightCollapsed))}>
              <span>{rightCollapsed ? 'Show' : 'Hide'} Components panel</span>
            </button>
          </div>
        {/if}
      </div>

      <div class="menubar__item">
        <button
          class="menubar__btn"
          class:is-open={openMenu === 'run'}
          aria-haspopup="menu"
          aria-expanded={openMenu === 'run'}
          onclick={() => toggleMenu('run')}
          onpointerenter={() => hoverMenu('run')}
        >
          Run
        </button>
        {#if openMenu === 'run'}
          <div class="menu menu--anchored">
            <button
              class="menu__item"
              disabled={!inTauri || running}
              onclick={() => menuPick(runSim)}
            >
              <span>Run</span>
            </button>
            <button class="menu__item" disabled={!running} onclick={() => menuPick(restartSim)}>
              <span>Restart</span>
            </button>
            <button class="menu__item" disabled={!running} onclick={() => menuPick(stopSim)}>
              <span>Stop</span>
            </button>
            <div class="menu__rule"></div>
            <!-- A setting rather than an action, which is why it sits below the rule.
                 Takes effect on the next Run, since the contacts' behaviour is fixed
                 when the simulation starts. -->
            <button
              class="menu__item"
              role="menuitemcheckbox"
              aria-checked={switchBounce}
              onclick={() => menuPick(() => (switchBounce = !switchBounce))}
            >
              <span>Switch bounce</span>
              <span class="menu__key">{switchBounce ? 'On' : 'Off'}</span>
            </button>
          </div>
        {/if}
      </div>
    </nav>
  </header>

  <main class="workspace">
    <section
      class="panel panel--code"
      class:is-collapsed={leftCollapsed}
      style="flex:0 0 {leftCollapsed ? 0 : leftWidth}px"
    >
      <header class="panel__head">
        <span class="panel__title">Sketch</span>
        <button
          class="icon-btn"
          title="Collapse the sketch panel"
          aria-label="Collapse the sketch panel"
          onclick={() => (leftCollapsed = true)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html CHEVRON_LEFT}</svg>
        </button>
      </header>
      <div class="panel__body panel__body--code">
        <CodeEditor bind:value={editor.text} {jump} />
      </div>
    </section>

    {#if leftCollapsed}
      <aside class="rail rail--left">
        <button
          class="rail__btn"
          title="Sketch"
          aria-label="Expand the sketch panel"
          onclick={() => (leftCollapsed = false)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html ICON_SKETCH}</svg>
        </button>
      </aside>
    {:else}
      <div
        class="splitter"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the sketch panel"
        onpointerdown={(e) => startResize(e, 'left')}
      ></div>
    {/if}

    <section class="panel panel--canvas">
      <header class="panel__head">
        <span class="panel__title">Circuit</span>
        {#if selectedWireId}
          <div class="wire-colours">
            {#each WIRE_COLORS as c (c.name)}
              <button
                class="wire-colour"
                class:wire-colour--active={wires.find((w) => w.id === selectedWireId)?.colour === c.name}
                style="--swatch: {c.hex}"
                title={c.name}
                aria-label="Wire colour: {c.name}"
                onclick={() => setWireColour(c.name)}
              ></button>
            {/each}
          </div>
        {/if}
        <button class="icon-btn" title="Toggle the serial monitor" aria-label="Toggle the serial monitor" onclick={() => (showSerial = !showSerial)}>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 6h16M4 10h16M4 14h10" />
          </svg>
        </button>
        {#if selectedId}
          <button class="icon-btn" title="Rotate 90° (R)" aria-label="Rotate part" onclick={() => rotatePart(1)}>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v4h-4" />
            </svg>
          </button>
        {/if}
        <button class="icon-btn" title="Fit view to circuit" aria-label="Fit view to circuit" onclick={fitView}>
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
          </svg>
        </button>
        <button
          class="icon-btn"
          disabled={running}
          title={running ? 'Stop the simulation to clear the circuit' : 'Clear the canvas'}
          aria-label="Clear the canvas"
          onclick={clearCanvas}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">
            <path d="M5 7h14M9 7V5h6v2M7 7l1 12h8l1-12" />
          </svg>
        </button>
        <div class="zoom">
          <button class="icon-btn" title="Zoom out (Ctrl -)" aria-label="Zoom out" onclick={zoomOut}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 12h12" /></svg>
          </button>
          <button class="zoom__level" title="Reset zoom to 100% (Ctrl 0)" onclick={resetZoom}>
            {Math.round(zoom * 100)}%
          </button>
          <button class="icon-btn" title="Zoom in (Ctrl +)" aria-label="Zoom in" onclick={zoomIn}>
            <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 6v12M6 12h12" /></svg>
          </button>
        </div>
        <span class="spacer"></span>
        {#if running}
          <button class="btn" title="Recompile and restart with the current sketch" onclick={restartSim}>
            Restart
          </button>
          <button class="btn" onclick={stopSim}>Stop</button>
        {:else}
          <button
            class="btn btn--primary"
            disabled={!inTauri}
            title={inTauri ? 'Compile and run' : 'Available in the desktop app'}
            onclick={runSim}
          >
            Run
          </button>
        {/if}
      </header>
      <!-- The selected part's properties, docked under the circuit bar. Always
           present (so the canvas never jumps) and generic: the catalogue says
           what's editable. -->
      <div class="props">
        {#if selectedPart}
          {@const def = partByKind(selectedPart.kind)}
          <span class="props__name">{def.label}</span>
          {#if warnings[selectedPart.id]}
            <span class="props__warn">{warnings[selectedPart.id]}</span>
          {/if}
          {@const attrs = attrsOf(selectedPart)}
          {#each def.fields ?? [] as field (`${selectedPart.id}:${field.name}`)}
            {@const id = `prop-${selectedPart.id}-${field.name}`}
            <span class="props__field">
              <label class="props__label" for={id}>{field.label}</label>
              {#if field.control.kind === 'select'}
                {@const current = field.control.options.find((o) => o.value === attrs[field.name])}
                <span class="props__control">
                  {#if current?.swatch}
                    <span class="props__swatch" style="background:{current.swatch}"></span>
                  {/if}
                  <select
                    {id}
                    class="props__input props__select"
                    value={attrs[field.name] ?? ''}
                    disabled={running}
                    onchange={(e) => setAttr(selectedPart, field.name, e.currentTarget.value)}
                  >
                    {#each field.control.options as opt (opt.value)}
                      <option value={opt.value}>{opt.label}</option>
                    {/each}
                  </select>
                </span>
              {:else if field.control.kind === 'range'}
                <span class="props__control">
                  <input
                    {id}
                    class="props__range"
                    type="range"
                    min={field.control.min}
                    max={field.control.max}
                    step={field.control.step}
                    value={attrs[field.name] ?? String(field.control.min)}
                    oninput={(e) => setRange(selectedPart, field.name, Number(e.currentTarget.value))}
                    onchange={endRange}
                    onpointerup={endRange}
                    onkeyup={endRange}
                  />
                  <span class="props__readout">{attrs[field.name] ?? field.control.min}</span>
                </span>
              {:else}
                <span class="props__control">
                  <input
                    {id}
                    class="props__input props__number"
                    type="text"
                    inputmode="decimal"
                    spellcheck="false"
                    bind:value={resText}
                    disabled={running}
                    onchange={() => commitResistance(field, resText)}
                  />
                  <select
                    class="props__input props__select props__select--unit"
                    aria-label={`${field.label} unit`}
                    value={String(resUnit)}
                    disabled={running}
                    onchange={(e) => pickUnit(Number(e.currentTarget.value))}
                  >
                    {#each field.control.units as unit (unit.factor)}
                      <option value={String(unit.factor)}>{unit.label}</option>
                    {/each}
                  </select>
                </span>
              {/if}
            </span>
          {/each}
        {:else}
          <span class="props__hint">Select a part to change its properties.</span>
        {/if}
      </div>

      {#if running}
        <div class="canvas-note">Simulation running — the circuit is frozen. Stop to edit it.</div>
      {/if}
      {#if renderError}
        <div class="render-error">{renderError}</div>
      {/if}
      {#if fileError}
        <div class="file-error" role="alert" title={fileError}>{fileError}</div>
      {/if}
      <div
        class="panel__body panel__body--canvas"
        class:is-wiring={!!wiring || !!rerouting}
        role="application"
        aria-label="Circuit canvas"
        tabindex="-1"
        bind:this={surface.canvasEl}
        onpointerdown={(e) => input.onCanvasPointerDown(e)}
        onpointermove={(e) => input.onCanvasHover(e)}
        oncontextmenu={openContextMenu}
      >
        <canvas class="circuit-canvas" bind:this={surface.renderCanvas}></canvas>
        {#if savedNote}
          <div class="saved-note" role="status">Saved</div>
        {/if}
        {#if marquee}
          <div
            class="marquee"
            style="left:{marquee.left}px;top:{marquee.top}px;width:{marquee.width}px;height:{marquee.height}px"
          ></div>
        {/if}
        <div
          class="canvas-content"
          style="transform: {zoom === 1 && panX === 0 && panY === 0
            ? 'none'
            : `translate(${panX}px, ${panY}px) scale(${zoom})`}"
        >
          {#if zoom >= 0.34}
            <svg class="grid" aria-hidden="true">
              <defs>
                <pattern id="grid-coarse" width="24" height="24" patternUnits="userSpaceOnUse">
                  <path
                    d="M24 0H0V24"
                    fill="none"
                    style="stroke: var(--border); stroke-width: 1; opacity: 0.3"
                  />
                </pattern>
                <pattern id="grid-fine" width="12" height="12" patternUnits="userSpaceOnUse">
                  <path
                    d="M12 0H0V12"
                    fill="none"
                    style="stroke: var(--border); stroke-width: 1; opacity: 0.13"
                  />
                </pattern>
              </defs>
              <rect width="100%" height="100%" fill="url(#grid-coarse)" />
              {#if zoom >= 0.67}
                <rect width="100%" height="100%" fill="url(#grid-fine)" />
              {/if}
            </svg>
          {/if}

        <!-- Wire hit-targets live under the parts, so they never block pins. -->
        <svg class="wires" aria-hidden="true">
          {#each wires as w (w.id)}
            {@const pts = input.wirePoints(w)}
            {#if pts.length >= 2}
              <path
                class="wire__hit"
                data-wire={w.id}
                role="button"
                aria-label="Select or reshape wire"
                tabindex="-1"
                d={routePath(pts)}
                onpointerdown={(e) => input.startWireDrag(e, w.id)}
              />
            {/if}
          {/each}
        </svg>

        <div class="canvas-layer">
          {#each renderOrder as inst (inst.id)}
            {@const def = partByKind(inst.kind)}
            <div
              class="instance"
              class:instance--selected={selectedIds.includes(inst.id)}
              data-id={inst.id}
              role="button"
              aria-label={def.label}
              tabindex="0"
              style="transform: translate({input.domX(inst)}px, {input.domY(inst)}px) rotate({inst.rotate ?? 0}deg)"
              onpointerdown={(e) => input.startMove(e, inst)}
            >
              <!-- One element for every part, named by the catalogue — so adding
                   a component is a catalogue entry, not a branch here. -->
              <svelte:element this={def.tag} use:liveState={liveProp(inst)} {...attrsOf(inst)}
              ></svelte:element>

              <!-- A breadboard's 400 holes are hit-tested from data, not DOM. -->
              {#if !def.skipDomPins}
                {#each pinsById[inst.id] ?? [] as p (p.name)}
                  <div
                    class="pin"
                    data-pin={p.name}
                    aria-hidden="true"
                    title={p.name}
                    style="left:{p.x}px; top:{p.y}px"
                  ></div>
                {/each}
              {/if}
            </div>
          {/each}
        </div>

        <!-- Visible wires (and the live one, snap ring, and endpoint handles)
             render above the parts, like Wokwi. -->
        <svg class="wires" aria-hidden="true">
          {#each wires as w (w.id)}
            {@const pts = input.wirePoints(w)}
            {#if pts.length >= 2}
              <path
                class="wire__line"
                class:wire__line--selected={selectedWireId === w.id}
                style="stroke: {wireHex(w.colour)}"
                d={routePath(pts)}
              />
            {/if}
          {/each}

          {#if wiring}
            {@const d = routePath(input.wiringPoints)}
            {#if d}
              <path class="wire__line wire__line--preview" d={d} />
            {/if}
            {@const snap = input.wiringTarget ? pinPoint(input.wiringTarget) : null}
            {#if snap}
              <circle class="wire__snap" cx={snap.x} cy={snap.y} r="7" />
            {/if}
          {/if}

          {#if selectedWireId}
            {@const w = wires.find((x) => x.id === selectedWireId)}
            {#if w}
              {@const a = pinPoint(w.a)}
              {@const b = pinPoint(w.b)}
              {#if a}
                <circle
                  class="wire__handle"
                  role="button"
                  aria-label="Wire endpoint"
                  tabindex="-1"
                  cx={a.x}
                  cy={a.y}
                  r="5"
                  onpointerdown={(e) => input.startEndpointDrag(e, w.id, 'a')}
                />
              {/if}
              {#if b}
                <circle
                  class="wire__handle"
                  role="button"
                  aria-label="Wire endpoint"
                  tabindex="-1"
                  cx={b.x}
                  cy={b.y}
                  r="5"
                  onpointerdown={(e) => input.startEndpointDrag(e, w.id, 'b')}
                />
              {/if}
            {/if}
          {/if}
        </svg>

        </div>

        {#if instances.length === 0}
          <p class="placeholder placeholder--center">Drag a component to start.</p>
        {/if}
      </div>

      {#if showSerial}
        <div class="serial">
          <header class="panel__head">
            <span class="panel__title">Serial Monitor</span>
            <button
              class="icon-btn"
              title="Close the serial monitor"
              aria-label="Close the serial monitor"
              onclick={() => (showSerial = false)}
            >
              <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M6 6l12 12M18 6L6 18" /></svg>
            </button>
          </header>
          {#if buildError}
            {#if compileErrors.length}
              <div class="serial__errors">
                {#each compileErrors as error}
                  <button
                    class="diagnostic"
                    class:diagnostic--warning={error.severity === 'warning'}
                    class:diagnostic--note={error.severity === 'note'}
                    type="button"
                    disabled={!isSketchFile(error.file)}
                    title={isSketchFile(error.file) ? 'Jump to this line in the sketch' : error.file}
                    onclick={() => jumpToError(error)}
                  >
                    <span class="diagnostic__where">
                      {#if isSketchFile(error.file)}
                        line {error.line}{#if error.column}:{error.column}{/if}
                      {:else}
                        {error.file}
                      {/if}
                    </span>
                    <span class="diagnostic__msg">{error.message}</span>
                  </button>
                {/each}
              </div>
            {:else}
              <pre class="serial__out serial__out--error">{buildError}</pre>
            {/if}
          {:else}
            <pre class="serial__out">{serial || (running ? '' : 'No output.')}</pre>
          {/if}
          <form
            class="serial__send"
            onsubmit={(e) => {
              e.preventDefault()
              sendSerial()
            }}
          >
            <input
              class="serial__input"
              bind:value={serialInput}
              placeholder="Send to sketch…"
              aria-label="Serial input"
              autocomplete="off"
              disabled={!running || !inTauri}
            />
            <label class="serial__newline">
              <input type="checkbox" bind:checked={serialNewline} disabled={!running || !inTauri} />
              newline
            </label>
            <button class="btn" type="submit" disabled={!running || !inTauri || !serialInput}>
              Send
            </button>
          </form>
        </div>
      {/if}
    </section>

    {#if rightCollapsed}
      <aside class="rail rail--right">
        <button
          class="rail__btn"
          title="Components"
          aria-label="Expand the components panel"
          onclick={() => (rightCollapsed = false)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html ICON_COMPONENTS}</svg>
        </button>
      </aside>
    {:else}
      <div
        class="splitter"
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize the components panel"
        onpointerdown={(e) => startResize(e, 'right')}
      ></div>
    {/if}

    <section
      class="panel panel--components"
      class:is-collapsed={rightCollapsed}
      style="flex:0 0 {rightCollapsed ? 0 : rightWidth}px"
    >
      <header class="panel__head">
        <span class="panel__title">Components</span>
        <button
          class="icon-btn"
          title="Collapse the components panel"
          aria-label="Collapse the components panel"
          onclick={() => (rightCollapsed = true)}
        >
          <svg viewBox="0 0 24 24" aria-hidden="true">{@html CHEVRON_RIGHT}</svg>
        </button>
      </header>
      <div class="panel__body">
        <ul class="parts" bind:this={surface.partsListEl}>
          {#each PARTS as part (part.kind)}
            <li
              class="part"
              class:is-locked={running}
              data-kind={part.kind}
              title={running ? 'Stop the simulation to add parts' : part.label}
              onpointerdown={(e) => input.startPaletteDrag(e, part)}
            >
              {#if partArt[part.kind]}
                <img
                  class="part__thumb"
                  src={partArt[part.kind].href}
                  alt=""
                  draggable="false"
                />
              {:else}
                <svg class="part__icon" viewBox="0 0 24 24" aria-hidden="true">{@html part.icon}</svg>
              {/if}
              <!-- The real element, hidden: it's what the thumbnail is drawn from. -->
              <span class="part__art" aria-hidden="true">
                <svelte:element this={part.tag} {...part.attrs}></svelte:element>
              </span>
              <span class="part__label">{part.label}</span>
            </li>
          {/each}
        </ul>
      </div>
    </section>
  </main>

  {#if dragging}
    <div class="drag-ghost" style="left:{dragging.x}px; top:{dragging.y}px">
      <svg viewBox="0 0 24 24" aria-hidden="true">{@html dragging.def.icon}</svg>
    </div>
  {/if}

  {#if menu}
    <div
      class="menu"
      style="left:{menu.x}px; top:{menu.y}px"
    >
      {#if menu.partId}
        <button class="menu__item" disabled={running} onclick={() => runMenu(copySelection)}>
          Copy
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(cutSelection)}>
          Cut
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(duplicateSelection)}>
          Duplicate
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(() => rotatePart(1))}>
          Rotate 90°
        </button>
        <button class="menu__item" disabled={running} onclick={() => runMenu(() => rotatePart(2))}>
          Rotate 180°
        </button>
        {#if menuIsMcu()}
          <div class="menu__rule"></div>
          <button class="menu__item" onclick={editSketch}>Edit code</button>
        {/if}
        <div class="menu__rule"></div>
        <button
          class="menu__item menu__item--danger"
          disabled={running}
          onclick={() => runMenu(deleteSelection)}
        >
          Delete
        </button>
      {:else}
        <button
          class="menu__item menu__item--danger"
          disabled={running}
          onclick={() => runMenu(deleteSelection)}
        >
          Delete wire
        </button>
      {/if}
      {#if running}
        <div class="menu__note">Circuit frozen while running</div>
      {/if}
    </div>
  {/if}

  {#if hoveredPin && hoverPos}
    {@const tip = pinTooltip(hoveredPin)}
    {#if tip}
      <div
        class="pin-tip"
        style="left:{Math.min(hoverPos.x + 14, window.innerWidth - 240)}px; top:{hoverPos.y + 16}px"
      >
        <span class="pin-tip__part">{tip.part}</span>
        <span class="pin-tip__name">{tip.name}</span>
        {#if tip.note}<span class="pin-tip__note">{tip.note}</span>{/if}
      </div>
    {/if}
  {/if}
</div>

<style>
  .app {
    display: grid;
    grid-template-rows: auto 1fr;
    height: 100vh;
  }

  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 40px;
    padding: 0 var(--space-4);
    background: var(--surface);
    border-bottom: 1px solid var(--border);
  }

  .brand {
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .spacer {
    flex: 1;
  }

  .workspace {
    display: flex;
    min-height: 0;
  }

  .panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface);
  }

  /* A collapsed panel keeps its content mounted (so the editor keeps its
     history) but clips it to nothing; the rail beside it takes over. */
  .panel.is-collapsed {
    overflow: hidden;
  }

  .panel--canvas {
    flex: 1 1 auto;
    background: var(--bg);
  }

  .panel__head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 32px;
    padding: 0 var(--space-2) 0 var(--space-3);
    font-size: var(--fs-ui-dense);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
  }

  .panel__title {
    flex: 1;
  }

  .panel__body {
    flex: 1;
    min-height: 0;
    padding: var(--space-3);
    overflow: auto;
  }

  .panel__body--code {
    padding: 0;
    overflow: hidden;
  }

  .panel__body--canvas {
    position: relative;
    display: grid;
    place-items: center;
    padding: 0;
    overflow: hidden;
  }

  /* The circuit's DOM lives in here for hit-testing, but the visible pixels come
     from the canvas — so the DOM itself is kept transparent. */
  .canvas-content {
    position: absolute;
    inset: 0;
    transform-origin: 0 0;
    opacity: 0;
  }

  /* The visible circuit, drawn at full device resolution. */
  .circuit-canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  /* The selected part's properties, docked under the circuit bar. */
  .props {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex: none;
    min-height: 30px;
    padding: 3px var(--space-3);
    font-size: var(--fs-ui-dense);
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }

  .props__name {
    color: var(--text);
  }

  /* A part that's wired wrong says so, in the strip, in the warning colour. */
  .props__warn {
    color: var(--warning);
  }

  .props__field {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .props__control {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  /* Both controls share one look: raised surface, hairline border, no shadow. */
  .props__input {
    font-family: inherit;
    font-size: 11px;
    padding: 2px 6px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
  }

  .props__input:disabled {
    color: var(--text-muted);
    cursor: not-allowed;
  }

  /* The platform arrow is replaced by a caret we can colour, so the control reads
     the same in both themes instead of depending on the GTK widget's styling. */
  .props__select {
    appearance: none;
    padding-right: 19px;
    background-image:
      linear-gradient(45deg, transparent 50%, currentColor 50%),
      linear-gradient(135deg, currentColor 50%, transparent 50%);
    background-position:
      right 8px top 55%,
      right 4px top 55%;
    background-size:
      4px 4px,
      4px 4px;
    background-repeat: no-repeat;
  }

  .props__select--unit {
    padding-right: 16px;
    background-position:
      right 6px top 55%,
      right 2px top 55%;
  }

  .props__number {
    width: 5.5em;
    font-family: var(--font-mono);
  }

  .props__range {
    width: 7em;
    accent-color: var(--accent);
    cursor: ew-resize;
  }

  /* The value a drag lands on, so a position is a number and not a guess. */
  .props__readout {
    min-width: 3.5em;
    font-family: var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }

  .props__swatch {
    width: 9px;
    height: 9px;
    border: 1px solid var(--border);
  }

  .props__hint {
    color: var(--text-muted);
  }

  /* The circuit is frozen while a sketch runs — say so, don't leave it a mystery. */
  /* The right-drag selection box. In the DOM rather than on the canvas because it is
     screen-space and has no circuit geometry to speak of. Hollow, so it reads as a
     selection rather than as a shape. */
  .marquee {
    position: absolute;
    z-index: 2;
    border: 1px solid var(--accent);
    pointer-events: none;
  }

  /* Floated rather than laid out, so it cannot shift the circuit when it comes and
     goes — and with no transition on purpose: WebKitGTK drops to low-resolution tiles
     while any animation runs, which blurs the whole canvas. It pops, then it is gone. */
  .saved-note {
    position: absolute;
    left: var(--space-3);
    bottom: var(--space-3);
    z-index: 3;
    padding: 3px var(--space-3);
    font-size: 11px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    pointer-events: none;
  }

  .canvas-note {
    flex: none;
    padding: 4px var(--space-3);
    font-size: 11px;
    color: var(--text-muted);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* The right-click menu: in-app, tokens only, and floated with a border
     rather than a shadow — the app has no shadows anywhere. */
  .menu {
    position: fixed;
    z-index: 120;
    min-width: 176px;
    padding: var(--space-1) 0;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-panel);
  }

  /* A dropdown anchored under its menu-bar title. */
  .menu--anchored {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 216px;
  }

  /* The menu bar: only commands that have no other home live in here. */
  .menubar {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-left: var(--space-3);
  }

  .menubar__item {
    position: relative;
  }

  .menubar__btn {
    font-family: inherit;
    font-size: var(--fs-ui-dense);
    padding: 3px var(--space-2);
    color: var(--text);
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .menubar__btn:hover,
  .menubar__btn.is-open {
    background: var(--surface-raised);
    border-color: var(--border);
  }

  .menu__item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-4);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    font-family: inherit;
    font-size: 12px;
    text-align: left;
    color: var(--text);
    background: none;
    border: 0;
    cursor: pointer;
  }

  .menu__item:hover:not(:disabled) {
    background: var(--surface);
  }

  .menu__item:disabled {
    color: var(--text-muted);
    opacity: 0.5;
    cursor: not-allowed;
  }

  .menu__item--danger:hover:not(:disabled) {
    color: var(--error);
  }

  .menu__rule {
    height: 1px;
    margin: var(--space-1) 0;
    background: var(--border);
  }

  .menu__note {
    padding: var(--space-1) var(--space-3) var(--space-2);
    font-size: 11px;
    color: var(--text-muted);
  }

  /* The keyboard shortcut, shown the way a menu bar normally shows it. */
  .menu__key {
    font-size: 11px;
    color: var(--text-muted);
  }

  /* The pin under the pointer: which pin it is, and what it means where the
     element tells us. Most of the value is on multi-pin parts. */
  .pin-tip {
    position: fixed;
    z-index: 130;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: 3px var(--space-2);
    font-size: 11px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    pointer-events: none;
    white-space: nowrap;
  }

  .pin-tip__part {
    color: var(--text-muted);
  }

  .pin-tip__name {
    font-family: var(--font-mono);
  }

  .pin-tip__note {
    color: var(--text-muted);
  }

  /* A part that couldn't be rasterised — shows why. */
  .render-error {
    flex: none;
    padding: 4px var(--space-3);
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--error);
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* A save or open that did not happen. Docked, like every other message here. */
  .file-error {
    flex: none;
    padding: 4px var(--space-3);
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--error);
    border-bottom: 1px solid var(--error);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Draggable column divider — a thin line with a forgiving hit area. */
  .splitter {
    flex: 0 0 6px;
    position: relative;
    cursor: col-resize;
    touch-action: none;
  }

  .splitter::before {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    transform: translateX(-50%);
    background: var(--border);
    transition: background var(--dur-micro) var(--ease);
  }

  .splitter:hover::before {
    background: var(--accent);
  }

  /* Collapsed panel rail. */
  .rail {
    flex: 0 0 40px;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding-top: var(--space-2);
    background: var(--surface);
  }

  .rail--left {
    border-right: 1px solid var(--border);
  }

  .rail--right {
    border-left: 1px solid var(--border);
  }

  .rail__btn {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    color: var(--text-muted);
    background: transparent;
    border: 0;
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .rail__btn:hover {
    color: var(--text);
    background: var(--surface-raised);
  }

  .rail__btn svg {
    width: 20px;
    height: 20px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .icon-btn {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    color: var(--text-muted);
    background: transparent;
    border: 0;
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .icon-btn:hover {
    color: var(--text);
    background: var(--surface-raised);
  }

  .icon-btn svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .zoom {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .wire-colours {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-right: var(--space-1);
  }

  .wire-colour {
    width: 12px;
    height: 12px;
    padding: 0;
    background: var(--swatch);
    border: 1px solid var(--border);
    border-radius: 2px;
    cursor: pointer;
  }

  .wire-colour--active {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }

  .zoom__level {
    min-width: 46px;
    padding: 2px 4px;
    font: inherit;
    font-size: var(--fs-ui-dense);
    font-variant-numeric: tabular-nums;
    color: var(--text-muted);
    background: transparent;
    border: 0;
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .zoom__level:hover {
    color: var(--text);
    background: var(--surface-raised);
  }

  /* Faint orthogonal grid. Drawn as SVG patterns rather than tiled CSS
     gradients: CSS background tiles get rasterised with rounding that
     accumulates across the surface (the fine grid visibly drifts away from the
     top-left). SVG patterns are exact geometry, so the grids stay locked. */
  .grid {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  /* Wire layers. Explicit size matters: an unsized <svg> collapses to 300x150
     and clips anything drawn outside it. */
  .wires {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .wire__hit {
    fill: none;
    stroke: transparent;
    stroke-width: 14;
    pointer-events: stroke;
    cursor: pointer;
  }

  .wire__line {
    fill: none;
    stroke: var(--wire);
    stroke-width: 2.5;
    stroke-linejoin: round;
    stroke-linecap: round;
  }

  .wire__line--selected {
    stroke-width: 4;
  }

  .wire__line--preview {
    stroke-dasharray: 4 4;
    opacity: 0.85;
  }

  .wire__handle {
    fill: var(--accent);
    stroke: var(--bg);
    stroke-width: 1.5;
    pointer-events: all;
    cursor: grab;
  }

  .wire__snap {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2;
    pointer-events: none;
  }

  /* Positioned layer holding placed parts. Passes clicks through empty space. */
  .canvas-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .instance {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 50% 50%;
    pointer-events: auto;
    cursor: grab;
    touch-action: none;
  }

  /* The elements exist only to be measured and hit-tested — the visible circuit
     is the canvas above them. `visibility: hidden` keeps them out of the *paint*
     (a breadboard is ~860 nodes, and dragging one re-rasterised all of it every
     frame) while keeping the layout size the rasteriser reads. */
  .instance > *:first-child {
    visibility: hidden;
  }

  .instance--selected {
    outline: 2px solid var(--accent);
    outline-offset: 3px;
  }

  /* Pins: small square markers, like Wokwi — visible on the part, brighter on
     hover. The wire always snaps to the pin's centre, never where you clicked. */
  .pin {
    position: absolute;
    width: 7px;
    height: 7px;
    margin: -3.5px 0 0 -3.5px;
    background: var(--wire);
    border: 1px solid var(--bg);
    border-radius: 0;
    opacity: 0.5;
    cursor: crosshair;
  }

  .is-wiring .pin {
    opacity: 1;
  }

  .pin:hover {
    transform: scale(1.5);
    background: var(--accent);
  }

  /* Keep the hit area close to the pin's own size, so neighbouring pins (9.5px
     apart on the Uno) don't overlap. Pressing near a pin still snaps to the
     nearest one, so it stays easy to start a wire. */
  .pin::before {
    content: '';
    position: absolute;
    inset: -1px;
  }

  .drag-ghost {
    position: fixed;
    width: 24px;
    height: 24px;
    transform: translate(-50%, -50%);
    pointer-events: none;
    color: var(--accent);
    z-index: 100;
  }

  .drag-ghost svg {
    width: 100%;
    height: 100%;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .placeholder {
    margin: 0;
    color: var(--text-muted);
    font-size: var(--fs-ui-dense);
  }

  .placeholder--center {
    position: relative;
  }

  /* The serial monitor: a docked strip under the canvas (never a popup). */
  .serial {
    display: flex;
    flex-direction: column;
    height: 170px;
    flex: none;
    background: var(--surface);
    border-top: 1px solid var(--border);
  }

  .serial__out {
    flex: 1;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    overflow: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-code);
    line-height: var(--lh-code);
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text);
  }

  .serial__out--error {
    color: var(--error);
  }

  .serial__errors {
    flex: 1;
    margin: 0;
    padding: 0;
    overflow: auto;
  }

  .diagnostic {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-1) var(--space-3);
    border: 0;
    border-bottom: 1px solid var(--border);
    background: none;
    color: var(--error);
    font-family: var(--font-mono);
    font-size: var(--fs-code);
    line-height: var(--lh-code);
    text-align: left;
    cursor: pointer;
  }

  .diagnostic:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text) 7%, transparent);
  }

  .diagnostic:disabled {
    color: var(--text-muted);
    cursor: default;
  }

  .diagnostic--warning {
    color: var(--warning);
  }

  .diagnostic--note {
    color: var(--text-muted);
  }

  .diagnostic__where {
    flex: none;
    color: var(--text-muted);
  }

  .diagnostic__msg {
    white-space: pre-wrap;
    word-break: break-word;
  }

  .serial__send {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: none;
    padding: var(--space-1) var(--space-3);
    border-top: 1px solid var(--border);
  }

  .serial__input {
    flex: 1;
    min-width: 0;
    padding: 3px var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-control, 2px);
    background: var(--bg);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-code);
  }

  .serial__input:focus-visible {
    outline: 1px solid var(--accent);
    outline-offset: 1px;
  }

  .serial__newline {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--text-muted);
    font-size: var(--fs-ui-dense);
    white-space: nowrap;
  }

  /* Parts: icon over label, in a two-up grid. */
  .parts {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-2);
  }

  .part.is-locked {
    cursor: not-allowed;
    opacity: 0.45;
  }

  .part {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-2);
    text-align: center;
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-tile);
    cursor: grab;
    touch-action: none;
    transition:
      border-color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .part:hover {
    border-color: var(--text-muted);
  }

  .part__icon {
    width: 24px;
    height: 24px;
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
    color: var(--text-muted);
    transition: color var(--dur-micro) var(--ease);
  }

  .part:hover .part__icon {
    color: var(--text);
  }

  /* The real drawing, scaled into a consistent box. */
  .part__thumb {
    max-width: 100%;
    height: 40px;
    object-fit: contain;
  }

  /* The real element behind each thumbnail: laid out (so it can be rasterised)
     but invisible. It fills the tile so a part that sizes itself from its
     container still lays out — `wokwi-resistor` does, and collapsed to 0px in a
     zero-sized box. `overflow: hidden` keeps a wide part (a breadboard is 627px)
     from spilling into the panel. */
  .part__art {
    position: absolute;
    inset: 0;
    overflow: hidden;
    opacity: 0;
    pointer-events: none;
  }

  .part__label {
    font-size: var(--fs-ui-dense);
    line-height: 1.2;
    color: var(--text);
  }

  .btn {
    font: inherit;
    font-size: var(--fs-ui-dense);
    /* The panel headers upper-case their titles; buttons shouldn't inherit that. */
    text-transform: none;
    padding: 4px 10px;
    color: var(--text);
    background: var(--surface-raised);
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    cursor: pointer;
    transition:
      border-color var(--dur-micro) var(--ease),
      background var(--dur-micro) var(--ease);
  }

  .btn:hover:not([disabled]) {
    border-color: var(--text-muted);
  }

  .btn--primary {
    color: var(--accent-contrast);
    background: var(--accent);
    border-color: var(--accent);
  }

  .btn[disabled] {
    opacity: 0.5;
    cursor: default;
  }
</style>
