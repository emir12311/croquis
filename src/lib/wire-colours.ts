/**
 * Wokwi's named wire colours. Shared by the properties strip (which offers them)
 * and the canvas renderer (which draws them).
 */

export const WIRE_COLORS: { name: string; hex: string }[] = [
  { name: 'green', hex: '#3fa34d' },
  { name: 'red', hex: '#d64545' },
  { name: 'blue', hex: '#3b7dd8' },
  { name: 'yellow', hex: '#d9b23a' },
  { name: 'orange', hex: '#dd8a3c' },
  { name: 'purple', hex: '#9b6dd6' },
  { name: 'teal', hex: '#2f9e8f' },
  { name: 'pink', hex: '#d96a9c' },
]

export const WIRE_HEX = new Map(WIRE_COLORS.map((c) => [c.name, c.hex]))

/** A colour name as CSS, falling back to the themed wire colour. */
export function wireHex(name?: string): string {
  return name ? (WIRE_HEX.get(name) ?? name) : 'var(--wire)'
}
