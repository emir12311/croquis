import { describe, expect, it } from 'vitest'
import { isAtBottom } from './serial-scroll'

describe('serial auto-scroll', () => {
  it('follows while the box is at the bottom', () => {
    expect(isAtBottom({ scrollTop: 100, clientHeight: 50, scrollHeight: 150 })).toBe(true)
  })

  it('stops following once the user scrolls up', () => {
    expect(isAtBottom({ scrollTop: 80, clientHeight: 50, scrollHeight: 150 })).toBe(false)
  })

  it('treats a box shorter than its viewport as at the bottom', () => {
    expect(isAtBottom({ scrollTop: 0, clientHeight: 150, scrollHeight: 80 })).toBe(true)
  })

  it('allows a few pixels of rounding slack', () => {
    expect(isAtBottom({ scrollTop: 93, clientHeight: 50, scrollHeight: 150 })).toBe(true)
    expect(isAtBottom({ scrollTop: 90, clientHeight: 50, scrollHeight: 150 })).toBe(false)
  })
})
