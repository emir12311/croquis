#ifndef CROQUIS_DHT_H
#define CROQUIS_DHT_H

#include <Arduino.h>

#define DHT11 11
#define DHT22 22

class DHT {
 public:
  DHT(uint8_t pin, uint8_t type, uint8_t count = 6);

  void begin();
  float readTemperature(bool fahrenheit = false);
  float readHumidity();
  bool read(bool force = false);
  float computeHeatIndex(float temperature, float humidity, bool fahrenheit = false);

 private:
  uint8_t pin;
  uint8_t type;
  bool ok;
  float humidity;
  float temperature;
  unsigned long lastRead;
};

#endif
