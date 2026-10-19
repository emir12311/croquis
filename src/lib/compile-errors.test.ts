import { describe, expect, it } from 'vitest'
import { isSketchFile, parseCompileErrors } from './compile-errors'

describe('parseCompileErrors', () => {
  it('reads file, line, column, severity and message in order', () => {
    const output = [
      "sketch.ino: In function 'void setup()':",
      "sketch.ino:12:5: error: 'foo' was not declared in this scope",
      'sketch.ino:12:5: note: suggested alternative: food',
      "sketch.ino:20:3: warning: unused variable 'x'",
    ].join('\n')

    expect(parseCompileErrors(output)).toEqual([
      {
        file: 'sketch.ino',
        line: 12,
        column: 5,
        severity: 'error',
        message: "'foo' was not declared in this scope",
      },
      {
        file: 'sketch.ino',
        line: 12,
        column: 5,
        severity: 'note',
        message: 'suggested alternative: food',
      },
      {
        file: 'sketch.ino',
        line: 20,
        column: 3,
        severity: 'warning',
        message: "unused variable 'x'",
      },
    ])
  })

  it('accepts a diagnostic without a column', () => {
    expect(parseCompileErrors('sketch.ino:3: error: expected declaration')).toEqual([
      {
        file: 'sketch.ino',
        line: 3,
        column: 0,
        severity: 'error',
        message: 'expected declaration',
      },
    ])
  })

  it('keeps header errors but ignores linker lines without a position', () => {
    const output = [
      '/usr/share/arduino/cores/arduino/Arduino.h:41:10: fatal error: avr/io.h: No such file or directory',
      'collect2: error: ld returned 1 exit status',
      "undefined reference to `setup'",
    ].join('\n')

    expect(parseCompileErrors(output)).toEqual([
      {
        file: '/usr/share/arduino/cores/arduino/Arduino.h',
        line: 41,
        column: 10,
        severity: 'error',
        message: 'avr/io.h: No such file or directory',
      },
    ])
  })

  it('returns nothing for output with no diagnostics', () => {
    expect(parseCompileErrors('')).toEqual([])
    expect(parseCompileErrors('avr-g++: fatal error: no input files')).toEqual([])
  })
})

describe('isSketchFile', () => {
  it('accepts the sketch and absolute paths to it, and nothing else', () => {
    expect(isSketchFile('sketch.ino')).toBe(true)
    expect(isSketchFile('/tmp/build/sketch.ino')).toBe(true)
    expect(isSketchFile('some-other-sketch.ino')).toBe(false)
    expect(isSketchFile('sketch.cpp')).toBe(false)
  })
})
