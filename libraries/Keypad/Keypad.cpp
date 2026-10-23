#include "Keypad.h"

/**
 * How long a row is held before its columns are read, in milliseconds.
 *
 * The sketch pulls a row low and the column level comes back through the
 * simulator's UI: measured at 6–9 ms end to end on this machine, so this is a
 * little over twice the observed round trip. It has to be longer than that trip
 * — the cost of being short is a *wrong* key rather than a missed one — but it
 * also sets how long a scan takes, and a scan has to finish inside one tap.
 * Four rows per pass, so this is a quarter of the tap's budget.
 */
static const unsigned long SETTLE_DELAY = 20;

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
    for (byte c = 0; c < columns; c++) pinMode(columnPins[c], INPUT_PULLUP);
    // The simulator applies the column level through the UI: the row change goes
    // out to the canvas, the netlist answers, and the level comes back — real
    // milliseconds later. Hold the row longer than that trip, then read once.
    //
    // Both halves matter. Read too early and the previous row's answer is still
    // on the line, so a key in one row is reported again as the first key of the
    // next: press '1' and it also reports '4'. Polling instead of waiting makes
    // that worse, not better — it grabs the stale level the moment it looks.
    delay(SETTLE_DELAY);
    for (byte c = 0; c < columns; c++) {
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
