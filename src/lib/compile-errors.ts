/**
 * Compiler diagnostics, parsed out of the text `avr-g++` writes to stderr.
 *
 * The compiler is invoked with `#line 1 "sketch.ino"`, so a sketch diagnostic
 * looks like `sketch.ino:12:5: error: ...`. Errors from headers or the linker
 * have no useful editor position; they are still parsed so the panel can show
 * them, but only sketch lines are jumpable.
 */

export type Severity = 'error' | 'warning' | 'note'

export interface CompileError {
  file: string
  line: number
  column: number
  severity: Severity
  message: string
}

const DIAGNOSTIC = /^(.*?):(\d+)(?::(\d+))?:\s*(fatal error|error|warning|note):\s*(.*)$/

export function parseCompileErrors(output: string): CompileError[] {
  const found: CompileError[] = []
  for (const raw of output.split(/\r?\n/)) {
    const match = raw.match(DIAGNOSTIC)
    if (!match) continue
    const file = match[1].trim()
    if (!file) continue
    const kind = match[4]
    found.push({
      file,
      line: Number(match[2]),
      column: match[3] ? Number(match[3]) : 0,
      severity: kind === 'warning' ? 'warning' : kind === 'note' ? 'note' : 'error',
      message: match[5].trim(),
    })
  }
  return found
}

/** Whether a diagnostic can be jumped to in the sketch editor. */
export function isSketchFile(file: string): boolean {
  return /(^|\/)sketch\.ino$/.test(file)
}
