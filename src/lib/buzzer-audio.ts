import type { OutputState } from './netlist'

/**
 * The buzzer's sound. The netlist decides whether a buzzer is across a live
 * supply and, when the pin is PWM, what its period says; this turns that into
 * one square-wave voice per buzzer. It is deliberately quiet and square, like a
 * piezo, and it never creates an AudioContext until a buzzer is actually driven.
 */
const MAX_GAIN = 0.035
const MIN_HZ = 80
const MAX_HZ = 4000

interface Voice {
  osc: OscillatorNode
  gain: GainNode
}

export class BuzzerAudio {
  private ctx: AudioContext | null = null
  private voices = new Map<string, Voice>()

  /** Resume (or create) the audio context from a user gesture. */
  unlock() {
    const ctx = this.ensureContext()
    if (ctx?.state === 'suspended') void ctx.resume().catch(() => {})
  }

  update(outputs: Record<string, OutputState>) {
    const active = new Map<string, number>()
    for (const [id, state] of Object.entries(outputs)) {
      if (!state.active || typeof state.frequency !== 'number') continue
      const hz = state.frequency > 0 ? state.frequency : 1000
      active.set(id, Math.min(MAX_HZ, Math.max(MIN_HZ, hz)))
    }
    if (!active.size) {
      this.stopAll()
      return
    }
    const ctx = this.ensureContext()
    if (!ctx) return
    // A first Run click is a user gesture, so this normally starts immediately.
    // If the host suspended it anyway, the next output update tries again.
    if (ctx.state === 'suspended') void ctx.resume().catch(() => {})
    for (const id of [...this.voices.keys()]) {
      if (!active.has(id)) this.stop(id)
    }
    for (const [id, hz] of active) {
      const voice = this.voices.get(id)
      if (voice) {
        voice.osc.frequency.setTargetAtTime(hz, ctx.currentTime, 0.01)
        continue
      }
      const osc = ctx.createOscillator()
      const gain = ctx.createGain()
      osc.type = 'square'
      osc.frequency.value = hz
      gain.gain.setValueAtTime(0, ctx.currentTime)
      gain.gain.linearRampToValueAtTime(MAX_GAIN, ctx.currentTime + 0.012)
      osc.connect(gain)
      gain.connect(ctx.destination)
      osc.start()
      this.voices.set(id, { osc, gain })
    }
  }

  stopAll() {
    for (const id of [...this.voices.keys()]) this.stop(id)
  }

  dispose() {
    this.stopAll()
    void this.ctx?.close().catch(() => {})
    this.ctx = null
  }

  private stop(id: string) {
    const ctx = this.ctx
    const voice = this.voices.get(id)
    if (!ctx || !voice) return
    this.voices.delete(id)
    const now = ctx.currentTime
    voice.gain.gain.cancelScheduledValues(now)
    voice.gain.gain.setTargetAtTime(0, now, 0.008)
    try {
      voice.osc.stop(now + 0.04)
    } catch {
      // Already stopped — nothing to do.
    }
    voice.osc.onended = () => {
      voice.osc.disconnect()
      voice.gain.disconnect()
    }
  }

  private ensureContext(): AudioContext | null {
    if (this.ctx) return this.ctx
    if (typeof window === 'undefined') return null
    const Ctor =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
    if (!Ctor) return null
    try {
      this.ctx = new Ctor()
    } catch {
      return null
    }
    return this.ctx
  }
}
