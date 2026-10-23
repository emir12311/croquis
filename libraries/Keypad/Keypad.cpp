#include "Keypad.h"

Keypad::Keypad(char *userKeymap, byte *row, byte *col, byte numRows, byte numCols)
    : keymap(userKeymap),
      rowPins(row),
      columnPins(col),
      rows(numRows),
      columns(numCols),
      debounceTime(50),
      lastChange(0),
      lastKey(NO_KEY) {}

char Keypad::getKey() {
  char found = NO_KEY;
  for (byte r = 0; r < rows && found == NO_KEY; r++) {
    for (byte i = 0; i < rows; i++) {
      pinMode(rowPins[i], OUTPUT);
      digitalWrite(rowPins[i], i == r ? LOW : HIGH);
    }
    // The simulator applies the column level through the UI. Hold the row long
    // enough for that round trip; a real keypad needs no such pause.
    delay(10);
    for (byte c = 0; c < columns; c++) {
      pinMode(columnPins[c], INPUT_PULLUP);
      if (digitalRead(columnPins[c]) == LOW) {
        found = keymap[r * columns + c];
        break;
      }
    }
  }
  for (byte i = 0; i < rows; i++) pinMode(rowPins[i], INPUT_PULLUP);

  unsigned long now = millis();
  if (found != lastKey && now - lastChange >= debounceTime) {
    lastChange = now;
    lastKey = found;
    if (found != NO_KEY) return found;
  }
  return NO_KEY;
}
