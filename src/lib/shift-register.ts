/**
 * The 74HC595's shift register, as a pure state machine.
 *
 * The chip is not a load like an LED: it remembers the bits it was clocked,
 * so its state has to survive between netlist solves. Keeping the state
 * transition here means the netlist can apply it and the simulation can step it
 * per pin edge without either owning the rules.
 */

/** The chip's control inputs, named as its element pins. */
export type ShiftSignal = 'ds' | 'shcp' | 'stcp' | 'mr' | 'oe'

export interface ShiftRegisterWiring {
  /** True when VCC and GND are the right way round. */
  powered: boolean
  /** The board pin key (`D3`) driving each input, when one reaches it. */
  pins: Partial<Record<ShiftSignal, string>>
  /** The level each input settles at from the rails, when no board pin drives it. */
  levels: Record<ShiftSignal, boolean>
}

export interface ShiftRegisterState {
  /** The bits currently clocked in, LSB = Q0. */
  shift: number
  /** The bits latched onto Q0…Q7, LSB = Q0. */
  latch: number
  /** The input levels at the last step, for edge detection. */
  levels: Record<ShiftSignal, boolean>
}

export function emptyShiftState(): ShiftRegisterState {
  return {
    shift: 0,
    latch: 0,
    levels: { ds: false, shcp: false, stcp: false, mr: false, oe: false },
  }
}

function levelOf(
  signal: ShiftSignal,
  wiring: ShiftRegisterWiring,
  avr: Record<string, boolean>,
): boolean {
  const key = wiring.pins[signal]
  return key ? avr[key] === true : wiring.levels[signal]
}

/**
 * Advance one solve's worth of time. The chip shifts on a rising SHCP, latches
 * on a rising STCP, and clears the shift register while MR is low. The storage
 * register is deliberately untouched by MR — that is what the real part does.
 */
export function stepShiftRegister(
  state: ShiftRegisterState,
  wiring: ShiftRegisterWiring,
  avr: Record<string, boolean>,
): ShiftRegisterState {
  const levels: Record<ShiftSignal, boolean> = {
    ds: levelOf('ds', wiring, avr),
    shcp: levelOf('shcp', wiring, avr),
    stcp: levelOf('stcp', wiring, avr),
    mr: levelOf('mr', wiring, avr),
    oe: levelOf('oe', wiring, avr),
  }
  if (!wiring.powered) return { shift: 0, latch: 0, levels }

  let shift = state.shift
  let latch = state.latch
  if (!levels.mr) {
    shift = 0
  } else if (levels.shcp && !state.levels.shcp) {
    shift = ((shift << 1) | (levels.ds ? 1 : 0)) & 0xff
  }
  if (levels.stcp && !state.levels.stcp) latch = shift
  return { shift, latch, levels }
}
