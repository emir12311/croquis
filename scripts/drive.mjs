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
        this.consoleErrors.push('EXCEPTION ' + (msg.params.exceptionDetails?.text ?? ''))
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
    await sleep(2500) // the app mounts its parts and fonts
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

  /** Load one of the shipped circuits by name, the way a person would. */
  async loadPreset(label) {
    const file = await this.ev(`(() => { const b = [...document.querySelectorAll('.menubar__btn')].find(e => e.textContent.trim() === 'File'); const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } })()`)
    await this.click(file.x, file.y)
    await sleep(350)
    const item = await this.ev(`(() => { const b = [...document.querySelectorAll('.menu__item')].find(e => e.textContent.trim() === ${JSON.stringify(label)}); if (!b) return null; const r = b.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 } })()`)
    if (!item) throw new Error(`no preset called ${label}`)
    await this.click(item.x, item.y)
    await sleep(900)
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

check('Ctrl+A selects everything', async (app) => {
  await app.ev(`document.querySelector('.panel__body--canvas').focus()`)
  await app.key('a', { code: 'KeyA', vk: 65, modifiers: 2 })
  await sleep(250)
  assert((await app.ev(`document.querySelectorAll('.instance--selected').length`)) === 3, 'all three parts selected')
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
