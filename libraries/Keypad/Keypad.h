#ifndef CROQUIS_KEYPAD_H
#define CROQUIS_KEYPAD_H

#include <Arduino.h>

#define NO_KEY '\0'
#define makeKeymap(x) ((char *)(x))

class Keypad {
 public:
  Keypad(char *userKeymap, byte *row, byte *col, byte numRows, byte numCols);

  char getKey();
  void setDebounceTime(unsigned int ms) { debounceTime = ms; }
  void setHoldTime(unsigned int) {}

 private:
  char *keymap;
  byte *rowPins;
  byte *columnPins;
  byte rows;
  byte columns;
  unsigned int debounceTime;
  unsigned long lastChange;
  char lastKey;
};

#endif
