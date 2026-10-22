#!/usr/bin/env node
/**
 * Drive the real app from a script.
 *
 * Most of this project's actual bugs were invisible in the source and obvious within a
 * minute of driving it — the button latch, six pixels of paste drift, a selection that
 * collapsed when you grabbed a part by its leg. None were findable by reading, and the
 * unit tests cannot reach any of them: they run against pure modules, not the app.
 *
 * This is the harness those were found with, kept instead of thrown away each time.
 *
 *   npm run drive              run the smoke checks below
 *   npm run drive -- --help    what else it can do
 *
 * To use it as a library, import `connect` and drive whatever you are working on:
 *
 *   import { connect } from './scripts/drive.mjs'
 *   const app = await connect()
 *   await app.goto('../')
 *   await app.click(...)      // screen coordinates
 *   console.log(await app.ev('...'))   // any expression, evaluated in the page
 *   await app.close()
 *
 * It starts what it needs and kills only what it started, by pid. It never touches
 * port 5173: that is the development port, and something important may be on it.
 */

import { spawn } from 'node:child_process'
import { createServer } from 'node:net'

const DEV_PORT = 5199 // deliberately not 5173 — see the README's development notes
const CDP_PORT = 9222
const sleep = (ms) => new Promise((r) => setTimeout(r, ms))

/** Is something already listening? */
function listening(port) {
  return new Promise((resolve) => {
    const socket = createServer()
    socket.once('error', () => resolve(true)) // in use
    socket.once('listening', () => {
      socket.close()
      resolve(false)
    })
    socket.listen(port, '127.0.0.1')
  })
}

async function waitFor(url, timeoutMs = 40_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(1500) })
      if (res.ok) return true
    } catch {
      /* not up yet */
    }
    await sleep(300)
  }
  throw new Error(`timed out waiting for ${url}`)
}

/** One DevTools-protocol connection to a page. */
export class App {
  constructor(ws, page) {
    this.ws = ws
    this.page = page
    this.next = 0
    this.pending = new Map()
    this.consoleErrors = []
    ws.addEventListener('message', (event) => {
      const msg = JSON.parse(event.data)
      if (msg.method === 'Runtime.consoleAPICalled' && msg.params.type === 'error') {
        this.consoleErrors.push(msg.params.args.map((a) => a.value ?? a.description).join(' '))
      }
      if (msg.method === 'Runtime.exceptionThrown') {
        const details = msg.params.exceptionDetails
        this.consoleErrors.push(
          'EXCEPTION ' + (details?.exception?.description ?? details?.text ?? ''),
        )
      }
      const resolve = this.pending.get(msg.id)
      if (resolve) {
        this.pending.delete(msg.id)
        resolve(msg)
      }
    })
  }

  send(method, params = {}) {
    const id = ++this.next
    return Promise.race([
      new Promise((resolve) => {
        this.pending.set(id, resolve)
        this.ws.send(JSON.stringify({ id, method, params }))
      }),
      sleep(15_000).then(() => {
        throw new Error(`timed out: ${method}`)
      }),
    ])
  }

  /** Evaluate an expression in the page and return its value. */
  async ev(expression) {
    const reply = await this.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })
    if (reply.result?.exceptionDetails) throw new Error(reply.result.exceptionDetails.text ?? 'page error')
    return reply.result?.result?.value
  }

  async goto(url) {
    await this.send('Page.navigate', { url })
    // Wait for the app shell, not a fixed guess: the first load compiles the dev
    // bundle, and a 2.5 s sleep made the first checks race that compile.
    const deadline = Date.now() + 15_000
    while (Date.now() < deadline) {
      try {
        if (await this.ev(`!!document.querySelector('.panel__body--canvas')`)) break
      } catch {
        /* the navigation is still settling */
      }
      await sleep(100)
    }
    await sleep(500) // fonts and the hidden part elements
  }

  mouse(type, x, y, button = 'left', buttons = 0) {
    return this.send('Input.dispatchMouseEvent', { type, x, y, button, buttons, clickCount: type === 'mousePressed' ? 1 : 0 })
  }

  async click(x, y) {
    await this.mouse('mousePressed', x, y, 'left', 1)
    await this.mouse('mouseReleased', x, y, 'left', 0)
  }

  async rightClick(x, y) {
    await this.mouse('mousePressed', x, y, 'right', 2)
    await this.mouse('mouseReleased', x, y, 'right', 0)
  }

  /**
   * Drag from one screen point to another in small steps. Small steps matter: a press
   * near a pin arms a wire and only becomes a drag after a few pixels, so one big jump
   * loses distance.
   */
  async drag(from, to, { button = 'left', buttons = 1, steps = 10 } = {}) {
    await this.mouse('mousePressed', from.x, from.y, button, buttons)
    for (let i = 1; i <= steps; i++) {
      await this.mouse('mouseMoved', from.x + ((to.x - from.x) * i) / steps, from.y + ((to.y - from.y) * i) / steps, button, buttons)
      await sleep(15)
    }
    await this.mouse('mouseReleased', to.x, to.y, button, 0)
  }

  /** Press a key. `modifiers` is CDP's bitmask: 1 alt, 2 ctrl, 4 meta, 8 shift. */
  async key(key, { code = `Key${key.toUpperCase()}`, vk = key.toUpperCase().charCodeAt(0), modifiers = 0 } = {}) {
    const base = { key, code, windowsVirtualKeyCode: vk, nativeVirtualKeyCode: vk, modifiers }
    await this.send('Input.dispatchKeyEvent', { type: 'keyDown', ...base })
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', ...base })
  }

  /** The screen-space rectangle a part's pins occupy — a reliable way to find it. */
  partBox(tag) {
    return this.ev(`(() => {
      const el = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === ${JSON.stringify(tag)})
      if (!el) return null
      const pins = [...el.querySelectorAll('[data-pin]')].map(p => p.getBoundingClientRect())
      if (!pins.length) return null
      const xs = pins.map(r => r.x), ys = pins.map(r => r.y)
      return { x: Math.round(Math.min(...xs)), y: Math.round(Math.min(...ys)) }
    })()`)
  }

  /** The screen-space centre of one named pin on a part. */
  pinAt(tag, pin) {
    return this.ev(`(() => {
      const el = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === ${JSON.stringify(tag)})
      const p = el?.querySelector(${JSON.stringify(`[data-pin="${pin}"]`)})
      if (!p) return null
      const r = p.getBoundingClientRect()
      return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }
    })()`)
  }

  /** The screen-space centre of a part's body. */
  partCentre(tag) {
    return this.ev(`(() => {
      const el = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === ${JSON.stringify(tag)})
      if (!el) return null
      const r = el.getBoundingClientRect()
      return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }
    })()`)
  }

  /** Load one of the shipped circuits by name, the way a person would. */
  async loadPreset(label) {
    const file = await this.ev(`(() => { const b = [...document.querySelectorAll('.menubar__btn')].find(e => e.textContent.trim() === 'File'); const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } })()`)
    await this.click(file.x, file.y)
    await sleep(350)
    // Presets are behind a disclosure; open it before looking for the tab.
    const presets = await this.ev(`(() => { const b = [...document.querySelectorAll('.menu__item')].find(e => e.textContent.trim().startsWith('Presets')); if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } })()`)
    if (!presets) throw new Error('no Presets item')
    await this.click(presets.x, presets.y)
    await sleep(150)
    for (const group of ['Basics', 'Sensors', 'Outputs']) {
      const tab = await this.ev(`(() => { const b = [...document.querySelectorAll('.menu__tab')].find(e => e.textContent.trim() === ${JSON.stringify(group)}); if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } })()`)
      if (!tab) continue
      await this.click(tab.x, tab.y)
      await sleep(100)
      const item = await this.ev(`(() => { const b = [...document.querySelectorAll('.menu__item')].find(e => e.textContent.trim() === ${JSON.stringify(label)}); if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } })()`)
      if (item) {
        await this.click(item.x, item.y)
        await sleep(900)
        return
      }
    }
    throw new Error(`no preset called ${label}`)
  }

  /** The screen-space centre of a palette tile. */
  async paletteTile(kind) {
    // The palette scrolls; a newly added part can push a tile below the fold.
    // Scroll it into view before measuring, or the drag starts from a stale
    // off-screen coordinate.
    await this.ev(`document.querySelector('.part[data-kind="${kind}"]')?.scrollIntoView({ block: 'center' })`)
    await sleep(100)
    return this.ev(`(() => {
      const el = document.querySelector('.part[data-kind="${kind}"]')
      if (!el) return null
      const r = el.getBoundingClientRect()
      return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }
    })()`)
  }

  /** The screen-space centre of the circuit canvas panel. */
  async canvasCentre() {
    return this.ev(`(() => {
      const r = document.querySelector('.panel__body--canvas').getBoundingClientRect()
      return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }
    })()`)
  }

  async close() {
    try {
      this.ws.close()
    } catch {
      /* already gone */
    }
  }
}

/**
 * Start whatever is missing and connect. Returns an `App` with `stop()` on it, which
 * kills only the processes this call started.
 */
export async function connect({ port = DEV_PORT, cdpPort = CDP_PORT, startServer = true, startBrowser = true } = {}) {
  const children = []
  const stop = () => {
    for (const child of children) {
      try {
        process.kill(-child.pid, 'SIGTERM')
      } catch {
        /* already gone */
      }
    }
  }
  // Own process groups, so killing a child never reaches anything else.
  const launch = (cmd, args) => {
    const child = spawn(cmd, args, { detached: true, stdio: 'ignore' })
    children.push(child)
    return child
  }

  if (await listening(port)) {
    if (!(await fetchUrl(`http://localhost:${port}`))) throw new Error(`port ${port} is busy with something that is not the app`)
  } else if (startServer) {
    console.log(`  starting the dev server on ${port}`)
    launch('npm', ['run', 'dev', '--', '--port', String(port), '--strictPort'])
    await waitFor(`http://localhost:${port}`)
  }

  if (!(await listening(cdpPort)) && startBrowser) {
    console.log(`  starting chromium on ${cdpPort}`)
    launch('chromium', [
      '--headless=new',
      '--no-sandbox',
      '--disable-dev-shm-usage',
      `--remote-debugging-port=${cdpPort}`,
      '--window-size=1500,950',
      'about:blank',
    ])
    await waitFor(`http://127.0.0.1:${cdpPort}/json/version`)
  }

  const targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json/list`)).json()
  const page = targets.find((t) => t.type === 'page')
  if (!page) throw new Error('no page target — is the browser up?')

  const ws = new WebSocket(page.webSocketDebuggerUrl)
  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve, { once: true })
    ws.addEventListener('error', reject, { once: true })
  })
  const app = new App(ws, page)
  await app.send('Runtime.enable')
  await app.send('Page.enable')
  app.stop = () => {
    app.close()
    stop()
  }
  return app
}

async function fetchUrl(url) {
  try {
    return (await fetch(url, { signal: AbortSignal.timeout(1500) })).ok
  } catch {
    return false
  }
}

// ---------------------------------------------------------------------------
// The smoke checks. Each one covers something the unit tests cannot reach,
// because it only exists once the app is running.
// ---------------------------------------------------------------------------

const checks = []
const check = (name, fn) => checks.push({ name, fn })
const asserts = []
const assert = (ok, description) => asserts.push({ ok: !!ok, description })

check('the app loads, empty', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 0, 'starts with an empty canvas')
  assert(await app.ev(`!!document.querySelector('.panel__body--canvas')`), 'the canvas exists')
})

check('the view opens centred, not in the corner', async (app) => {
  // The origin in the corner is what made parts impossible to move up or left.
  const transform = await app.ev(`document.querySelector('.canvas-content')?.style.transform ?? ''`)
  const [, x, y] = /translate\((-?[\d.]+)px,\s*(-?[\d.]+)px\)/.exec(transform) ?? []
  assert(x !== undefined && Number(x) > 0 && Number(y) > 0, `world origin starts away from the corner (${transform || 'none'})`)
})

check('a preset loads', async (app) => {
  await app.loadPreset('Blink')
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 3, 'Blink gives three parts')
})

check('the Fade preset loads with a PWM LED', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Fade')
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 3, 'Fade gives three parts')
  assert(await app.ev(`!!document.querySelector('wokwi-led')`), 'the LED is on the canvas')
})

check('the output presets load', async (app) => {
  const cases = [
    ['Seven segment', 2, 'wokwi-7segment'],
    ['Four digit', 2, 'wokwi-7segment'],
    ['LED bar', 2, 'wokwi-led-bar-graph'],
    ['Buzzer & relay', 3, 'wokwi-buzzer'],
  ]
  for (const [title, parts, tag] of cases) {
    await app.goto(`http://localhost:${DEV_PORT}`)
    await app.loadPreset(title)
    assert((await app.ev(`document.querySelectorAll('.instance').length`)) === parts, `${title} gives ${parts} parts`)
    assert(await app.ev(`!!document.querySelector('${tag}')`), `${tag} is on the canvas`)
  }
})

check('Ctrl+A selects everything', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  await app.ev(`document.querySelector('.panel__body--canvas').focus()`)
  await app.key('a', { code: 'KeyA', vk: 65, modifiers: 2 })
  await sleep(250)
  assert((await app.ev(`document.querySelectorAll('.instance--selected').length`)) === 3, 'all three parts selected')
})

check('the Digital sensors preset loads all four modules', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Digital sensors')
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 5, 'Digital sensors gives five parts')
  for (const tag of [
    'wokwi-tilt-switch',
    'wokwi-pir-motion-sensor',
    'wokwi-flame-sensor',
    'wokwi-small-sound-sensor',
  ]) {
    assert(await app.ev(`!!document.querySelector('${tag}')`), `${tag} is on the canvas`)
  }
})

check('the RGB preset loads with three channels', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('RGB LED')
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 5, 'RGB gives five parts')
  assert(await app.ev(`!!document.querySelector('wokwi-rgb-led')`), 'the RGB LED is on the canvas')
})

check('the servo preset loads with a horn', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Servo')
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 2, 'Servo gives two parts')
  assert(await app.ev(`!!document.querySelector('wokwi-servo')`), 'the servo is on the canvas')
})

check('the ultrasonic preset loads with a sensor', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Ultrasonic')
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 2, 'Ultrasonic gives two parts')
  assert(await app.ev(`!!document.querySelector('wokwi-hc-sr04')`), 'the sensor is on the canvas')
})

check('the joystick preset loads, drags, snaps back and clicks', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Joystick')
  assert((await app.ev(`document.querySelectorAll('.instance').length`)) === 2, 'Joystick gives two parts')
  const centre = await app.partCentre('wokwi-analog-joystick')

  // Drag right: the stick moves, and the press is cancelled (not a click).
  await app.mouse('mousePressed', centre.x, centre.y, 'left', 1)
  await app.mouse('mouseMoved', centre.x + 30, centre.y, 'left', 1)
  await sleep(200)
  assert(
    (await app.ev(`document.querySelector('wokwi-analog-joystick')?.xValue`)) < -0.05,
    'the stick moved right',
  )
  assert(!(await app.ev(`!!document.querySelector('wokwi-analog-joystick')?.pressed`)), 'the drag is not a click')
  await app.mouse('mouseReleased', centre.x + 30, centre.y, 'left', 0)
  await sleep(200)
  assert(
    (await app.ev(`document.querySelector('wokwi-analog-joystick')?.xValue`)) === 0,
    'the stick snapped back to centre',
  )

  // A click with no movement is the button.
  await app.mouse('mousePressed', centre.x, centre.y, 'left', 1)
  await sleep(150)
  assert(await app.ev(`!!document.querySelector('wokwi-analog-joystick')?.pressed`), 'the stick is pressed')
  await app.mouse('mouseReleased', centre.x, centre.y, 'left', 0)
  await sleep(150)
  assert(!(await app.ev(`!!document.querySelector('wokwi-analog-joystick')?.pressed`)), 'the stick released')
})

check('a right-drag selects exactly what is in the box', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  const resistor = await app.partBox('wokwi-resistor')
  const led = await app.partBox('wokwi-led')
  await app.drag(
    { x: Math.min(resistor.x, led.x) - 40, y: Math.min(resistor.y, led.y) - 40 },
    { x: Math.max(resistor.x, led.x) + 90, y: Math.max(resistor.y, led.y) + 90 },
    { button: 'right', buttons: 2 },
  )
  await sleep(400)
  const selected = await app.ev(`JSON.stringify([...document.querySelectorAll('.instance--selected')].map(e => e.firstElementChild.tagName.toLowerCase()).sort())`)
  assert(selected === '["wokwi-led","wokwi-resistor"]', `exactly the two parts in the box (${selected})`)
})

check('a group drags past the old origin corner and keeps its shape', async (app) => {
  // The bug: single drags were clamped at zero while a group was not. With no clamp
  // anywhere, both should move by the same delta and keep their relative positions.
  const resistorBefore = await app.partBox('wokwi-resistor')
  const ledBefore = await app.partBox('wokwi-led')
  await app.drag({ x: resistorBefore.x + 25, y: resistorBefore.y + 5 }, { x: resistorBefore.x - 475, y: resistorBefore.y - 495 })
  await sleep(600)
  const resistorAfter = await app.partBox('wokwi-resistor')
  const ledAfter = await app.partBox('wokwi-led')
  const moved = (a, b) => `${b.x - a.x},${b.y - a.y}`
  assert(moved(resistorBefore, resistorAfter) === moved(ledBefore, ledAfter), `both moved together (${moved(resistorBefore, resistorAfter)})`)
  assert(resistorAfter.x - ledAfter.x === resistorBefore.x - ledBefore.x, 'the group kept its shape')
  assert(resistorAfter.y < resistorBefore.y - 200, 'it went past where the old clamp was')
})

check('clicking two pins wires them, and the same two again removes the wire', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  const count = () => app.ev(`document.querySelectorAll('.wire__line').length`)
  const before = await count()
  // Two pins on the Uno that the preset leaves free, so no wire sits over them.
  const a = await app.pinAt('wokwi-arduino-uno', '12')
  const b = await app.pinAt('wokwi-arduino-uno', '11')
  assert(a && b, 'both free pins are on screen')
  await app.click(a.x, a.y)
  await sleep(200)
  await app.click(b.x, b.y)
  await sleep(400)
  assert((await count()) === before + 1, `a wire was added (${before} -> ${await count()})`)
  await app.click(a.x, a.y)
  await sleep(200)
  await app.click(b.x, b.y)
  await sleep(400)
  assert((await count()) === before, `the same two pins remove it again (${await count()})`)
})

check('undo and redo restore a dragged part', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  const before = await app.partBox('wokwi-resistor')
  await app.drag({ x: before.x + 25, y: before.y + 5 }, { x: before.x + 125, y: before.y + 105 })
  await sleep(400)
  const after = await app.partBox('wokwi-resistor')
  assert(after.x !== before.x || after.y !== before.y, `the drag moved it (${before.x},${before.y} -> ${after.x},${after.y})`)
  await app.ev(`document.querySelector('.panel__body--canvas').focus()`)
  await app.key('z', { code: 'KeyZ', vk: 90, modifiers: 2 })
  await sleep(400)
  const undone = await app.partBox('wokwi-resistor')
  assert(undone.x === before.x && undone.y === before.y, `undo put it back (${undone.x},${undone.y})`)
  await app.key('z', { code: 'KeyZ', vk: 90, modifiers: 10 })
  await sleep(400)
  const redone = await app.partBox('wokwi-resistor')
  assert(redone.x === after.x && redone.y === after.y, `redo moved it again (${redone.x},${redone.y})`)
})

check('copy and paste duplicate the whole selection', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  const count = () => app.ev(`document.querySelectorAll('.instance').length`)
  assert((await count()) === 3, 'starts with three parts')
  await app.ev(`document.querySelector('.panel__body--canvas').focus()`)
  await app.key('a', { code: 'KeyA', vk: 65, modifiers: 2 })
  await app.key('c', { code: 'KeyC', vk: 67, modifiers: 2 })
  await app.key('v', { code: 'KeyV', vk: 86, modifiers: 2 })
  await sleep(500)
  assert((await count()) === 6, `paste gave six parts (${await count()})`)
  assert(
    (await app.ev(`document.querySelectorAll('.instance--selected').length`)) === 3,
    'the pasted copy is selected, not the original',
  )
})

check('R rotates the selected part', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  const centre = await app.partCentre('wokwi-resistor')
  await app.click(centre.x, centre.y)
  await sleep(200)
  await app.key('r', { code: 'KeyR', vk: 82 })
  await sleep(300)
  const transform = await app.ev(`(() => {
    const el = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'wokwi-resistor')
    return el?.style.transform ?? ''
  })()`)
  assert(transform.includes('rotate(90deg)'), `the resistor turned 90 degrees (${transform})`)
})

check('right-clicking a part opens the in-app menu', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  const centre = await app.partCentre('wokwi-led')
  await app.ev(`(() => {
    const el = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'wokwi-led')
    const r = el.getBoundingClientRect()
    el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2 }))
  })()`)
  await sleep(200)
  const items = await app.ev(`[...document.querySelectorAll('.menu__item')].map(b => b.textContent.trim())`)
  assert(Array.isArray(items) && items.length > 0, `the menu opened (${JSON.stringify(items)})`)
  assert(items.some((t) => t.includes('Duplicate')) && items.some((t) => t.includes('Delete')), 'it offers Duplicate and Delete')
  await app.key('Escape', { code: 'Escape', vk: 27 })
  await sleep(200)
  assert((await app.ev(`document.querySelectorAll('.menu').length`)) === 0, 'Escape closes it')
})

check('editing a property changes the part, and undo restores it', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  // Selecting through the context menu avoids arming a wire on the LED's pins.
  const centre = await app.partCentre('wokwi-led')
  await app.ev(`(() => {
    const el = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'wokwi-led')
    const r = el.getBoundingClientRect()
    el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: r.x + r.width / 2, clientY: r.y + r.height / 2 }))
  })()`)
  await sleep(200)
  // Close the menu without Escape: Escape also clears the selection in this app.
  await app.ev(`document.body.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }))`)
  await sleep(200)
  // The colour is a property of the custom element, not an attribute it renders.
  const colour = () => app.ev(`document.querySelector('wokwi-led')?.color`)
  assert((await colour()) === 'red', 'the LED starts red')
  await app.ev(`(() => {
    const s = document.querySelector('.props select')
    s.value = 'blue'
    s.dispatchEvent(new Event('change', { bubbles: true }))
  })()`)
  await sleep(300)
  assert((await colour()) === 'blue', `the property changed the part (${await colour()})`)
  await app.ev(`document.querySelector('.panel__body--canvas').focus()`)
  await app.key('z', { code: 'KeyZ', vk: 90, modifiers: 2 })
  await sleep(300)
  assert((await colour()) === 'red', `undo restored it (${await colour()})`)
})

check('a board on a fresh canvas seats a part dropped on it', async (app) => {
  // The first-launch case: the palette's hidden elements are measured
  // asynchronously, and a part dropped before that pass must still land in holes
  // rather than on the free grid or a random single hole.
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.drag(await app.paletteTile('breadboard'), await app.canvasCentre())
  await sleep(400)

  const hole = await app.ev(`(() => {
    const board = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'sim-breadboard')
    if (!board) return null
    const host = board.firstElementChild
    const pins = host.pinInfo ?? []
    const target = pins.find(p => p.name === 'e5') ?? pins[0]
    if (!target) return null
    const r = host.getBoundingClientRect()
    return { x: Math.round(r.x + target.x), y: Math.round(r.y + target.y) }
  })()`)
  assert(!!hole, 'the breadboard is on the canvas with its holes measured')

  await app.drag(await app.paletteTile('led'), hole)
  await sleep(500)
  const misses = await app.ev(`(() => {
    const board = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'sim-breadboard')
    const led = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'wokwi-led')
    if (!board || !led) return null
    const host = board.firstElementChild
    const r = host.getBoundingClientRect()
    const holes = (host.pinInfo ?? []).map(p => ({ x: r.x + p.x, y: r.y + p.y }))
    return [...led.querySelectorAll('[data-pin]')].map(pin => {
      const pr = pin.getBoundingClientRect()
      const x = pr.x + pr.width / 2
      const y = pr.y + pr.height / 2
      return Math.round(Math.min(...holes.map(h => Math.hypot(h.x - x, h.y - y))) * 10) / 10
    })
  })()`)
  assert(Array.isArray(misses) && misses.length === 2, `the LED has two legs (${JSON.stringify(misses)})`)
  assert(
    Array.isArray(misses) && misses.every((d) => d <= 2.2),
    `both legs are in holes (misses ${JSON.stringify(misses)})`,
  )

  // The world origin is at the middle of the view, so a board dropped there sits
  // at negative coordinates. Snapping used to clamp the pointer to >= 0, which
  // made the origin a wall and pulled parts onto the board from well outside it.
  const board = await app.ev(`(() => {
    const b = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'sim-breadboard')
    const r = b.getBoundingClientRect()
    return { x: r.x, y: r.y, w: r.width }
  })()`)
  await app.drag(await app.paletteTile('led'), { x: board.x - 40, y: board.y + 100 })
  await sleep(500)
  const outsideMiss = await app.ev(`(() => {
    const board = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'sim-breadboard')
    const leds = [...document.querySelectorAll('.instance')].filter(e => e.firstElementChild.tagName.toLowerCase() === 'wokwi-led')
    const led = leds[leds.length - 1]
    if (!board || !led) return null
    const host = board.firstElementChild
    const r = host.getBoundingClientRect()
    const holes = (host.pinInfo ?? []).map(p => ({ x: r.x + p.x, y: r.y + p.y }))
    return [...led.querySelectorAll('[data-pin]')].map(pin => {
      const pr = pin.getBoundingClientRect()
      const x = pr.x + pr.width / 2
      const y = pr.y + pr.height / 2
      return Math.round(Math.min(...holes.map(h => Math.hypot(h.x - x, h.y - y))) * 10) / 10
    })
  })()`)
  assert(
    Array.isArray(outsideMiss) && outsideMiss.every((d) => d > 2.2),
    `a part dropped left of the board is not pulled onto it (misses ${JSON.stringify(outsideMiss)})`,
  )
})

check('the canvas bitmap follows a layout resize', async (app) => {
  // The visible circuit is a bitmap. If it is not redrawn when the canvas changes
  // size, CSS stretches the old bitmap and the circuit warps until something else
  // happens to trigger a draw.
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Blink')
  const size = () =>
    app.ev(`(() => {
      const c = document.querySelector('canvas.circuit-canvas')
      const r = c.parentElement.getBoundingClientRect()
      return { cw: c.width, ch: c.height, hw: Math.round(r.width), hh: Math.round(r.height), dpr: devicePixelRatio }
    })()`)
  const before = await size()
  assert(
    before.cw === Math.round(before.hw * before.dpr) && before.ch === Math.round(before.hh * before.dpr),
    `starts in step with its layout (${before.cw}x${before.ch} for ${before.hw}x${before.hh} @${before.dpr})`,
  )

  await app.send('Emulation.setDeviceMetricsOverride', {
    width: 1100,
    height: 720,
    deviceScaleFactor: 1,
    mobile: false,
  })
  await sleep(500)
  const resized = await size()
  assert(
    resized.cw === Math.round(resized.hw * resized.dpr) &&
      resized.ch === Math.round(resized.hh * resized.dpr),
    `redrew after a window resize (${resized.cw}x${resized.ch} for ${resized.hw}x${resized.hh} @${resized.dpr})`,
  )
  await app.send('Emulation.clearDeviceMetricsOverride')
  await sleep(400)

  // Opening the serial monitor shrinks the canvas; the same rule applies.
  const toggle = await app.ev(`(() => {
    const b = document.querySelector('button[title="Toggle the serial monitor"]')
    const r = b.getBoundingClientRect()
    return { x: Math.round(r.x + r.width / 2), y: Math.round(r.y + r.height / 2) }
  })()`)
  await app.click(toggle.x, toggle.y)
  await sleep(500)
  const withSerial = await size()
  assert(
    withSerial.cw === Math.round(withSerial.hw * withSerial.dpr) &&
      withSerial.ch === Math.round(withSerial.hh * withSerial.dpr),
    `redrew when the serial monitor opened (${withSerial.cw}x${withSerial.ch} for ${withSerial.hw}x${withSerial.hh} @${withSerial.dpr})`,
  )
  await app.click(toggle.x, toggle.y)
  await sleep(300)
})

check('the potentiometer knob turns a live value', async (app) => {
  await app.goto(`http://localhost:${DEV_PORT}`)
  await app.loadPreset('Potentiometer')
  const knob = await app.ev(`(() => {
    const inst = [...document.querySelectorAll('.instance')].find(e => e.firstElementChild.tagName.toLowerCase() === 'wokwi-potentiometer')
    if (!inst) return null
    const svg = inst.firstElementChild.shadowRoot?.querySelector('svg')
    const r = svg?.getBoundingClientRect()
    if (!r) return null
    return { x: r.x + (9.91 / 20) * r.width, y: r.y + (8.18 / 20) * r.height, w: r.width }
  })()`)
  assert(!!knob, 'the potentiometer and its knob are on the canvas')
  const read = () =>
    app.ev(`(() => ({
      value: Number(document.querySelector('wokwi-potentiometer')?.value),
      readout: document.querySelector('.props__readout')?.textContent,
    }))()`)

  await app.drag({ x: knob.x, y: knob.y }, { x: knob.x + 27, y: knob.y })
  await sleep(400)
  const turned = await read()
  assert(turned.value >= 840 && turned.value <= 865, `a drag to the right lands near 853 (${turned.value})`)
  assert(turned.readout === String(turned.value), `the readout follows the knob (${turned.readout})`)

  await app.drag({ x: knob.x, y: knob.y }, { x: knob.x, y: knob.y - 27 })
  await sleep(400)
  const up = await read()
  assert(up.value >= 495 && up.value <= 525, `a drag straight up lands near mid-track (${up.value})`)
})

async function main() {
  if (process.argv.includes('--help')) {
    console.log(`
  npm run drive                 run the smoke checks
  npm run drive -- --list       list them without running
  npm run drive -- --keep       leave the server and browser running afterwards
  npm run drive -- --no-start   fail rather than start a server

  The dev server runs on ${DEV_PORT} and chromium's debugging port is ${CDP_PORT}.
  Neither is ${5173}: that is the development port and must be left alone.
`)
    return
  }
  if (process.argv.includes('--list')) {
    for (const c of checks) console.log('  ' + c.name)
    return
  }

  const keep = process.argv.includes('--keep')
  const app = await connect({ startServer: !process.argv.includes('--no-start') })
  const cleanup = () => {
    if (!keep) app.stop()
  }
  process.on('exit', cleanup)
  process.on('SIGINT', () => {
    cleanup()
    process.exit(130)
  })

  let failures = 0
  try {
    for (const { name, fn } of checks) {
      asserts.length = 0
      app.consoleErrors.length = 0
      let error = null
      try {
        await fn(app)
      } catch (err) {
        error = err
      }
      const bad = asserts.filter((a) => !a.ok)
      const ok = !error && bad.length === 0
      console.log(`\n  ${ok ? '✓' : '✗'} ${name}`)
      for (const a of asserts) console.log(`      ${a.ok ? '✓' : '✗'} ${a.description}`)
      if (error) console.log(`      ✗ threw: ${error.message}`)
      // A silent console error in the app is a bug even when the assertions pass.
      const noise = app.consoleErrors.filter((e) => !/Lit is in dev mode/.test(e))
      if (noise.length) {
        console.log(`      ✗ console: ${noise.slice(0, 3).join(' | ')}`)
        failures++
      }
      if (!ok) failures++
    }
  } finally {
    if (!keep) app.stop()
  }

  console.log(failures ? `\n  ${failures} check(s) failed\n` : `\n  all ${checks.length} checks passed\n`)
  process.exit(failures ? 1 : 0)
}

// Only run when invoked directly, so the file can also be imported as a library.
if (process.argv[1] && import.meta.url === `file://${process.argv[1]}`) {
  main().catch((err) => {
    console.error('\n  drive failed:', err.message, '\n')
    process.exit(1)
  })
}
