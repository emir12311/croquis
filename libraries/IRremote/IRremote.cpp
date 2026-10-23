#include "IRremote.h"

IRrecv *IRrecv::instance = nullptr;

IRrecv::IRrecv(int pin) : pin(pin), lastEdge(0), count(0) {}

void IRrecv::enableIRIn() {
  pinMode(pin, INPUT_PULLUP);
  instance = this;
  count = 0;
  lastEdge = micros();
  attachInterrupt(digitalPinToInterrupt(pin), IRrecv::isr, CHANGE);
}

void IRrecv::isr() {
  IRrecv *self = instance;
  if (!self) return;
  unsigned long now = micros();
  unsigned long duration = now - self->lastEdge;
  self->lastEdge = now;
  uint8_t level = digitalRead(self->pin);
  self->record(duration, level ? LOW : HIGH);
}

void IRrecv::record(uint32_t duration, uint8_t level) {
  uint8_t i = count;
  if (i >= CAPACITY) {
    count = 0;
    i = 0;
  }
  durations[i] = duration;
  levels[i] = level;
  count = i + 1;
}

void IRrecv::resume() {}

bool IRrecv::decode(decode_results *results) {
  noInterrupts();
  uint8_t n = count;
  uint32_t d[CAPACITY];
  uint8_t l[CAPACITY];
  for (uint8_t i = 0; i < n; i++) {
    d[i] = durations[i];
    l[i] = levels[i];
  }
  interrupts();

  int start = -1;
  for (int i = 0; i + 1 < n; i++) {
    if (l[i] == LOW && d[i] >= 8000 && d[i] <= 10000 && l[i + 1] == HIGH &&
        d[i + 1] >= 3500 && d[i + 1] <= 5500) {
      start = i;
      break;
    }
  }
  if (start < 0) return false;

  uint8_t bytes[4] = {0, 0, 0, 0};
  int pos = start + 2;
  bool ok = pos + 64 <= n;
  for (int bit = 0; ok && bit < 32; bit++) {
    if (l[pos] != LOW || d[pos] < 300 || d[pos] > 900) {
      ok = false;
      break;
    }
    if (l[pos + 1] != HIGH || d[pos + 1] < 300 || d[pos + 1] > 2000) {
      ok = false;
      break;
    }
    if (d[pos + 1] > 1000) bytes[bit / 8] |= 1 << (bit % 8);
    pos += 2;
  }
  if (!ok || pos >= n || l[pos] != LOW || d[pos] < 300 || d[pos] > 900) return false;
  if ((uint8_t)~bytes[0] != bytes[1] || (uint8_t)~bytes[2] != bytes[3]) return false;

  noInterrupts();
  count = 0;
  interrupts();

  results->address = bytes[0];
  results->command = bytes[2];
  results->bits = 32;
  results->decode_type = NEC;
  results->value = ((uint32_t)bytes[0] << 24) | ((uint32_t)bytes[1] << 16) |
                   ((uint32_t)bytes[2] << 8) | bytes[3];
  return true;
}
