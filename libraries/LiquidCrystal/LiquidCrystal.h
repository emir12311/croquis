#ifndef CROQUIS_LIQUIDCRYSTAL_H
#define CROQUIS_LIQUIDCRYSTAL_H

#include <Arduino.h>
#include <Print.h>

class LiquidCrystal : public Print {
 public:
  LiquidCrystal(uint8_t rs, uint8_t enable, uint8_t d4, uint8_t d5, uint8_t d6, uint8_t d7);
  LiquidCrystal(uint8_t rs, uint8_t rw, uint8_t enable, uint8_t d4, uint8_t d5, uint8_t d6,
                uint8_t d7);

  void begin(uint8_t cols = 16, uint8_t rows = 2);
  void clear();
  void home();
  void setCursor(uint8_t col, uint8_t row);
  size_t write(uint8_t value) override;
  void command(uint8_t value);
  void cursor();
  void noCursor();
  void blink();
  void noBlink();
  void display();
  void noDisplay();
  void scrollDisplayLeft();
  void scrollDisplayRight();
  void leftToRight();
  void rightToLeft();
  void autoscroll();
  void noAutoscroll();
  void createChar(uint8_t location, uint8_t charmap[]);

 private:
  uint8_t rsPin;
  uint8_t rwPin;
  uint8_t enablePin;
  uint8_t dataPins[4];
  bool hasRw;

  void send(uint8_t value, bool data);
  void write4bits(uint8_t value);
  void pulseEnable();
};

#endif
