/* Tiny accessors for fields bindgen treated as opaque (avr_t has bitfields),
 * so Rust can set the clock and pace the simulation to real time. */
#include "sim_avr.h"

unsigned long long sim_shim_cycle(avr_t *avr) { return avr->cycle; }
unsigned int sim_shim_freq(avr_t *avr) { return avr->frequency; }
void sim_shim_set_freq(avr_t *avr, unsigned int freq) { avr->frequency = freq; }
