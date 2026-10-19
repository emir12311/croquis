/* Tiny accessors for fields bindgen treated as opaque (avr_t has bitfields),
 * so Rust can set the clock and pace the simulation to real time. */
#include "sim_avr.h"

unsigned long long sim_shim_cycle(avr_t *avr) { return avr->cycle; }
unsigned int sim_shim_freq(avr_t *avr) { return avr->frequency; }
void sim_shim_set_freq(avr_t *avr, unsigned int freq) { avr->frequency = freq; }

/* The reference the ADC measures against. An Uno ties AVCC and AREF to 5V and
 * runs its I/O at 5V, so all three follow the supply; without this simavr has no
 * reference and quietly falls back to 3.3V, making every analogRead 5V reads. */
void sim_shim_set_supply(avr_t *avr, unsigned int mv) {
    avr->vcc = mv;
    avr->avcc = mv;
    avr->aref = mv;
}
