#include "Stepper.h"

// Two coils energised at a time: A-/B+, A+/B+, A+/B-, A-/B-.
static const byte SEQUENCE[4][4] = {
    {1, 0, 1, 0},
    {0, 1, 1, 0},
    {0, 1, 0, 1},
    {1, 0, 0, 1},
};

Stepper::Stepper(int number_of_steps, int pin1, int pin2, int pin3, int pin4)
    : stepsPerRevolution(number_of_steps),
      pins{pin1, pin2, pin3, pin4},
      stepDelay(1000),
      stepIndex(0) {
  for (int i = 0; i < 4; i++) {
    pinMode(pins[i], OUTPUT);
    digitalWrite(pins[i], LOW);
  }
}

void Stepper::setSpeed(long rpm) {
  if (rpm <= 0 || stepsPerRevolution <= 0) return;
  unsigned long delay = 60000000UL / (unsigned long)stepsPerRevolution / (unsigned long)rpm;
  stepDelay = delay < 100 ? 100 : delay;
}

void Stepper::step(int steps) {
  int direction = steps >= 0 ? 1 : -1;
  if (steps < 0) steps = -steps;
  for (int i = 0; i < steps; i++) {
    stepIndex = (stepIndex + direction + 4) % 4;
    for (int p = 0; p < 4; p++) {
      digitalWrite(pins[p], SEQUENCE[stepIndex][p] ? HIGH : LOW);
    }
    delayMicroseconds(stepDelay);
  }
}
