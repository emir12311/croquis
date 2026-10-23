#ifndef CROQUIS_STEPPER_H
#define CROQUIS_STEPPER_H

#include <Arduino.h>

class Stepper {
 public:
  Stepper(int number_of_steps, int pin1, int pin2, int pin3, int pin4);

  void setSpeed(long rpm);
  void step(int steps);

 private:
  int stepsPerRevolution;
  int pins[4];
  unsigned long stepDelay;
  int stepIndex;
};

#endif
