#!/usr/bin/env node
/**
 * Extract the Wokwi element drawings and pin metadata for the native renderer.
 *
 * The elements are Lit web components: their SVG only exists after a real browser
 * upgrades them. This uses the same Vite + Chromium harness as `npm run drive`
 * rather than trying to regex the JavaScript templates.
 *
 * Wokwi Elements are MIT-licensed (see NOTICE). The two exceptions are the
 * breadboards and the 74HC595, which Croquis draws itself.
 *
 *   npm run extract:wokwi
 *
 * Output:
 *   assets/wokwi/<kind>.svg
 *   assets/wokwi/manifest.json
 *   assets/wokwi/LICENSE          (MIT notice for the extracted artwork)
 */
import { copyFile, mkdir, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { connect } from '../scripts/drive.mjs'

const OUT = join(process.cwd(), 'assets/wokwi')
const SKIP = new Set(['breadboard', 'breadboard-full', '74hc595'])

const app = await connect()
try {
  await app.goto('http://localhost:5199')
  const extracted = await app.ev(`(async () => {
    const { PARTS } = await import('/src/lib/parts.ts')
    const out = []
    for (const part of PARTS) {
      if (${JSON.stringify([...SKIP])}.includes(part.kind)) continue
      if (!customElements.get(part.tag)) {
        await customElements.whenDefined(part.tag)
      }
      const el = document.createElement(part.tag)
      el.style.position = 'absolute'
      el.style.left = '-10000px'
      el.style.top = '0'
      for (const [key, value] of Object.entries(part.attrs ?? {})) {
        el.setAttribute(key, value)
      }
      document.body.append(el)
      if (el.updateComplete) await el.updateComplete
      await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)))
      const svgs = [...(el.shadowRoot?.querySelectorAll('svg') ?? [])]
      const svg = svgs.sort((a, b) => {
        const area = (node) => {
          const box = node.getBoundingClientRect()
          return box.width * box.height
        }
        return area(b) - area(a)
      })[0]
      const pins = Array.isArray(el.pinInfo) ? el.pinInfo.map((pin) => ({
        name: pin.name,
        x: pin.x,
        y: pin.y,
        description: pin.description,
        signals: (pin.signals ?? []).map((signal) => signal.type ?? signal),
      })) : []
      out.push({
        kind: part.kind,
        tag: part.tag,
        attrs: part.attrs ?? {},
        width: el.offsetWidth || el.getBoundingClientRect().width,
        height: el.offsetHeight || el.getBoundingClientRect().height,
        svg: svg?.outerHTML ?? null,
        pins,
      })
      el.remove()
    }
    return JSON.stringify(out)
  })()`)

  const parts = JSON.parse(extracted)
  await mkdir(OUT, { recursive: true })
  const manifest = []
  for (const part of parts) {
    if (!part.svg) {
      console.warn(`  no SVG for ${part.kind} (${part.tag})`)
      continue
    }
    await writeFile(join(OUT, `${part.kind}.svg`), `${part.svg.replaceAll('&nbsp;', '&#160;')}\n`)
    manifest.push({
      kind: part.kind,
      tag: part.tag,
      attrs: part.attrs,
      width: part.width,
      height: part.height,
      svg: `${part.kind}.svg`,
      pins: part.pins,
    })
  }
  await writeFile(
    join(OUT, 'manifest.json'),
    `${JSON.stringify({ license: 'MIT', source: '@wokwi/elements 1.9.2', parts: manifest }, null, 2)}\n`,
  )
  await copyFile(
    join(process.cwd(), 'node_modules/@wokwi/elements/LICENSE'),
    join(OUT, 'LICENSE'),
  )
  console.log(`extracted ${manifest.length} Wokwi parts to ${OUT}`)
} finally {
  app.stop()
}
