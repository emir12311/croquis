#ifndef CROQUIS_IRREMOTE_H
#define CROQUIS_IRREMOTE_H

#include <Arduino.h>

#define NEC 1

struct decode_results {
  uint32_t value;
  uint16_t address;
  uint16_t command;
  uint8_t bits;
  int decode_type;
};

class IRrecv {
 public:
  IRrecv(int pin);

  void enableIRIn();
  bool decode(decode_results *results);
  void resume();

 private:
  static IRrecv *instance;
  static void isr();
  void record(uint32_t duration, uint8_t level);

  int pin;
  volatile unsigned long lastEdge;
  static const uint8_t CAPACITY = 100;
  volatile uint8_t count;
  volatile uint32_t durations[CAPACITY];
  volatile uint8_t levels[CAPACITY];
};

#endif
