/**
 * The circuits we ship, so a first run has something to press Run on.
 *
 * Each one is a real `diagram.json` — the same shape File → Save writes, sketch
 * included — so a preset is loaded by exactly the code that opens a file, and
 * anything that works here works as a diagram you can share. The files live in
 * `presets/` at the repository root: adding one is a new file and nothing else.
 *
 * They are pulled in as text at build time, so there is no path to find at run time.
 */
const files = import.meta.glob('/presets/*.json', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>

export interface Preset {
  id: string
  label: string
  /** `diagram.json` text, ready for the loader that opens a file. */
  text: string
}

/** "button-serial" → "Button serial", for a file carrying no title of its own. */
const prettify = (id: string) =>
  id.replace(/[-_]+/g, ' ').replace(/^./, (c) => c.toUpperCase())

function labelOf(id: string, text: string): string {
  try {
    const title = (JSON.parse(text) as { title?: unknown }).title
    if (typeof title === 'string' && title.trim()) return title.trim()
  } catch {
    // A file that will not parse keeps its filename; a test says so out loud.
  }
  return prettify(id)
}

/** Sorted by id, which happens to be a sensible teaching order. */
export const PRESETS: Preset[] = Object.entries(files)
  .map(([path, text]) => {
    const id = path.split('/').pop()!.replace(/\.json$/, '')
    return { id, label: labelOf(id, text), text }
  })
  .sort((a, b) => a.id.localeCompare(b.id))
