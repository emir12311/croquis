#include "DHT.h"
#include <math.h>

// Measure the current level's pulse without skipping one that has already
// started: pulseIn() waits for a *new* pulse, which loses the high edge that
// follows the response low. The Adafruit library times the level directly.
static unsigned long measurePulse(uint8_t pin, uint8_t level, unsigned long timeout) {
  unsigned long start = micros();
  while (digitalRead(pin) != level) {
    if (micros() - start > timeout) return 0;
  }
  unsigned long begin = micros();
  while (digitalRead(pin) == level) {
    if (micros() - begin > timeout) return 0;
  }
  return micros() - begin;
}

DHT::DHT(uint8_t pin, uint8_t type, uint8_t)
    : pin(pin), type(type), ok(false), humidity(NAN), temperature(NAN), lastRead(0) {}

void DHT::begin() {
  pinMode(pin, INPUT_PULLUP);
  lastRead = 0;
}

bool DHT::read(bool force) {
  unsigned long now = millis();
  if (!force && lastRead != 0 && now - lastRead < 2000) return ok;
  lastRead = now;

  pinMode(pin, OUTPUT);
  digitalWrite(pin, LOW);
  delay(2);
  digitalWrite(pin, HIGH);
  delayMicroseconds(30);
  pinMode(pin, INPUT_PULLUP);

  // The simulator's UI round trip can be slow; wait generously for the answer.
  unsigned long low = measurePulse(pin, LOW, 100000);
  unsigned long high = measurePulse(pin, HIGH, 100000);
  if (low == 0 || high == 0) {
    ok = false;
    return false;
  }

  uint8_t data[5] = {0, 0, 0, 0, 0};
  for (int i = 0; i < 40; i++) {
    unsigned long lowBit = measurePulse(pin, LOW, 10000);
    unsigned long highBit = measurePulse(pin, HIGH, 10000);
    if (lowBit == 0 || highBit == 0) {
      ok = false;
      return false;
    }
    data[i / 8] <<= 1;
    if (highBit > 70) data[i / 8] |= 1;
  }
  if ((uint8_t)(data[0] + data[1] + data[2] + data[3]) != data[4]) {
    ok = false;
    return false;
  }

  uint16_t rh = ((uint16_t)data[0] << 8) | data[1];
  uint16_t rawTemperature = ((uint16_t)data[2] << 8) | data[3];
  bool negative = (rawTemperature & 0x8000) != 0;
  if (negative) rawTemperature &= 0x7fff;
  if (type == DHT11) {
    humidity = (float)data[0];
    temperature = (float)data[2];
  } else {
    humidity = rh / 10.0;
    temperature = (negative ? -1.0 : 1.0) * rawTemperature / 10.0;
  }
  ok = true;
  return true;
}

float DHT::readTemperature(bool fahrenheit) {
  read();
  if (fahrenheit) return temperature * 9.0 / 5.0 + 32.0;
  return temperature;
}

float DHT::readHumidity() {
  read();
  return humidity;
}

float DHT::computeHeatIndex(float t, float h, bool fahrenheit) {
  float temp = fahrenheit ? (t - 32.0) * 5.0 / 9.0 : t;
  float index = 0.5 * (temp + 61.0 + ((temp - 68.0) * 1.2) + (h * 0.094));
  if (index < 80.0) return fahrenheit ? index * 9.0 / 5.0 + 32.0 : index;
  index = -42.379 + 2.04901523 * temp + 10.14333127 * h - 0.22475541 * temp * h -
          0.00683783 * temp * temp - 0.05481717 * h * h + 0.00122874 * temp * temp * h +
          0.00085282 * temp * h * h - 0.00000199 * temp * temp * h * h;
  return fahrenheit ? index * 9.0 / 5.0 + 32.0 : index;
}
