/**
 * Rasterising a `wokwi-*` element's own drawing.
 *
 * The circuit is painted onto a `<canvas>` and the palette shows thumbnails, and
 * both need the same thing: the element's shadow-root SVG, serialised as valid
 * XML (HTML `outerHTML` emits entities an SVG parser rejects, e.g. `&nbsp;`),
 * carrying its styles across — which Lit may put in `adoptedStyleSheets` rather
 * than in a `<style>` element.
 *
 * It lives here so the canvas and the palette can never drift apart.
 */

/**
 * Serialise the element's shadow-root drawing as a standalone SVG document.
 *
 * Returns `null` when the element isn't laid out yet (Lit renders
 * asynchronously) — the caller retries. Throws when the serialised SVG won't
 * parse, with the parser's own complaint.
 */
export function shadowSvgMarkup(host: HTMLElement): string | null {
  const root = host.shadowRoot
  const width = host.offsetWidth
  const height = host.offsetHeight
  if (!root || !width || !height) return null

  const style = [
    ...Array.from(root.querySelectorAll('style')).map((c) => c.textContent ?? ''),
    ...Array.from(root.adoptedStyleSheets ?? []).map((sheet) => {
      try {
        return Array.from(sheet.cssRules)
          .map((r) => r.cssText)
          .join('\n')
      } catch {
        return ''
      }
    }),
  ].join('\n')

  // Prefer the element's own <svg>; some wrap it in HTML (the LED's
  // `.led-container`), which can't be serialised into an <svg>. Some elements
  // carry a small hidden decoration SVG before the real body (the buzzer's music
  // note), so take the largest one rather than the first.
  const svgs = Array.from(root.querySelectorAll('svg'))
  const svg = svgs.sort((a, b) => {
    const ar = a.getBoundingClientRect()
    const br = b.getBoundingClientRect()
    return br.width * br.height - ar.width * ar.height
  })[0]
  // Element connected but Lit hasn't drawn into it yet — snapshotting now would
  // cache a blank drawing for good, so say "not ready" and let the caller retry.
  if (!svg && !root.children.length) return null
  let markup: string
  if (svg) {
    const clone = svg.cloneNode(true) as SVGSVGElement
    clone.setAttribute('xmlns', 'http://www.w3.org/2000/svg')
    clone.setAttribute('xmlns:xlink', 'http://www.w3.org/1999/xlink')
    if (style) {
      const st = document.createElementNS('http://www.w3.org/2000/svg', 'style')
      st.textContent = style
      clone.insertBefore(st, clone.firstChild)
    }
    markup = new XMLSerializer().serializeToString(clone)
  } else {
    markup =
      `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">` +
      (style ? `<style>${style}</style>` : '') +
      Array.from(root.children)
        .filter((c) => c.tagName.toLowerCase() !== 'style')
        .map((c) => c.outerHTML)
        .join('') +
      `</svg>`
  }

  const parsed = new DOMParser().parseFromString(markup, 'image/svg+xml')
  const parseError = parsed.querySelector('parsererror')
  if (parseError) {
    throw new Error((parseError.textContent ?? 'invalid SVG').slice(0, 200))
  }
  return markup
}

export interface PartArt {
  image: HTMLImageElement
  /** A live object URL for the drawing — the caller decides when to revoke it. */
  href: string
  width: number
  height: number
}

/** Load serialised SVG as an image, sized by the drawing rather than the box. */
export function svgMarkupToImage(
  markup: string,
  fallback?: { width: number; height: number },
): Promise<PartArt> {
  const href = URL.createObjectURL(new Blob([markup], { type: 'image/svg+xml' }))
  return new Promise((resolve, reject) => {
    const image = new Image()
    image.onload = () => {
      resolve({
        image,
        href,
        width: image.naturalWidth || fallback?.width || 0,
        height: image.naturalHeight || fallback?.height || 0,
      })
    }
    image.onerror = () => {
      URL.revokeObjectURL(href)
      reject(new Error('the SVG failed to load as an image'))
    }
    image.src = href
  })
}
