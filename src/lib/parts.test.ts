import { describe, expect, it } from 'vitest'
import { RESISTANCE_UNITS, formatResistance, kindForTag, partByKind, parseResistance } from './parts'

/** The resistor's own control, so the tests use the real bounds. */
function resistorField() {
  const field = (partByKind('resistor').fields ?? [])[0]
  if (field.control.kind !== 'resistance') throw new Error('the resistor should have a resistance field')
  return field.control
}
const { min, max } = resistorField()
const parse = (text: string, factor: number) => parseResistance(text, factor, min, max)

describe('the display catalogue', () => {
  it('keeps the 1- and 4-digit displays as separate kinds', () => {
    expect(kindForTag('wokwi-7segment')).toBe('seven-segment')
    expect(kindForTag('wokwi-7segment-4')).toBe('seven-segment-4')
    expect(partByKind('seven-segment-4').attrs.digits).toBe('4')
    expect(partByKind('seven-segment').fields).toBeUndefined()
  })
})

describe('showing a resistance', () => {
  it('picks the unit that keeps the number readable', () => {
    expect(formatResistance(100)).toEqual({ text: '100', factor: 1 })
    expect(formatResistance(220)).toEqual({ text: '220', factor: 1 })
    expect(formatResistance(1000)).toEqual({ text: '1', factor: 1000 })
    expect(formatResistance(4700)).toEqual({ text: '4.7', factor: 1000 })
    expect(formatResistance(100_000)).toEqual({ text: '100', factor: 1000 })
    expect(formatResistance(4_700_000)).toEqual({ text: '4.7', factor: 1_000_000 })
  })
})

describe('reading a typed resistance', () => {
  it('turns the number and the unit into ohms', () => {
    expect(parse('4.7', 1000)).toBe(4700)
    expect(parse('100', 1)).toBe(100)
    expect(parse('4.7', 1_000_000)).toBe(4_700_000)
  })

  it('takes a comma as a decimal point, the way much of the world types', () => {
    expect(parse('4,7', 1000)).toBe(4700)
  })

  it('clamps to what the part supports, so the box can show what is stored', () => {
    expect(parse('0.0000000000000001', 1)).toBe(min) // below a pΩ
    expect(parse('9999', 1e12)).toBe(max) // above a TΩ
  })

  it('returns null for anything that is not a number, to put back what is stored', () => {
    for (const bad of ['', '   ', 'abc', '0', '-470', '4.7.1']) expect(parse(bad, 1), bad).toBeNull()
  })

  it('round-trips every resistance the old list offered, in every unit', () => {
    // Type what is shown and you get the same resistance back. These are the values
    // people actually use, and the ones the picker used to hard-code.
    const real = [1, 10, 100, 220, 330, 470, 680, 1000, 2200, 4700, 10_000, 100_000, 1_000_000, 4_700_000]
    for (const ohms of real) {
      const shown = formatResistance(ohms)
      expect(parse(shown.text, shown.factor), `${ohms} Ω shown as ${shown.text}`).toBe(ohms)
      for (const unit of RESISTANCE_UNITS) {
        // Reading it in any other unit must land on the same resistance too.
        const asOther = String(ohms / unit.factor)
        expect(parse(asOther, unit.factor), `${ohms} Ω typed as ${asOther} ${unit.label}`).toBe(ohms)
      }
    }
  })
})

describe('the resistor field', () => {
  it('offers the whole SI ladder, smallest first', () => {
    expect(RESISTANCE_UNITS.map((u) => u.label)).toEqual([
      'pΩ', 'nΩ', 'µΩ', 'mΩ', 'Ω', 'kΩ', 'MΩ', 'GΩ', 'TΩ',
    ])
    const factors = RESISTANCE_UNITS.map((u) => u.factor)
    expect(factors).toEqual([...factors].sort((a, b) => a - b))
  })

  it('can reach every unit it offers, so none of them is a dead choice', () => {
    const factors = RESISTANCE_UNITS.map((u) => u.factor)
    expect(min).toBeLessThanOrEqual(Math.min(...factors))
    expect(max).toBeGreaterThanOrEqual(Math.max(...factors))
  })

  it('keeps values below an ohm, which rounding to whole ohms would erase', () => {
    expect(parse('470', 1e-3)).toBeCloseTo(0.47, 12) // 470 mΩ
    expect(parse('4.7', 1e-6)).toBeCloseTo(4.7e-6, 18) // 4.7 µΩ
    expect(parse('1', 1e-9)).toBe(1e-9) // 1 nΩ
    expect(parse('1', 1e-12)).toBe(1e-12) // 1 pΩ
    expect(parse('1', 1e12)).toBe(1e12) // 1 TΩ
  })
})
