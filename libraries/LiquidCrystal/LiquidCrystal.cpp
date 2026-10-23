#include "LiquidCrystal.h"

LiquidCrystal::LiquidCrystal(uint8_t rs, uint8_t enable, uint8_t d4, uint8_t d5, uint8_t d6,
                             uint8_t d7)
    : rsPin(rs), rwPin(0), enablePin(enable), dataPins{d4, d5, d6, d7}, hasRw(false) {}

LiquidCrystal::LiquidCrystal(uint8_t rs, uint8_t rw, uint8_t enable, uint8_t d4, uint8_t d5,
                             uint8_t d6, uint8_t d7)
    : rsPin(rs), rwPin(rw), enablePin(enable), dataPins{d4, d5, d6, d7}, hasRw(true) {}

void LiquidCrystal::begin(uint8_t, uint8_t) {
  pinMode(rsPin, OUTPUT);
  pinMode(enablePin, OUTPUT);
  if (hasRw) pinMode(rwPin, OUTPUT);
  for (uint8_t i = 0; i < 4; i++) pinMode(dataPins[i], OUTPUT);
  digitalWrite(enablePin, LOW);
  delay(50);
  // The usual HD44780 wake-up sequence, then 4-bit, two-line, 5x8.
  send(0x33, false);
  send(0x32, false);
  send(0x28, false);
  send(0x0c, false);
  send(0x06, false);
  send(0x01, false);
  delay(2);
}

void LiquidCrystal::clear() {
  command(0x01);
  delay(2);
}

void LiquidCrystal::home() {
  command(0x02);
  delay(2);
}

void LiquidCrystal::setCursor(uint8_t col, uint8_t row) {
  command(0x80 | (row == 0 ? col : 0x40 + col));
}

size_t LiquidCrystal::write(uint8_t value) {
  send(value, true);
  return 1;
}

void LiquidCrystal::command(uint8_t value) { send(value, false); }

void LiquidCrystal::cursor() { command(0x0e); }
void LiquidCrystal::noCursor() { command(0x0c); }
void LiquidCrystal::blink() { command(0x0d); }
void LiquidCrystal::noBlink() { command(0x0c); }
void LiquidCrystal::display() { command(0x0c); }
void LiquidCrystal::noDisplay() { command(0x08); }
void LiquidCrystal::scrollDisplayLeft() { command(0x18); }
void LiquidCrystal::scrollDisplayRight() { command(0x1c); }
void LiquidCrystal::leftToRight() { command(0x06); }
void LiquidCrystal::rightToLeft() { command(0x04); }
void LiquidCrystal::autoscroll() { command(0x07); }
void LiquidCrystal::noAutoscroll() { command(0x06); }
void LiquidCrystal::createChar(uint8_t, uint8_t[]) {}

void LiquidCrystal::send(uint8_t value, bool data) {
  digitalWrite(rsPin, data ? HIGH : LOW);
  if (hasRw) digitalWrite(rwPin, LOW);
  write4bits(value >> 4);
  pulseEnable();
  write4bits(value & 0x0f);
  pulseEnable();
  delayMicroseconds(50);
}

void LiquidCrystal::write4bits(uint8_t value) {
  for (uint8_t i = 0; i < 4; i++) digitalWrite(dataPins[i], (value >> i) & 1);
}

void LiquidCrystal::pulseEnable() {
  digitalWrite(enablePin, LOW);
  delayMicroseconds(1);
  digitalWrite(enablePin, HIGH);
  delayMicroseconds(1);
  digitalWrite(enablePin, LOW);
  delayMicroseconds(50);
}
